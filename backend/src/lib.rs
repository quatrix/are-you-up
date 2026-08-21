use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::connect_info::ConnectInfo;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{DateTime, FixedOffset, Local, SecondsFormat};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;
use tracing::{debug, error, warn};
use utoipa::{OpenApi, ToSchema};

pub mod intervals;

fn peer_ip(peer: Option<SocketAddr>) -> String {
    peer
        .map(|address| address.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod peer_logging_tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::peer_ip;

    #[test]
    fn peer_ip_formats_the_remote_host_without_its_port() {
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(100, 107, 46, 54)), 41302);
        assert_eq!(peer_ip(Some(peer)), "100.107.46.54");
        assert_eq!(peer_ip(None), "unknown");
    }
}

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
    // `id` is the rowid alias (auto-incrementing); the old composite PK
    // lives on as UNIQUE so the POST upsert stays idempotent.
    conn.execute(
        "CREATE TABLE IF NOT EXISTS events (
            id         INTEGER PRIMARY KEY,
            event_name TEXT NOT NULL,
            ts         TEXT NOT NULL,
            UNIQUE (event_name, ts)
        )",
        [],
    )
    .expect("create events table");
    // One-time in-place migration for databases created before events had an
    // id (2026-08-20 shape: PRIMARY KEY (event_name, ts), no id column). The
    // CREATE above is a no-op on such a table, so detect and rebuild; sqlite
    // cannot add a PK via ALTER TABLE. First real migration in this codebase
    // (ADR-0012) - the schema had shipped, "recreate the db" stopped being
    // an option.
    let has_id = conn
        .prepare("SELECT 1 FROM pragma_table_info('events') WHERE name = 'id'")
        .and_then(|mut stmt| stmt.exists([]))
        .expect("inspect events schema");
    if !has_id {
        conn.execute_batch(
            "BEGIN;
             ALTER TABLE events RENAME TO events_old;
             CREATE TABLE events (
                 id         INTEGER PRIMARY KEY,
                 event_name TEXT NOT NULL,
                 ts         TEXT NOT NULL,
                 UNIQUE (event_name, ts)
             );
             INSERT INTO events (event_name, ts)
                 SELECT event_name, ts FROM events_old;
             DROP TABLE events_old;
             COMMIT;",
        )
        .expect("migrate events table to the id schema");
        tracing::info!(path, "migrated events table to the id schema");
    }
    debug!(path, journal_mode = mode, "database open");
    conn
}

#[cfg(test)]
mod migration_tests {
    use super::open_db;

    /// A database created with the pre-id events shape (PK (event_name, ts),
    /// no id column) must be rebuilt in place: rows preserved, ids assigned,
    /// and the (event_name, ts) upsert idempotency still intact.
    #[test]
    fn open_db_migrates_pre_id_events_table() {
        let dir = std::env::temp_dir().join(format!("are-you-up-migration-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("old.db");
        let path = path.to_str().expect("temp path is valid UTF-8");
        {
            let conn = rusqlite::Connection::open(path).expect("open raw db for seeding");
            conn.execute_batch(
                "CREATE TABLE events (
                     event_name TEXT NOT NULL,
                     ts         TEXT NOT NULL,
                     PRIMARY KEY (event_name, ts)
                 );
                 INSERT INTO events (event_name, ts)
                     VALUES ('took pills', '2026-08-20T09:05:00+03:00');",
            )
            .expect("seed old-shape db");
        }

        let conn = open_db(path);
        let (id, name): (i64, String) = conn
            .query_row("SELECT id, event_name FROM events", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("migrated row is readable with an id");
        assert_eq!((id, name.as_str()), (1, "took pills"));

        conn.execute(
            "INSERT INTO events (event_name, ts)
                 VALUES ('took pills', '2026-08-20T09:05:00+03:00')
             ON CONFLICT (event_name, ts) DO NOTHING",
            [],
        )
        .expect("upsert against the migrated table");
        let count: i64 = conn
            .query_row("SELECT count(*) FROM events", [], |row| row.get(0))
            .expect("count rows");
        assert_eq!(count, 1, "upsert idempotency survives the migration");

        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
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
        .route("/v1/events", post(post_event).get(get_events))
        .route("/v1/events/{id}", delete(delete_event))
        .route("/openapi.json", get(openapi_json))
        .route("/docs", get(docs))
        .route("/docs/rapidoc-min.js", get(rapidoc_js))
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

/// The API reference. The OpenAPI document is generated from the very types
/// the handlers deserialize/serialize (utoipa derives plus the
/// `#[utoipa::path]` annotations on each handler), so it cannot drift from
/// the wire format without failing to compile. New endpoints must carry the
/// annotation and be registered in `ApiDoc`'s `paths(...)` list.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "are-you-up",
        description = "Personal activity tracker feeding whoop sleep-detection \
                       correction. All timestamps are RFC 3339 strings carrying the \
                       device's local UTC offset (ADR-0004); percent-encode '+' \
                       as %2B in query parameters."
    ),
    paths(post_samples, get_intervals, post_event, get_events, delete_event)
)]
struct ApiDoc;

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

/// /docs renders /openapi.json with RapiDoc, vendored into the binary - the
/// same no-CDN, copy-one-file deployment treatment as the timeline page.
async fn docs() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../static/docs.html"))
}

