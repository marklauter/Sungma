//! Posting to the routes, on each store.

// Each test file compiles this module separately and uses only part of it.
#![allow(dead_code)]

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::Value;
use sungma_api::{AppState, router};
use sungma_dynamodb::{DynamoReader, DynamoWriter, Table, local};
use tower::ServiceExt;

/// The API on an empty in-memory store, and on an empty table in
/// DynamoDB Local at `SUNGMA_DYNAMODB_URL`, `http://localhost:8000` by
/// default.
pub async fn stores() -> Vec<(&'static str, Router)> {
    vec![
        ("memory", router(Arc::new(AppState::memory()))),
        ("dynamodb", router(Arc::new(dynamodb().await))),
    ]
}

async fn dynamodb() -> AppState<DynamoReader, DynamoWriter<sungma_clock::DevClock>> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let url =
        std::env::var("SUNGMA_DYNAMODB_URL").unwrap_or_else(|_| "http://localhost:8000".to_owned());
    let name = format!(
        "sungma-api-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let table = Table::new(local(&url), name);
    table.create().await.unwrap_or_else(|error| {
        panic!("DynamoDB Local at {url} refused a table; run scripts/dynamodb.sh: {error}")
    });
    let reader = Arc::new(DynamoReader::new(table.clone()));
    let writer = Arc::new(DynamoWriter::dev(table).await.unwrap());
    AppState::new(reader, writer)
}

/// Posts `body` as JSON to `path`, and gives the status and the JSON reply.
pub async fn post(app: &Router, path: &str, body: Value) -> (StatusCode, Value) {
    post_raw(app, path, &body.to_string(), Some("application/json")).await
}

pub async fn post_raw(
    app: &Router,
    path: &str,
    body: &str,
    content_type: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = Request::post(path);
    if let Some(content_type) = content_type {
        request = request.header("content-type", content_type);
    }
    let request = request.body(Body::from(body.to_owned())).unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let reply = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&bytes)));
    (status, reply)
}
