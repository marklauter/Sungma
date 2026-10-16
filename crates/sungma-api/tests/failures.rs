//! Failures outside the caller's control: a store or a clock that fails
//! is 502, and a check past Sungma's own limits is 500.

mod common;

use std::{ops::Range, sync::Arc};

use axum::{Router, http::StatusCode};
use serde_json::{Value, json};
use sungma::{
    closure::MAX_DEPTH,
    graph::{Subject, Subjectset},
    id::NamespaceId,
    revision::Revision,
    store::{DocumentReader, FactReader, NameReader, NamespaceReader, Pool, StoreError},
    version::Version,
};
use sungma_api::{AppState, router};
use sungma_clock::{ClockFault, ManualClock, Reading};
use sungma_dynamodb::{DynamoReader, DynamoWriter, Table, local};
use sungma_write::store::{
    FactWrite, FactWriteError, FactWriter, NameWriter, NamespaceWrite, NamespaceWriteError,
    NamespaceWriter,
};

use common::{dynamodb_url, post};

/// A store that is down.
struct Down;

fn down() -> StoreError {
    StoreError("the store is down".to_owned())
}

impl NameReader for Down {
    async fn id(&self, _: Pool, _: &str) -> Result<Option<u32>, StoreError> {
        Err(down())
    }

    async fn name(&self, _: Pool, _: u32) -> Result<Option<String>, StoreError> {
        Err(down())
    }
}

impl NameWriter for Down {
    async fn insert_if_absent(&self, _: Pool, _: &str, _: u32) -> Result<u32, StoreError> {
        Err(down())
    }

    async fn lease(&self, _: Pool, _: u32) -> Result<Range<u32>, StoreError> {
        Err(down())
    }
}

impl FactReader for Down {
    async fn head(&self) -> Result<Revision, StoreError> {
        Err(down())
    }

    async fn written(
        &self,
        _: &Subjectset,
        _: &Subject,
        _: Revision,
    ) -> Result<Option<Revision>, StoreError> {
        Err(down())
    }

    async fn subjectsets(
        &self,
        _: &Subjectset,
        _: Revision,
    ) -> Result<Vec<(Subjectset, Revision)>, StoreError> {
        Err(down())
    }

    async fn subjects(
        &self,
        _: &Subjectset,
        _: Revision,
    ) -> Result<Vec<(Subject, Revision)>, StoreError> {
        Err(down())
    }
}

impl FactWriter for Down {
    async fn write(&self, _: &[FactWrite]) -> Result<Revision, FactWriteError> {
        Err(down().into())
    }
}

impl NamespaceReader for Down {
    async fn version(
        &self,
        _: NamespaceId,
        _: Revision,
    ) -> Result<Option<Arc<Version>>, StoreError> {
        Err(down())
    }
}

impl DocumentReader for Down {
    async fn document(
        &self,
        _: NamespaceId,
        _: Revision,
    ) -> Result<Option<(Revision, String)>, StoreError> {
        Err(down())
    }
}

impl NamespaceWriter for Down {
    async fn write_namespace(
        &self,
        _: NamespaceId,
        _: NamespaceWrite,
    ) -> Result<Revision, NamespaceWriteError> {
        Err(down().into())
    }
}

fn readme(identity: &str) -> Value {
    json!({ "namespace": "file", "resource": "readme", "relation": "owner",
            "subject": { "identity": identity } })
}

/// Every route, with a body it would otherwise take.
fn requests() -> Vec<(&'static str, Value)> {
    let zookie = json!({ "revision": 1 });
    vec![
        (
            "/check",
            json!({ "set": "file:readme#owner", "identity": "alice" }),
        ),
        ("/facts/write", json!({ "add": [readme("alice")] })),
        ("/facts/write", json!({ "retire": [readme("alice")] })),
        (
            "/facts/read",
            json!({ "namespace": "file", "resource": "readme", "relation": "owner" }),
        ),
        (
            "/namespaces/write",
            json!({ "create": { "file": { "owner": "this" } } }),
        ),
        (
            "/namespaces/write",
            json!({ "new_version": { "file": { "owner": "this" } }, "based_on": zookie }),
        ),
        (
            "/namespaces/write",
            json!({ "retire": "file", "based_on": zookie }),
        ),
        ("/namespaces/read", json!({ "namespace": "file" })),
    ]
}

