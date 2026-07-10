use std::sync::{Arc, Mutex};

use axum::routing::get;
use axum::Router;
use rusqlite::Connection;

pub mod intervals;

/// Opens (creating if needed) the sqlite database and ensures the schema.
/// Panics on failure: without a database the server has no reason to run.
pub fn open_db(path: &str) -> Connection {
    let conn = Connection::open(path).expect("open sqlite database file");
    let _mode: String = conn
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .expect("set WAL journal mode");
    conn.execute(
        "CREATE TABLE IF NOT EXISTS samples (
            source TEXT NOT NULL,
            ts     TEXT NOT NULL,
            idle_s INTEGER NOT NULL,
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
        .with_state(state)
}
