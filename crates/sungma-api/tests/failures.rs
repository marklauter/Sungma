//! Failures outside the caller's control: a store or a clock that fails
//! is 502, and a check past Sungma's own limits is 500.

mod common;

use std::{ops::Range, sync::Arc};

use axum::{Router, http::StatusCode};
use serde_json::{Value, json};
use sungma::revision::Revision;
use sungma_api::{AppState, router};
use sungma_check::closure::MAX_DEPTH;
use sungma_clock::{ClockFault, DevClock, ManualClock, Reading};
use sungma_edge::{
    EdgeError,
    item::Item,
    key::Key,
    memory::{MemorySequence, MemoryTable},
    read::{Consistency, ItemReader, Query},
    sequence::Sequence,
    write::{CommitError, ItemWriter, Work},
};
use sungma_node::Node;

use common::post;

/// A store that is down.
#[derive(Clone, Copy)]
struct Down;

fn down() -> EdgeError {
    EdgeError("the store is down".to_owned())
}

impl ItemReader for Down {
    async fn get(&self, _: &Key, _: Consistency) -> Result<Option<Item>, EdgeError> {
        Err(down())
    }

    async fn query(&self, _: &Query, _: Consistency) -> Result<Vec<(Vec<u8>, Item)>, EdgeError> {
        Err(down())
    }
}

impl ItemWriter for Down {
    fn max_steps(&self) -> usize {
        100
    }

    async fn commit(&self, _: Work) -> Result<(), CommitError> {
        Err(down().into())
    }
}

impl Sequence for Down {
    async fn next(&self, _: &Key, _: u64) -> Result<Range<u64>, EdgeError> {
        Err(down())
    }
}

fn broken() -> AppState<Down, Down> {
    AppState::new(Node::new(Down, Down, DevClock::default()))
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
    let app = router(Arc::new(broken()));
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
    let state = broken();
    let namespace = r#"{ "file": { "owner": "this" } }"#;
    assert!(state.seed(&[namespace], "[]").await.is_err());
}

/// A writer whose clock can't give a revision it trusts.
fn faulted() -> Router {
    let clock = ManualClock::new(Reading {
        revision: Revision(1),
        settled: Revision(1),
    });
    clock.fault(ClockFault::Drifted);
    let edge = Arc::new(MemoryTable::new());
    let node = Node::new(edge, Arc::new(MemorySequence::new()), clock);
    router(Arc::new(AppState::new(node)))
}

#[tokio::test]
async fn a_clock_that_fails_is_a_bad_gateway() {
    let app = faulted();
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
    // A unit of work holds 100 rows and each new fact takes two, so the
    // chain goes in a few writes.
    let chain: Vec<Value> = (0..=MAX_DEPTH).map(|n| link(n, n + 1)).collect();
    for links in chain.chunks(40) {
        let (status, _) = post(&app, "/facts/write", json!({ "add": links })).await;
        assert_eq!(status, StatusCode::OK);
    }
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
    let sequence = Arc::new(MemorySequence::new());
    // The relation pool's counter, run out.
    let relations = Key::new((3u8, 1u8), 0u8);
    sequence.next(&relations, u64::MAX).await.unwrap();
    let node = Node::new(Arc::new(MemoryTable::new()), sequence, DevClock::default());
    let app = router(Arc::new(AppState::new(node)));
    let (status, reply) = post(
        &app,
        "/namespaces/write",
        json!({ "create": { "file": { "owner": "this" } } }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{reply}");
}