async fn rapidoc_js() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/javascript")],
        include_str!("../static/vendor/rapidoc-min.js"),
    )
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
    (status, Json(ApiError { error: message })).into_response()
}

/// Uniform body of every 4xx/5xx the handlers produce.
#[derive(Serialize, ToSchema)]
struct ApiError {
    /// Human-readable reason.
    error: String,
}

#[derive(Deserialize, ToSchema)]
struct SamplesRequest {
    /// Free-form device name; adding a device never requires a schema change.
    #[schema(example = "macbook")]
    source: String,
    samples: Vec<SampleIn>,
}

#[derive(Deserialize, ToSchema)]
struct SampleIn {
    /// RFC 3339 with the device's local UTC offset (ADR-0004).
    #[schema(example = "2026-07-10T23:41:03+03:00")]
    ts: String,
    /// Wall-clock seconds since last input on the device.
    #[schema(minimum = 0)]
    idle_s: i64,
}

#[derive(Serialize, ToSchema)]
struct SamplesAck {
    /// Count of stored samples. Clients must verify it equals the batch size
    /// before marking rows synced; a bare 200 is not an ack (contract).
    accepted: usize,
}

#[utoipa::path(
    post,
    path = "/v1/samples",
    request_body = SamplesRequest,
    responses(
        (status = 200, description = "Whole batch stored, all-or-nothing; \
         upsert on (source, ts) makes retries idempotent", body = SamplesAck),
        (status = 400, description = "Malformed payload; nothing stored", body = ApiError),
    )
)]
async fn post_samples(
    State(state): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    body: String,
) -> Response {
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
    debug!(
        source = %req.source,
        peer = %peer_ip(peer.map(|Extension(ConnectInfo(address))| address)),
        accepted = req.samples.len(),
        "stored samples batch"
    );
    Json(SamplesAck {
        accepted: req.samples.len(),
    })
    .into_response()
}

/// Parses the from/to range shared by /v1/intervals and /v1/events: both
/// required, both RFC 3339. Err carries the ready-to-return 400 response.
fn parse_range(
    from: Option<&str>,
    to: Option<&str>,
) -> Result<(DateTime<FixedOffset>, DateTime<FixedOffset>), Response> {
    let (Some(from_raw), Some(to_raw)) = (from, to) else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "from and to are required (RFC 3339; percent-encode '+' as %2B)".into(),
        ));
    };
    let Ok(from) = DateTime::parse_from_rfc3339(from_raw) else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            format!("from is not RFC 3339 (percent-encode '+' as %2B): {from_raw:?}"),
        ));
    };
    let Ok(to) = DateTime::parse_from_rfc3339(to_raw) else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            format!("to is not RFC 3339 (percent-encode '+' as %2B): {to_raw:?}"),
        ));
    };
    Ok((from, to))
}

#[derive(Deserialize, ToSchema)]
struct EventRequest {
    /// Arbitrary non-empty label for what happened.
    #[schema(example = "took pills")]
    event_name: String,
    /// RFC 3339 with local UTC offset. Omit to have the server stamp its
    /// local now (ADR-0010).
    #[schema(example = "2026-08-20T09:12:00+03:00")]
    ts: Option<String>,
}

#[derive(Serialize, ToSchema)]
struct EventAck {
    accepted: usize,
    /// The stored timestamp: echoes the request `ts`, or the server-stamped
    /// now when the request omitted it.
    #[schema(example = "2026-08-20T09:12:00+03:00")]
    ts: String,
}

