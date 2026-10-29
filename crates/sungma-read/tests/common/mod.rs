//! Posting to the routes, on each store. The Write service's routes share
//! each store, so a test can write what it reads.

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
use sungma_clock::DevClock;
use sungma_dynamodb::{Table, items::DynamoTable, local};
use sungma_read::http::{AppState, router};
use sungma_table::{
    memory::{MemorySequence, MemoryTable},
    read::ItemReader,
    sequence::Sequence,
    write::ItemWriter,
};
use tower::ServiceExt;

/// The Read service's routes over the table, and the Write service's
/// over the same table.
pub fn app<E, S>(table: E, sequence: S) -> Router
where
    E: ItemReader + ItemWriter + Clone + Send + Sync + 'static,
    S: Sequence + Send + Sync + 'static,
{
    let write = sungma_write::http::AppState::new(
        sungma_node::interner(&table, sequence),
        sungma_node::writer(&table, DevClock::default()),
    );
    let read = AppState::new(sungma_node::names(&table), table);
    router(Arc::new(read)).merge(sungma_write::http::router(Arc::new(write)))
}

/// The routes on an empty in-memory store and, with `SUNGMA_DYNAMODB_URL`
/// set, on an empty table in DynamoDB Local at that URL. An empty value
/// counts as unset, so CI can leave it out where DynamoDB Local can't run.
pub async fn stores() -> Vec<(&'static str, Router)> {
    let memory = app(
        Arc::new(MemoryTable::new()),
        Arc::new(MemorySequence::new()),
    );
    let mut stores = vec![("memory", memory)];
    if let Some(url) = dynamodb_url() {
        stores.push(("dynamodb", dynamodb(&url).await));
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

async fn dynamodb(url: &str) -> Router {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "{}-{}-{}",
        env!("CARGO_PKG_NAME"),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let table = Table::new(local(url), name);
    table.create().await.unwrap_or_else(|error| {
        panic!("DynamoDB Local at {url} refused a table; run scripts/dynamodb.sh: {error}")
    });
    let table = DynamoTable::new(table);
    app(table.clone(), table)
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
