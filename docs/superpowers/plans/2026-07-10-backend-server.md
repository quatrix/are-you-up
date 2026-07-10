# Backend Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** REST server that ingests raw `(source, ts, idle_s)` activity samples
and serves derived active/idle intervals over a range query.

**Architecture:** Single Rust crate in `backend/`. axum handlers over a
`Arc<Mutex<rusqlite::Connection>>` (one user, ~3 writes/min). Pure interval
derivation logic lives in its own module. Timestamps are RFC 3339 TEXT with
local offset, stored verbatim, compared as parsed instants (see spec
`docs/superpowers/specs/2026-07-10-are-you-up-design.md` and DECISIONS.md).

**Tech Stack:** Rust, axum, rusqlite (bundled), chrono, serde. Dev: tower
(oneshot), http-body-util.

**Conventions that apply to every commit in this plan:** semantic commit
titles, no co-author lines, prose wrapped, single-line commands.

## File structure

```
backend/
  Cargo.toml
  src/
    main.rs         thin binary: env config, bind, serve
    lib.rs          open_db, router, handlers, validation
    intervals.rs    pure derivation: samples -> intervals (all real logic)
  tests/
    api.rs          integration tests through the router
  scripts/
    smoke.sh        E2E: real server, synthetic samples, assert intervals
  README.md
```

`lib.rs` holds handlers + db because they are thin glue around SQL and
validation; `intervals.rs` is separate because it is the one piece of real
logic and must be unit-testable without HTTP or sqlite.

---

### Task 1: Crate scaffold + /healthz

**Files:**
- Create: `backend/Cargo.toml` (via cargo init/add)
- Create: `backend/src/lib.rs`
- Create: `backend/src/main.rs`
- Test: `backend/tests/api.rs`

- [ ] **Step 1: Initialize the crate and add dependencies**

Run (from repo root):

```bash
cd backend && cargo init --name are-you-up-backend
cargo add axum
cargo add tokio --features macros,rt-multi-thread,net
cargo add rusqlite --features bundled
cargo add chrono
cargo add serde --features derive
cargo add serde_json
cargo add --dev tower --features util
cargo add --dev http-body-util
```

Expected: `Cargo.toml` gains the dependencies. (`bundled` compiles sqlite
into the binary so the system sqlite version never matters.)

- [ ] **Step 2: Write the failing integration test**

Create `backend/tests/api.rs`:

```rust
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use are_you_up_backend::{app, open_db};

fn test_app() -> axum::Router {
    app(open_db(":memory:"))
}

/// Sends one request through the router and returns (status, parsed body).
/// Non-JSON bodies come back as a JSON string value.
async fn send(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let request = match body {
        Some(v) => Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .expect("request literals in tests are well-formed"),
        None => Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .expect("request literals in tests are well-formed"),
    };
    let response = app.clone().oneshot(request).await.expect("router is infallible");
    let status = response.status();
    let bytes = response.into_body().collect().await.expect("body reads to end").to_bytes();
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, value)
}

#[tokio::test]
async fn healthz_returns_ok() {
    let app = test_app();
    let (status, body) = send(&app, "GET", "/healthz", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, Value::String("ok".into()));
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cd backend && cargo test`
Expected: FAIL to compile with `unresolved import are_you_up_backend` (no lib yet).

- [ ] **Step 4: Write the minimal lib and binary**

Create `backend/src/lib.rs`:

```rust
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
```

Create `backend/src/intervals.rs` (empty module for now, filled in Task 2):

```rust
// Interval derivation lives here; implemented in Task 2.
```

Replace `backend/src/main.rs`:

```rust
use are_you_up_backend::{app, open_db};

#[tokio::main]
async fn main() {
    let addr = std::env::var("ARE_YOU_UP_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let db_path = std::env::var("ARE_YOU_UP_DB").unwrap_or_else(|_| "./are-you-up.db".into());
    let conn = open_db(&db_path);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind ARE_YOU_UP_ADDR; the address must be free and well-formed");
    println!("listening on {addr}, database at {db_path}");
    axum::serve(listener, app(conn)).await.expect("server runs until killed");
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd backend && cargo test`
Expected: PASS (`healthz_returns_ok`), zero failures.

