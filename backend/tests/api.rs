use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
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

/// A db-level constraint violation on the second row of a batch must roll
/// back the whole batch, not leave the first row half-committed. Simulated
/// with a stricter schema than the app would ever create itself, since
/// handler-level validation alone can't produce this failure.
#[tokio::test]
async fn post_samples_rolls_back_whole_batch_on_db_error() {
    let conn = open_db(":memory:");
    conn.execute("DROP TABLE samples", []).expect("drop table");
    conn.execute(
        "CREATE TABLE samples (
            source TEXT NOT NULL, ts TEXT NOT NULL,
            idle_s INTEGER NOT NULL CHECK (idle_s >= 0 AND idle_s < 100),
            PRIMARY KEY (source, ts)
        )",
        [],
    )
    .expect("recreate table with a stricter check");
    let app = app(conn);

    let batch = json!({
        "source": "macbook",
        "samples": [
            {"ts": "2026-07-10T22:00:00+03:00", "idle_s": 1},
            {"ts": "2026-07-10T22:00:30+03:00", "idle_s": 200}
        ]
    });
    let (status, _) = send(&app, "POST", "/v1/samples", Some(batch)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    // Row 1 alone would satisfy every constraint; prove it did not persist.
    let (_, body) = send(&app, "GET", "/v1/intervals?from=2026-07-10T22:00:00%2B03:00&to=2026-07-10T22:01:00%2B03:00", None).await;
    assert_eq!(body["intervals"], json!([]));
}

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
