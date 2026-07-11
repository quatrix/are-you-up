use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::DateTime;
use rusqlite::Connection;
use serde::Deserialize;
use tower_http::trace::TraceLayer;
use tracing::{debug, error, warn};

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
    conn.busy_timeout(std::time::Duration::from_millis(5000))
        .expect("set busy_timeout");
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
    debug!(path, journal_mode = mode, "database open");
    conn
}

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
}

pub fn app(conn: Connection) -> Router {
    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
    };
    Router::new()
        .route("/", get(timeline))
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/samples", post(post_samples))
        .route("/v1/intervals", get(get_intervals))
        // Per-request logging (method, path, status, latency) at debug level;
        // enable with RUST_LOG=debug or RUST_LOG=tower_http=debug.
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// The timeline visualization: one self-contained page (vanilla JS/CSS, no
/// CDNs), embedded in the binary so deployment stays copy-one-file. It
/// fetches /v1/intervals?consolidate=true from the same origin, so no CORS
/// machinery exists or is needed; tailscale remains the only perimeter.
async fn timeline() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../static/timeline.html"))
}

/// Uniform JSON error body. Client mistakes are always 4xx, never 500.
///
/// Two rejections never reach this function and so don't get this shape:
/// axum's body extractors reject a non-UTF-8 body with a plain-text 400 and
/// an oversize body with a plain-text 413, before our handlers run.
fn error_response(status: StatusCode, message: String) -> Response {
    // Central log point for every non-2xx we produce: server faults are
    // always visible, client mistakes only under RUST_LOG=debug.
    if status.is_server_error() {
        error!(%status, message, "request failed");
    } else {
        debug!(%status, message, "request rejected");
    }
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
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("samples[{i}].ts is not RFC 3339: {:?}", sample.ts),
            );
        }
        if sample.idle_s < 0 {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("samples[{i}].idle_s is negative"),
            );
        }
    }

    let mut conn = state
        .db
        .lock()
        .expect("db mutex is never poisoned: no handler panics while holding it");
    // Every early return below drops `tx` without calling commit(); rusqlite's
    // Transaction defaults to DropBehavior::Rollback, so the whole batch is
    // rolled back rather than leaving earlier rows half-committed.
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
    debug!(source = %req.source, accepted = req.samples.len(), "stored samples batch");
    Json(serde_json::json!({ "accepted": req.samples.len() })).into_response()
}

#[derive(Deserialize)]
struct IntervalsQuery {
    from: Option<String>,
    to: Option<String>,
    threshold_s: Option<i64>,
    source: Option<String>,
    consolidate: Option<String>,
}

async fn get_intervals(
    State(state): State<AppState>,
    query: Result<Query<IntervalsQuery>, QueryRejection>,
) -> Response {
    // Parsed as a Result (not a bare Query<_> extractor arg) so a malformed
    // threshold_s gets our uniform JSON 400 instead of axum's plain-text one,
    // matching post_samples' hand-parse rationale.
    let Query(q) = match query {
        Ok(q) => q,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, format!("invalid query: {e}")),
    };
    let (Some(from_raw), Some(to_raw)) = (&q.from, &q.to) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "from and to are required (RFC 3339; percent-encode '+' as %2B)".into(),
        );
    };
    let Ok(from) = DateTime::parse_from_rfc3339(from_raw) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("from is not RFC 3339 (percent-encode '+' as %2B): {from_raw:?}"),
        );
    };
    let Ok(to) = DateTime::parse_from_rfc3339(to_raw) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("to is not RFC 3339 (percent-encode '+' as %2B): {to_raw:?}"),
        );
    };
    let threshold_s = q.threshold_s.unwrap_or(900);
    if threshold_s <= 0 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "threshold_s must be positive".into(),
        );
    }
    // Strict tri-state: absent, "true", or "false". Bool-ish leniency
    // ("1", "True") would let a typo silently fall back to the raw shape.
    let consolidate = match q.consolidate.as_deref() {
        None | Some("false") => false,
        Some("true") => true,
        Some(other) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("consolidate must be \"true\" or \"false\", got {other:?}"),
            );
        }
    };

    // ponytail: full scan + parse. Measured (Task 4 quality review, see
    // LAB_NOTES.md 2026-07-10) at 1M rows (one device-year): ~0.8s/request
    // warm, ~100-150MB transient RSS - memory on a small host bites before
    // latency would. Revisit with an epoch column + index if that changes.
    let rows: Vec<(String, String, i64)> = {
        let conn = state
            .db
            .lock()
            .expect("db mutex is never poisoned: no handler panics while holding it");
        let mut stmt = match conn.prepare("SELECT source, ts, idle_s FROM samples") {
            Ok(stmt) => stmt,
            Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")),
        };
        let result = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .and_then(|mapped| mapped.collect());
        match result {
            Ok(rows) => rows,
            Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")),
        }
    };

    let mut by_source: BTreeMap<String, Vec<intervals::Sample>> = BTreeMap::new();
    let mut skipped_unparseable_ts = 0u32;
    for (source, ts, idle_s) in rows {
        if q.source.as_deref().is_some_and(|wanted| wanted != source) {
            continue;
        }
        // Rows were validated at insert time; a bad one here means out-of-band
        // edits or schema drift, not a client mistake - skip it rather than
        // 500 the whole request, but this must never happen silently: a
        // dropped row is a dropped interval in the whoop-correction data the
        // user acts on.
        let Ok(t) = DateTime::parse_from_rfc3339(&ts) else {
            skipped_unparseable_ts += 1;
            continue;
        };
        if t < from || t >= to {
            continue;
        }
        by_source
            .entry(source)
            .or_default()
            .push(intervals::Sample { t, idle_s });
    }
    if skipped_unparseable_ts > 0 {
        warn!(
            skipped = skipped_unparseable_ts,
            "skipped rows with unparseable ts in /v1/intervals"
        );
    }

    let derived: Vec<(String, Vec<intervals::Interval>)> = by_source
        .into_iter()
        .map(|(source, mut samples)| {
            samples.sort_by_key(|s| s.t);
            let ivs = intervals::derive(&samples, threshold_s, intervals::MAX_GAP_S);
            (source, ivs)
        })
        .collect();

    let out: Vec<serde_json::Value> = if consolidate {
        // The cross-source awake-evidence view: active time only, exact
        // source set per piece, no state field (see the spec's API section).
        intervals::consolidate(&derived)
            .into_iter()
            .map(|iv| {
                serde_json::json!({
                    "start": iv.start.to_rfc3339(),
                    "end": iv.end.to_rfc3339(),
                    "sources": iv.sources,
                })
            })
            .collect()
    } else {
        derived
            .iter()
            .flat_map(|(source, ivs)| {
                ivs.iter().map(move |iv| {
                    serde_json::json!({
                        "source": source,
                        "start": iv.start.to_rfc3339(),
                        "end": iv.end.to_rfc3339(),
                        "state": match iv.state {
                            intervals::State::Active => "active",
                            intervals::State::Idle => "idle",
                        },
                    })
                })
            })
            .collect()
    };
    debug!(
        from = %from,
        to = %to,
        threshold_s,
        source = q.source.as_deref().unwrap_or("<all>"),
        consolidate,
        intervals = out.len(),
        "derived intervals"
    );
    Json(serde_json::json!({ "intervals": out })).into_response()
}