- [ ] **Step 6: Commit**

```bash
git add backend
git commit -m "feat(backend): scaffold axum server with healthz and sqlite schema"
```

---

### Task 2: Interval derivation (the real logic)

**Files:**
- Modify: `backend/src/intervals.rs`

Derivation rules from the spec: a sample is *active* iff
`idle_s < threshold_s`. Consecutive same-state samples merge while the gap
between them is <= 90s; larger gaps break the interval. `start`/`end` are
the first/last sample timestamps of the run, never extrapolated. Samples
must arrive sorted ascending by time.

- [ ] **Step 1: Write the failing unit tests**

Replace `backend/src/intervals.rs`:

```rust
use chrono::{DateTime, FixedOffset};

/// Samples further apart than this cannot belong to the same interval: the
/// tracker was off or asleep in between, and that time must stay no-signal.
/// 3x the client sample period (30s).
pub const MAX_GAP_S: i64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Active,
    Idle,
}

#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub t: DateTime<FixedOffset>,
    pub idle_s: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Interval {
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub state: State,
}

/// Turns time-sorted samples into merged intervals. See module tests for
/// the exact semantics of the threshold and the gap break.
pub fn derive(samples: &[Sample], threshold_s: i64, max_gap_s: i64) -> Vec<Interval> {
    todo!("implemented in the next step")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(s).expect("test literals are valid RFC 3339")
    }

    fn s(ts: &str, idle_s: i64) -> Sample {
        Sample { t: t(ts), idle_s }
    }

    #[test]
    fn empty_input_gives_no_intervals() {
        assert_eq!(derive(&[], 900, MAX_GAP_S), vec![]);
    }

    #[test]
    fn single_sample_is_a_point_interval() {
        let got = derive(&[s("2026-07-10T22:00:00+03:00", 5)], 900, MAX_GAP_S);
        assert_eq!(
            got,
            vec![Interval { start: t("2026-07-10T22:00:00+03:00"), end: t("2026-07-10T22:00:00+03:00"), state: State::Active }]
        );
    }

    #[test]
    fn threshold_boundary_is_idle() {
        // idle_s == threshold means the last input was exactly threshold ago:
        // that is NOT "within the last threshold seconds", so it is idle.
        let got = derive(&[s("2026-07-10T22:00:00+03:00", 900)], 900, MAX_GAP_S);
        assert_eq!(got[0].state, State::Idle);
        let got = derive(&[s("2026-07-10T22:00:00+03:00", 899)], 900, MAX_GAP_S);
        assert_eq!(got[0].state, State::Active);
    }

    #[test]
    fn same_state_samples_within_gap_merge() {
        let got = derive(
            &[
                s("2026-07-10T22:00:00+03:00", 1),
                s("2026-07-10T22:00:30+03:00", 2),
                s("2026-07-10T22:01:00+03:00", 3),
            ],
            900,
            MAX_GAP_S,
        );
        assert_eq!(
            got,
            vec![Interval { start: t("2026-07-10T22:00:00+03:00"), end: t("2026-07-10T22:01:00+03:00"), state: State::Active }]
        );
    }

    #[test]
    fn state_change_splits_intervals() {
        let got = derive(
            &[
                s("2026-07-10T22:00:00+03:00", 1),
                s("2026-07-10T22:00:30+03:00", 1000),
                s("2026-07-10T22:01:00+03:00", 1030),
            ],
            900,
            MAX_GAP_S,
        );
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].state, State::Active);
        assert_eq!(got[0].end, t("2026-07-10T22:00:00+03:00"));
        assert_eq!(got[1].state, State::Idle);
        assert_eq!(got[1].start, t("2026-07-10T22:00:30+03:00"));
        assert_eq!(got[1].end, t("2026-07-10T22:01:00+03:00"));
    }

    #[test]
    fn gap_over_max_splits_even_with_same_state() {
        let got = derive(
            &[
                s("2026-07-10T22:00:00+03:00", 1),
                s("2026-07-10T22:00:30+03:00", 2),
                // 91s after the previous sample: one over the limit
                s("2026-07-10T22:02:01+03:00", 3),
            ],
            900,
            MAX_GAP_S,
        );
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].end, t("2026-07-10T22:00:30+03:00"));
        assert_eq!(got[1].start, t("2026-07-10T22:02:01+03:00"));
    }

    #[test]
    fn gap_of_exactly_max_still_merges() {
        let got = derive(
            &[s("2026-07-10T22:00:00+03:00", 1), s("2026-07-10T22:01:30+03:00", 2)],
            900,
            MAX_GAP_S,
        );
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn mixed_offsets_compare_as_instants() {
        // 20:00:00Z and 23:00:30+03:00 are 30 seconds apart in real time.
        let got = derive(
            &[s("2026-07-10T20:00:00Z", 1), s("2026-07-10T23:00:30+03:00", 2)],
            900,
            MAX_GAP_S,
        );
        assert_eq!(got.len(), 1);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd backend && cargo test intervals`