#[utoipa::path(
    post,
    path = "/v1/events",
    request_body = EventRequest,
    responses(
        (status = 200, description = "Event stored; upsert on (event_name, ts) \
         makes retries idempotent", body = EventAck),
        (status = 400, description = "Empty event_name or unparseable ts", body = ApiError),
    )
)]
async fn post_event(
    State(state): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    body: String,
) -> Response {
    // Hand-parsed for the same reason as post_samples: uniform JSON 400s.
    let req: EventRequest = match serde_json::from_str(&body) {
        Ok(req) => req,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, format!("invalid body: {e}")),
    };
    if req.event_name.trim().is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "event_name must be non-empty".into());
    }
    let ts = match req.ts {
        Some(ts) => {
            if DateTime::parse_from_rfc3339(&ts).is_err() {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    format!("ts is not RFC 3339: {ts:?}"),
                );
            }
            ts
        }
        // The one place the server originates a timestamp (ADR-0010): events
        // are typically logged as they happen from curl/shortcuts, where
        // typing an RFC 3339 instant is hostile. Still local offset,
        // per ADR-0004; the response echoes it so the caller knows what
        // was stored.
        None => Local::now().to_rfc3339_opts(SecondsFormat::Secs, false),
    };

    let conn = state
        .db
        .lock()
        .expect("db mutex is never poisoned: no handler panics while holding it");
    // Upsert on (event_name, ts) so retries after a lost response are harmless.
    if let Err(e) = conn.execute(
        "INSERT INTO events (event_name, ts) VALUES (?1, ?2)
         ON CONFLICT (event_name, ts) DO NOTHING",
        rusqlite::params![req.event_name, ts],
    ) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}"));
    }
    debug!(
        event_name = %req.event_name,
        ts = %ts,
        peer = %peer_ip(peer.map(|Extension(ConnectInfo(address))| address)),
        "stored event"
    );
    Json(EventAck { accepted: 1, ts }).into_response()
}

#[derive(Deserialize)]
struct EventsQuery {
    from: Option<String>,
    to: Option<String>,
}

#[derive(Serialize, ToSchema)]
struct EventOut {
    /// Auto-incremented; the handle for DELETE /v1/events/{id}.
    #[schema(example = 1)]
    id: i64,
    #[schema(example = "took pills")]
    event_name: String,
    #[schema(example = "2026-08-20T09:12:00+03:00")]
    ts: String,
}

#[derive(Serialize, ToSchema)]
struct DeleteAck {
    deleted: usize,
}

#[utoipa::path(
    delete,
    path = "/v1/events/{id}",
    params(("id" = i64, Path, description = "Event id, as returned by GET /v1/events")),
    responses(
        (status = 200, description = "Event deleted", body = DeleteAck),
        (status = 400, description = "Non-numeric id", body = ApiError),
        (status = 404, description = "No event with that id", body = ApiError),
    )
)]
async fn delete_event(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Response {
    // Result-parsed like the query extractors: a non-numeric id gets our
    // uniform JSON 400, not axum's plain-text rejection.
    let Path(id) = match id {
        Ok(id) => id,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, format!("invalid id: {e}")),
    };
    let conn = state
        .db
        .lock()
        .expect("db mutex is never poisoned: no handler panics while holding it");
    match conn.execute("DELETE FROM events WHERE id = ?1", [id]) {
        // Deleting a missing id is a 404, not a silent no-op: the UI acts on
        // ids it just listed, so a miss means someone else already deleted
        // it - worth surfacing rather than swallowing.
        Ok(0) => error_response(StatusCode::NOT_FOUND, format!("no event with id {id}")),
        Ok(_) => {
            debug!(id, "deleted event");
            Json(DeleteAck { deleted: 1 }).into_response()
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")),
    }
}

#[derive(Serialize, ToSchema)]
struct EventsResponse {
    /// Sorted by instant, event_name as tiebreak.
    events: Vec<EventOut>,
}

#[utoipa::path(
    get,
    path = "/v1/events",
    params(
        ("from" = String, Query, description = "RFC 3339, inclusive; \
          percent-encode '+' as %2B", example = "2026-08-20T00:00:00+03:00"),
        ("to" = String, Query, description = "RFC 3339, exclusive \
          (from <= ts < to)", example = "2026-08-21T00:00:00+03:00"),
    ),
    responses(
        (status = 200, description = "Events in range", body = EventsResponse),
        (status = 400, description = "Missing or unparseable from/to", body = ApiError),
    )
)]
async fn get_events(
    State(state): State<AppState>,
    query: Result<Query<EventsQuery>, QueryRejection>,
) -> Response {
    let Query(q) = match query {
        Ok(q) => q,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, format!("invalid query: {e}")),
    };
    let (from, to) = match parse_range(q.from.as_deref(), q.to.as_deref()) {
        Ok(range) => range,
        Err(response) => return response,
    };

    // Same full-scan-then-parse discipline as /v1/intervals: TEXT
    // range-filtering mixed-offset RFC 3339 is unsound (see the ts invariant
    // in CLAUDE.md), and this table is a handful of rows per day.
    let rows: Vec<(i64, String, String)> = {
        let conn = state
            .db
            .lock()
            .expect("db mutex is never poisoned: no handler panics while holding it");
        let mut stmt = match conn.prepare("SELECT id, event_name, ts FROM events") {
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

    let mut skipped_unparseable_ts = 0u32;
    let mut events: Vec<(DateTime<FixedOffset>, i64, String, String)> = rows
        .into_iter()
        .filter_map(|(id, event_name, ts)| {
            // Insert-time validation makes a bad row out-of-band edits, not a
            // client mistake: skip it loudly rather than 500 the request.
            let Ok(t) = DateTime::parse_from_rfc3339(&ts) else {
                skipped_unparseable_ts += 1;
                return None;
            };
            (t >= from && t < to).then_some((t, id, event_name, ts))
        })
        .collect();
    if skipped_unparseable_ts > 0 {
        warn!(
            skipped = skipped_unparseable_ts,
            "skipped rows with unparseable ts in /v1/events"
        );
    }
    // Instant first (TEXT order lies across offsets), name as a tiebreak so
    // simultaneous events come back in a deterministic order.
    events.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.cmp(&b.2)));

    let events: Vec<EventOut> = events
        .into_iter()
        .map(|(_, id, event_name, ts)| EventOut { id, event_name, ts })
        .collect();
    debug!(from = %from, to = %to, events = events.len(), "listed events");
    Json(EventsResponse { events }).into_response()
}

