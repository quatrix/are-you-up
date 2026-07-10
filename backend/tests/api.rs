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