Expected: FAIL, every test panicking at `todo!` (or the empty-input test
failing to compile against `todo!` is also acceptable; the point is red).

- [ ] **Step 3: Implement derive**

Replace the `derive` function body in `backend/src/intervals.rs`:

```rust
pub fn derive(samples: &[Sample], threshold_s: i64, max_gap_s: i64) -> Vec<Interval> {
    let mut out: Vec<Interval> = Vec::new();
    for sample in samples {
        let state = if sample.idle_s < threshold_s { State::Active } else { State::Idle };
        match out.last_mut() {
            // Extend the current run: same state, and close enough in time.
            Some(last) if last.state == state && (sample.t - last.end).num_seconds() <= max_gap_s => {
                last.end = sample.t;
            }
            // State flip or gap break: start a new interval at this sample.
            _ => out.push(Interval { start: sample.t, end: sample.t, state }),
        }
    }
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd backend && cargo test intervals`
Expected: PASS, 8 tests, zero failures.

- [ ] **Step 5: Commit**

```bash
git add backend/src/intervals.rs
git commit -m "feat(backend): interval derivation with threshold and gap break"
```

---

### Task 3: POST /v1/samples

**Files:**
- Modify: `backend/src/lib.rs`
- Test: `backend/tests/api.rs`

- [ ] **Step 1: Write the failing integration tests**

Append to `backend/tests/api.rs`:

```rust
use serde_json::json;

fn batch() -> Value {
    json!({
        "source": "macbook",
        "samples": [
            {"ts": "2026-07-10T22:00:00+03:00", "idle_s": 4},
            {"ts": "2026-07-10T22:00:30+03:00", "idle_s": 34}
        ]
    })
}

#[tokio::test]
async fn post_samples_accepts_a_batch() {
    let app = test_app();
    let (status, body) = send(&app, "POST", "/v1/samples", Some(batch())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["accepted"], 2);
}

#[tokio::test]
async fn post_samples_is_idempotent() {
    let app = test_app();
    let (first, _) = send(&app, "POST", "/v1/samples", Some(batch())).await;
    let (second, body) = send(&app, "POST", "/v1/samples", Some(batch())).await;
    assert_eq!(first, StatusCode::OK);
    assert_eq!(second, StatusCode::OK);
    assert_eq!(body["accepted"], 2);
}

#[tokio::test]
async fn post_samples_rejects_bad_input_with_400_and_reason() {
    let app = test_app();
    let cases: Vec<Value> = vec![
        json!({"source": "", "samples": [{"ts": "2026-07-10T22:00:00+03:00", "idle_s": 1}]}),
        json!({"source": "macbook", "samples": [{"ts": "not a timestamp", "idle_s": 1}]}),
        json!({"source": "macbook", "samples": [{"ts": "2026-07-10T22:00:00+03:00", "idle_s": -1}]}),
        json!({"source": "macbook"}),
    ];
    for case in cases {
        let (status, body) = send(&app, "POST", "/v1/samples", Some(case.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "case: {case}");
        assert!(body["error"].is_string(), "case: {case}, body: {body}");
    }
    // Invalid JSON entirely.
    let request = Request::builder()
        .method("POST")
        .uri("/v1/samples")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{nope"))
        .expect("request literals in tests are well-formed");
    let response = app.clone().oneshot(request).await.expect("router is infallible");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd backend && cargo test --test api`
