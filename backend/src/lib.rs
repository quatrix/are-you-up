use std::sync::{Arc, Mutex};

use axum::routing::get;
use axum::Router;
use rusqlite::Connection;

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