#[tokio::test]
async fn a_store_that_fails_is_a_bad_gateway() {
    let down = Arc::new(Down);
    let state = AppState::new(Arc::clone(&down), down);
    let app = router(Arc::new(state));
    for (path, body) in requests() {
        let (status, reply) = post(&app, path, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{path}: {body}");
        assert!(
            reply["error"].as_str().unwrap().contains("down"),
            "{path}: {reply}"
        );
    }
}

#[tokio::test]
async fn a_seed_into_a_store_that_fails_is_refused() {
    let down = Arc::new(Down);
    let state = AppState::new(Arc::clone(&down), down);
    let namespace = r#"{ "file": { "owner": "this" } }"#;
    assert!(state.seed(&[namespace], "[]").await.is_err());
}

/// A writer whose clock can't give a revision it trusts.
async fn faulted(url: &str) -> Router {
    let table = Table::new(
        local(url),
        format!("sungma-api-clock-{}", std::process::id()),
    );
    table.create().await.unwrap();
    let reader = Arc::new(DynamoReader::new(table.clone()));
    let clock = ManualClock::new(Reading {
        revision: Revision(1),
        settled: Revision(1),
    });
    clock.fault(ClockFault::Drifted);
    let writer = Arc::new(DynamoWriter::new(table, clock));
    router(Arc::new(AppState::new(reader, writer)))
}

#[tokio::test]
async fn a_clock_that_fails_is_a_bad_gateway() {
    let Some(url) = dynamodb_url() else { return };
    let app = faulted(&url).await;
    let (status, reply) = post(
        &app,
        "/namespaces/write",
        json!({ "create": { "file": { "owner": "this" } } }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{reply}");
    assert!(
        reply["error"].as_str().unwrap().contains("drifted"),
        "{reply}"
    );
}

/// A chain of groups longer than a check may walk.
#[tokio::test]
async fn a_check_past_the_depth_limit_is_a_fault() {
    let app = router(Arc::new(AppState::memory()));
    post(
        &app,
        "/namespaces/write",
        json!({ "create": { "group": { "member": "this" } } }),
    )
    .await;
    let link = |from: usize, to: usize| {
        json!({ "namespace": "group", "resource": format!("g{from}"), "relation": "member",
                "subject": { "namespace": "group", "resource": format!("g{to}"), "relation": "member" } })
    };
    let chain: Vec<Value> = (0..=MAX_DEPTH).map(|n| link(n, n + 1)).collect();
    let (status, _) = post(&app, "/facts/write", json!({ "add": chain })).await;
    assert_eq!(status, StatusCode::OK);
    let (status, reply) = post(
        &app,
        "/check",
        json!({ "set": "group:g0#member", "identity": "alice" }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{reply}");
}

/// A body past the size limit is too large, not malformed.
#[tokio::test]
async fn a_body_past_the_limit_is_too_large() {
    let app = router(Arc::new(AppState::memory()));
    let big = format!(
        r#"{{ "add": [], "pad": "{}" }}"#,
        "x".repeat(3 * 1024 * 1024)
    );
    let (status, _) = common::post_raw(&app, "/facts/write", &big, Some("application/json")).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

/// Running out of ids is Sungma's own limit, not the caller's mistake.
#[tokio::test]
async fn a_pool_with_no_ids_left_is_a_fault() {
    let memory = Arc::new(sungma_api::memory::Memory::default());
    memory.names.lease(Pool::Relations, u32::MAX).await.unwrap();
    let app = router(Arc::new(AppState::new(Arc::clone(&memory), memory)));
    let (status, reply) = post(
        &app,
        "/namespaces/write",
        json!({ "create": { "file": { "owner": "this" } } }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{reply}");
}