Expected: FAIL. The three new tests get 404 (route does not exist yet);
`healthz_returns_ok` still passes.

- [ ] **Step 3: Implement the handler**

In `backend/src/lib.rs`, replace the whole import block at the top of the
file with (do not keep the old `use` lines; duplicates will not compile):

```rust
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::DateTime;
use rusqlite::Connection;
use serde::Deserialize;
```

Replace the `app` function so it registers the new route:

```rust
pub fn app(conn: Connection) -> Router {
    let state = AppState { db: Arc::new(Mutex::new(conn)) };
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/samples", post(post_samples))
        .with_state(state)
}
```

Append the handler and its helper to `backend/src/lib.rs`:

```rust
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd backend && cargo test --test api`
Expected: PASS, 4 tests, zero failures.

- [ ] **Step 5: Commit**

```bash
git add backend/src/lib.rs backend/tests/api.rs
git commit -m "feat(backend): POST /v1/samples with validation and idempotent upsert"
```

---

### Task 4: GET /v1/intervals

**Files:**
- Modify: `backend/src/lib.rs`
- Test: `backend/tests/api.rs`

Note for consumers and tests: `+` in query strings decodes as a space, so
RFC 3339 offsets in `from`/`to` must be percent-encoded (`%2B03:00`).

- [ ] **Step 1: Write the failing integration tests**

Append to `backend/tests/api.rs`:

```rust
/// A synthetic evening for one source: active run, idle run, then a
/// >90s gap, then another active run.
async fn seed_evening(app: &axum::Router, source: &str) {
    let (status, _) = send(app, "POST", "/v1/samples", Some(json!({
        "source": source,
        "samples": [
            {"ts": "2026-07-10T22:00:00+03:00", "idle_s": 5},
            {"ts": "2026-07-10T22:00:30+03:00", "idle_s": 2},
            {"ts": "2026-07-10T22:01:00+03:00", "idle_s": 9},
            {"ts": "2026-07-10T22:01:30+03:00", "idle_s": 1000},
            {"ts": "2026-07-10T22:02:00+03:00", "idle_s": 1030},
            {"ts": "2026-07-10T22:10:00+03:00", "idle_s": 3},
            {"ts": "2026-07-10T22:10:30+03:00", "idle_s": 4}
        ]
    }))).await;
    assert_eq!(status, StatusCode::OK);
}

const RANGE: &str = "from=2026-07-10T22:00:00%2B03:00&to=2026-07-10T23:00:00%2B03:00";

#[tokio::test]
async fn intervals_derives_active_idle_and_gap_break() {
    let app = test_app();
    seed_evening(&app, "macbook").await;
    let (status, body) = send(&app, "GET", &format!("/v1/intervals?{RANGE}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["intervals"], json!([
        {"source": "macbook", "start": "2026-07-10T22:00:00+03:00", "end": "2026-07-10T22:01:00+03:00", "state": "active"},
        {"source": "macbook", "start": "2026-07-10T22:01:30+03:00", "end": "2026-07-10T22:02:00+03:00", "state": "idle"},
        {"source": "macbook", "start": "2026-07-10T22:10:00+03:00", "end": "2026-07-10T22:10:30+03:00", "state": "active"}
    ]));
}

#[tokio::test]
async fn intervals_threshold_is_a_query_param() {
    let app = test_app();
    seed_evening(&app, "macbook").await;
    // Threshold above every idle_s in the fixture: everything is active,
    // but the >90s gap still splits.
    let (status, body) = send(&app, "GET", &format!("/v1/intervals?{RANGE}&threshold_s=1031"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["intervals"], json!([
        {"source": "macbook", "start": "2026-07-10T22:00:00+03:00", "end": "2026-07-10T22:02:00+03:00", "state": "active"},
        {"source": "macbook", "start": "2026-07-10T22:10:00+03:00", "end": "2026-07-10T22:10:30+03:00", "state": "active"}
    ]));
}

#[tokio::test]
async fn intervals_separates_and_filters_sources() {
    let app = test_app();
    seed_evening(&app, "macbook").await;
    seed_evening(&app, "pixel").await;
    let (_, all) = send(&app, "GET", &format!("/v1/intervals?{RANGE}"), None).await;
    let intervals = all["intervals"].as_array().expect("intervals is an array");
    assert_eq!(intervals.len(), 6);
    assert!(intervals[..3].iter().all(|i| i["source"] == "macbook"));
    assert!(intervals[3..].iter().all(|i| i["source"] == "pixel"));

    let (_, only) = send(&app, "GET", &format!("/v1/intervals?{RANGE}&source=pixel"), None).await;
    let intervals = only["intervals"].as_array().expect("intervals is an array");
    assert_eq!(intervals.len(), 3);
    assert!(intervals.iter().all(|i| i["source"] == "pixel"));
}

#[tokio::test]
async fn intervals_range_is_half_open_and_empty_ranges_are_empty() {
    let app = test_app();
    seed_evening(&app, "macbook").await;
    // to == first sample ts: from <= ts < to excludes everything at 22:00:00.
    let (status, body) = send(&app, "GET", "/v1/intervals?from=2026-07-10T21:00:00%2B03:00&to=2026-07-10T22:00:00%2B03:00", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["intervals"], json!([]));
}

#[tokio::test]
async fn intervals_validates_params() {
    let app = test_app();
    let with_zero_threshold = format!("/v1/intervals?{RANGE}&threshold_s=0");
    let cases = [
        "/v1/intervals",
        "/v1/intervals?from=2026-07-10T22:00:00%2B03:00",
        "/v1/intervals?from=nope&to=2026-07-10T23:00:00%2B03:00",
        with_zero_threshold.as_str(),
    ];
    for uri in cases {
        let (status, _) = send(&app, "GET", uri, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "uri: {uri}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd backend && cargo test --test api`
Expected: FAIL. The five new tests get 404; earlier tests still pass.

- [ ] **Step 3: Implement the handler**

In `backend/src/lib.rs`, add to the imports (these are new; keep the rest):

```rust
use std::collections::BTreeMap;

use axum::extract::Query;
```

Register the route in `app` (add this line under the `/v1/samples` route):

```rust
        .route("/v1/intervals", get(get_intervals))
```

Append the handler:

```rust
#[derive(Deserialize)]
struct IntervalsQuery {
    from: Option<String>,
    to: Option<String>,
    threshold_s: Option<i64>,
    source: Option<String>,
}

async fn get_intervals(State(state): State<AppState>, Query(q): Query<IntervalsQuery>) -> Response {
    let (Some(from_raw), Some(to_raw)) = (&q.from, &q.to) else {
        return error_response(StatusCode::BAD_REQUEST, "from and to are required (RFC 3339; percent-encode '+')".into());
    };
    let Ok(from) = DateTime::parse_from_rfc3339(from_raw) else {
        return error_response(StatusCode::BAD_REQUEST, format!("from is not RFC 3339: {from_raw:?}"));
    };
    let Ok(to) = DateTime::parse_from_rfc3339(to_raw) else {
        return error_response(StatusCode::BAD_REQUEST, format!("to is not RFC 3339: {to_raw:?}"));
    };
    let threshold_s = q.threshold_s.unwrap_or(900);
    if threshold_s <= 0 {
        return error_response(StatusCode::BAD_REQUEST, "threshold_s must be positive".into());
    }

    // ponytail: full scan + parse, ~1M rows/year at one device. Add an epoch
    // column + index if a profile ever shows this query mattering.
    let rows: Vec<(String, String, i64)> = {
        let conn = state.db.lock().expect("db mutex is never poisoned: no handler panics while holding it");
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
    for (source, ts, idle_s) in rows {
        if let Some(wanted) = &q.source {
            if &source != wanted {
                continue;
            }
        }
        // Rows were validated at insert time; skip rather than 500 if one is somehow bad.
        let Ok(t) = DateTime::parse_from_rfc3339(&ts) else { continue };
        if t < from || t >= to {
            continue;
        }
        by_source.entry(source).or_default().push(intervals::Sample { t, idle_s });
    }

    let mut out = Vec::new();
    for (source, mut samples) in by_source {
        samples.sort_by_key(|s| s.t);
        for iv in intervals::derive(&samples, threshold_s, intervals::MAX_GAP_S) {
            out.push(serde_json::json!({
                "source": source,
                "start": iv.start.to_rfc3339(),
                "end": iv.end.to_rfc3339(),
                "state": match iv.state {
                    intervals::State::Active => "active",
                    intervals::State::Idle => "idle",
                },
            }));
        }
    }
    Json(serde_json::json!({ "intervals": out })).into_response()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd backend && cargo test`
Expected: PASS, all unit + integration tests, zero failures.

- [ ] **Step 5: Commit**

```bash
git add backend/src/lib.rs backend/tests/api.rs
git commit -m "feat(backend): GET /v1/intervals range query with per-source derivation"
```

---

### Task 5: Lint, format, and READMEs

**Files:**
- Create: `backend/README.md`
- Create: `README.md` (repo root)
- Possibly modify: any file cargo fmt/clippy touches

- [ ] **Step 1: Format and lint**

Run: `cd backend && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: clippy exits clean. Fix any warnings it reports (do not silence
with allow attributes unless the lint is genuinely wrong).

- [ ] **Step 2: Run the full test suite**

Run: `cd backend && cargo test`
Expected: PASS, zero failures.

- [ ] **Step 3: Write backend/README.md**

Create `backend/README.md`:

```markdown
# are-you-up backend

REST server that stores raw activity samples and serves derived
active/idle intervals. See `../docs/superpowers/specs/` for the design.

## Run

    ARE_YOU_UP_ADDR=127.0.0.1:8080 ARE_YOU_UP_DB=./are-you-up.db cargo run

Both env vars are optional; the values above are the defaults. Deploy by
binding the tailnet address.

## API

    POST /v1/samples
      {"source": "macbook", "samples": [{"ts": "2026-07-10T23:41:03+03:00", "idle_s": 4}]}
      -> {"accepted": 1}       (upsert on (source, ts); retries are harmless)

    GET /v1/intervals?from=...&to=...&threshold_s=900&source=macbook
      -> {"intervals": [{"source", "start", "end", "state": "active"|"idle"}]}
      from/to are RFC 3339 and required. Percent-encode "+" offsets (%2B).
      threshold_s (default 900): seconds without input before time counts
      as idle. Gaps in samples > 90s are returned as no interval at all.

    GET /healthz -> "ok"

## Test

    cargo test            # unit + integration
    scripts/smoke.sh      # E2E against a real server process
```

- [ ] **Step 4: Write the repo root README.md**

Create `README.md` at the repo root:

```markdown
# are-you-up

Tracks keyboard/mouse activity on my devices and serves it as
active/idle intervals, as a correction signal for whoop's time-in-bed
detection (sofa laptop sessions are not bed time).

- `mac/` - Swift menu-bar client: samples seconds-since-last-input every
  30s into local sqlite, syncs batches to the backend every 60s.
- `backend/` - Rust REST server: stores raw samples, derives intervals
  at query time (`threshold_s` is a query parameter, tunable forever).
- `android/` - later.

