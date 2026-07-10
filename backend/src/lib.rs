use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::DateTime;
use rusqlite::Connection;
use serde::Deserialize;

pub mod intervals;

/// Opens (creating if needed) the sqlite database and ensures the schema.
/// Panics on failure: without a database the server has no reason to run.
pub fn open_db(path: &str) -> Connection {
    let conn = Connection::open(path).expect("open sqlite database file");
    // sqlite reports the journal mode it actually settled on, not necessarily the
    // one requested: in-memory databases (used by tests) always stay "memory"
    // regardless of what we ask for, and only "wal" is otherwise expected here.
    let mode: String = conn
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .expect("query journal_mode pragma");
    assert!(
        mode == "wal" || mode == "memory",
        "sqlite refused WAL journal mode (got {mode:?} for {path}); \
         check the database file is not on a network filesystem"
    );
    // Lets a concurrent writer (e.g. an ad-hoc `sqlite3` CLI inspecting the live
    // db) block briefly instead of us surfacing its SQLITE_BUSY as a 500.
    let _busy_timeout_ms: i64 = conn
        .query_row("PRAGMA busy_timeout=5000", [], |row| row.get(0))
        .expect("set busy_timeout pragma");
    conn.execute(
        "CREATE TABLE IF NOT EXISTS samples (
            source TEXT NOT NULL,
            ts     TEXT NOT NULL,
            idle_s INTEGER NOT NULL CHECK (idle_s >= 0),
            PRIMARY KEY (source, ts)
        )",
        [],
    )
    .expect("create samples table");
    conn
}

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
}

pub fn app(conn: Connection) -> Router {
    let state = AppState { db: Arc::new(Mutex::new(conn)) };
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/samples", post(post_samples))
        .with_state(state)
}

/// Uniform JSON error body. Client mistakes are always 4xx, never 500.
fn error_response(status: StatusCode, message: String) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

#[derive(Deserialize)]
struct SamplesRequest {
    source: String,
    samples: Vec<SampleIn>,
}

#[derive(Deserialize)]
struct SampleIn {
    ts: String,
    idle_s: i64,
}

async fn post_samples(State(state): State<AppState>, body: String) -> Response {
    // Parsed by hand (not the Json extractor) so that every kind of client
    // mistake gets a 400 with a reason; axum's extractor 422s some of them.
    let req: SamplesRequest = match serde_json::from_str(&body) {
        Ok(req) => req,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, format!("invalid body: {e}")),
    };
    if req.source.trim().is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "source must be non-empty".into());
    }
    for (i, sample) in req.samples.iter().enumerate() {
        if DateTime::parse_from_rfc3339(&sample.ts).is_err() {
            return error_response(StatusCode::BAD_REQUEST, format!("samples[{i}].ts is not RFC 3339: {:?}", sample.ts));
        }
        if sample.idle_s < 0 {
            return error_response(StatusCode::BAD_REQUEST, format!("samples[{i}].idle_s is negative"));
        }
    }

    let mut conn = state.db.lock().expect("db mutex is never poisoned: no handler panics while holding it");
    let tx = match conn.transaction() {
        Ok(tx) => tx,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")),
    };
    for sample in &req.samples {
        // Upsert on (source, ts) so client retries after a lost response are harmless.
        if let Err(e) = tx.execute(
            "INSERT INTO samples (source, ts, idle_s) VALUES (?1, ?2, ?3)
             ON CONFLICT (source, ts) DO UPDATE SET idle_s = excluded.idle_s",
            rusqlite::params![req.source, sample.ts, sample.idle_s],
        ) {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}"));
        }
    }
    if let Err(e) = tx.commit() {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}"));
    }
    Json(serde_json::json!({ "accepted": req.samples.len() })).into_response()
}