#[derive(Deserialize)]
struct IntervalsQuery {
    from: Option<String>,
    to: Option<String>,
    threshold_s: Option<i64>,
    source: Option<String>,
    consolidate: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
enum IntervalState {
    Active,
    Idle,
}

#[derive(Serialize, ToSchema)]
struct RawInterval {
    #[schema(example = "macbook")]
    source: String,
    #[schema(example = "2026-07-10T22:00:12+03:00")]
    start: String,
    #[schema(example = "2026-07-10T23:15:42+03:00")]
    end: String,
    state: IntervalState,
}

#[derive(Serialize, ToSchema)]
struct ConsolidatedInterval {
    #[schema(example = "2026-07-10T22:00:12+03:00")]
    start: String,
    #[schema(example = "2026-07-10T22:31:42+03:00")]
    end: String,
    /// Devices active during this piece; boundaries split wherever this
    /// set changes. Sorted.
    sources: Vec<String>,
}

/// Two shapes behind one endpoint: `consolidate=true` returns the
/// cross-source awake-evidence view (active time only, no state field),
/// otherwise per-source active/idle runs.
#[derive(Serialize, ToSchema)]
#[serde(untagged)]
enum IntervalsResponse {
    Raw { intervals: Vec<RawInterval> },
    Consolidated { intervals: Vec<ConsolidatedInterval> },
}

#[utoipa::path(
    get,
    path = "/v1/intervals",
    params(
        ("from" = String, Query, description = "RFC 3339, inclusive; \
          percent-encode '+' as %2B", example = "2026-07-10T00:00:00+03:00"),
        ("to" = String, Query, description = "RFC 3339, exclusive \
          (from <= ts < to)", example = "2026-07-11T00:00:00+03:00"),
        ("threshold_s" = Option<i64>, Query, description = "A sample is active \
          when idle_s < threshold_s; positive, default 900"),
        ("source" = Option<String>, Query, description = "Restrict to one \
          device; default all, derived per source"),
        ("consolidate" = Option<String>, Query, description = "Exactly \
          \"true\" or \"false\" (default). true returns the cross-source \
          awake-evidence shape"),
    ),
    responses(
        (status = 200, description = "Derived intervals; time not covered by \
         samples is absent, consumers treat it as no-signal", body = IntervalsResponse),
        (status = 400, description = "Missing/unparseable parameters", body = ApiError),
    )
)]
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
    let (from, to) = match parse_range(q.from.as_deref(), q.to.as_deref()) {
        Ok(range) => range,
        Err(response) => return response,
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

    let response = if consolidate {
        // The cross-source awake-evidence view: active time only, exact
        // source set per piece, no state field (see the spec's API section).
        IntervalsResponse::Consolidated {
            intervals: intervals::consolidate(&derived)
                .into_iter()
                .map(|iv| ConsolidatedInterval {
                    start: iv.start.to_rfc3339(),
                    end: iv.end.to_rfc3339(),
                    sources: iv.sources,
                })
                .collect(),
        }
    } else {
        IntervalsResponse::Raw {
            intervals: derived
                .iter()
                .flat_map(|(source, ivs)| {
                    ivs.iter().map(move |iv| RawInterval {
                        source: source.clone(),
                        start: iv.start.to_rfc3339(),
                        end: iv.end.to_rfc3339(),
                        state: match iv.state {
                            intervals::State::Active => IntervalState::Active,
                            intervals::State::Idle => IntervalState::Idle,
                        },
                    })
                })
                .collect(),
        }
    };
    let count = match &response {
        IntervalsResponse::Raw { intervals } => intervals.len(),
        IntervalsResponse::Consolidated { intervals } => intervals.len(),
    };
    debug!(
        from = %from,
        to = %to,
        threshold_s,
        source = q.source.as_deref().unwrap_or("<all>"),
        consolidate,
        intervals = count,
        "derived intervals"
    );
    Json(response).into_response()
}