Design: `docs/superpowers/specs/2026-07-10-are-you-up-design.md`.
Decisions: `DECISIONS.md`. Findings: `LAB_NOTES.md`.

Timestamps are RFC 3339 with local offset everywhere. Communication is
plain HTTP over tailscale; there is no auth (single-user tailnet).
```

- [ ] **Step 5: Commit**

```bash
git add backend README.md
git commit -m "docs(backend): README for server and repo root; fmt and clippy pass"
```

---

### Task 6: E2E smoke script

**Files:**
- Create: `backend/scripts/smoke.sh`

- [ ] **Step 1: Write the script**

Create `backend/scripts/smoke.sh`:

```bash
#!/usr/bin/env bash
# E2E smoke: start the real server, POST a synthetic evening, assert the
# derived intervals byte-for-byte. Exits non-zero on any mismatch.
set -euo pipefail
cd "$(dirname "$0")/.."

PORT=$(( (RANDOM % 20000) + 20000 ))
DB="$(mktemp -d)/smoke.db"
BASE="http://127.0.0.1:$PORT"

cargo build --quiet
ARE_YOU_UP_ADDR="127.0.0.1:$PORT" ARE_YOU_UP_DB="$DB" ./target/debug/are-you-up-backend &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
    if curl -sf "$BASE/healthz" >/dev/null 2>&1; then break; fi
    sleep 0.1
done

curl -sf -X POST "$BASE/v1/samples" -H 'content-type: application/json' -d '{
  "source": "smoke",
  "samples": [
    {"ts": "2026-07-10T22:00:00+03:00", "idle_s": 5},
    {"ts": "2026-07-10T22:00:30+03:00", "idle_s": 2},
    {"ts": "2026-07-10T22:01:00+03:00", "idle_s": 9},
    {"ts": "2026-07-10T22:01:30+03:00", "idle_s": 1000},
    {"ts": "2026-07-10T22:02:00+03:00", "idle_s": 1030},
    {"ts": "2026-07-10T22:10:00+03:00", "idle_s": 3},
    {"ts": "2026-07-10T22:10:30+03:00", "idle_s": 4}
  ]}' >/dev/null

RESULT="$(curl -sf "$BASE/v1/intervals?from=2026-07-10T22:00:00%2B03:00&to=2026-07-10T23:00:00%2B03:00&source=smoke")"

python3 - "$RESULT" <<'EOF'
import json, sys
intervals = json.loads(sys.argv[1])["intervals"]
expected = [
    ("active", "2026-07-10T22:00:00+03:00", "2026-07-10T22:01:00+03:00"),
    ("idle",   "2026-07-10T22:01:30+03:00", "2026-07-10T22:02:00+03:00"),
    ("active", "2026-07-10T22:10:00+03:00", "2026-07-10T22:10:30+03:00"),
]
actual = [(i["state"], i["start"], i["end"]) for i in intervals]
assert actual == expected, f"unexpected intervals: {actual}"
print("smoke OK")
EOF
```

- [ ] **Step 2: Shellcheck it**

Run: `shellcheck backend/scripts/smoke.sh`
Expected: no output (clean). Fix anything it flags.

- [ ] **Step 3: Make it executable and run it**

Run: `chmod +x backend/scripts/smoke.sh && backend/scripts/smoke.sh`
Expected: prints `smoke OK`, exit code 0.

- [ ] **Step 4: Commit**

```bash
git add backend/scripts/smoke.sh
git commit -m "test(backend): E2E smoke script against a real server process"
```

---

## Plan self-review notes

- Spec coverage: schema/WAL (Task 1), derivation incl. boundary and gap
  semantics (Task 2), POST validation + upsert (Task 3), GET with
  half-open range, default 900, source filter (Task 4), env config
  (Task 1 main.rs), never-500-on-bad-input (Tasks 3-4 tests), E2E smoke
  (Task 6). Auth intentionally absent per spec.
- The `+`-in-query-string encoding pitfall is documented in Task 4, the
  README, and exercised by the smoke script.
