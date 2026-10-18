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
use sungma_dynamodb::{Table, items::DynamoTable, local};
use sungma_node::Node;
use tower::ServiceExt;

/// The API on an empty in-memory store and, with `SUNGMA_DYNAMODB_URL`
/// set, on an empty table in DynamoDB Local at that URL. An empty value
/// counts as unset, so CI can leave it out where DynamoDB Local can't run.
pub async fn stores() -> Vec<(&'static str, Router)> {
    let mut stores = vec![("memory", router(Arc::new(AppState::memory())))];
    if let Some(url) = dynamodb_url() {
        stores.push(("dynamodb", router(Arc::new(dynamodb(&url).await))));
    }
    stores
}

/// `SUNGMA_DYNAMODB_URL`, or none when it's unset or empty. The tests that
/// need DynamoDB Local pass without running when there's none.
pub fn dynamodb_url() -> Option<String> {
    std::env::var("SUNGMA_DYNAMODB_URL")
        .ok()
        .filter(|url| !url.is_empty())
}

async fn dynamodb(url: &str) -> AppState<DynamoTable, DynamoTable> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "sungma-api-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let table = Table::new(local(url), name);
    table.create().await.unwrap_or_else(|error| {
        panic!("DynamoDB Local at {url} refused a table; run scripts/dynamodb.sh: {error}")
    });
    let table = DynamoTable::new(table);
    AppState::new(Node::dev(table.clone(), table).await.unwrap())
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
