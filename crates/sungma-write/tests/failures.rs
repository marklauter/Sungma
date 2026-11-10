//! Failures outside the caller's control: a store or a clock that fails
//! is 502, and a write past Sungma's own limits is 500.

mod common;

use std::{ops::Range, sync::Arc};

use axum::{Router, http::StatusCode};
use serde_json::{Value, json};
use sungma::revision::Revision;
use sungma_clock::{Clock, ClockFault, DevClock, ManualClock, Reading};
use sungma_table::{
    TableError,
    item::Item,
    key::Key,
    memory::{MemorySequence, MemoryTable},
    read::{Consistency, ItemReader, Query},
    sequence::Sequence,
    write::{CommitError, ItemWriter, Work},
};
use sungma_write::http::{AppState, MAX_NAMESPACE_BODY_BYTES, router};

use common::{send, write_namespace};

/// A store that is down.
#[derive(Clone, Copy)]
struct Down;

fn down() -> TableError {
    TableError("the store is down".to_owned())
}

impl ItemReader for Down {
    async fn get(&self, _: &Key, _: Consistency) -> Result<Option<Item>, TableError> {
        Err(down())
    }

    async fn query(&self, _: &Query, _: Consistency) -> Result<Vec<(Vec<u8>, Item)>, TableError> {
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
    async fn next(&self, _: &Key, _: u64) -> Result<Range<u64>, TableError> {
        Err(down())
    }
}

/// The Write service's routes over the table, stamping writes from the
/// clock.
fn app<E, S, C>(table: E, sequence: S, clock: C) -> Router
where
    E: ItemReader + ItemWriter + Clone + Send + Sync + 'static,
    S: Sequence + Send + Sync + 'static,
    C: Clock + Send + Sync + 'static,
{
    let state = AppState::new(
        sungma_node::interner(&table, sequence),
        sungma_node::writer(&table, clock),
    );
    router(Arc::new(state))
}

fn memory() -> Router {
    app(
        Arc::new(MemoryTable::new()),
        Arc::new(MemorySequence::new()),
        DevClock::default(),
    )
}

fn readme(identity: &str) -> Value {
    json!({ "namespace": "file", "resource": "readme", "relation": "owner",
            "subject": { "identity": identity } })
}

/// A request to a route: its path, body and headers.
struct Sent {
    path: &'static str,
    body: String,
    headers: Vec<(&'static str, &'static str)>,
}

impl Sent {
    fn json(path: &'static str, body: Value) -> Self {
        let headers = vec![("content-type", "application/json")];
        let body = body.to_string();
        Self {
            path,
            body,
            headers,
        }
    }

    fn tdl(document: &str, headers: &[(&'static str, &'static str)]) -> Self {
        let mut all = vec![("content-type", "text/plain; charset=utf-8")];
        all.extend_from_slice(headers);
        Self {
            path: "/namespaces/write",
            body: document.to_owned(),
            headers: all,
        }
    }
}

/// Every route, with a request it would otherwise take.
fn requests() -> Vec<Sent> {
    vec![
        Sent::json("/facts/write", json!({ "add": [readme("alice")] })),
        Sent::json("/facts/write", json!({ "retire": [readme("alice")] })),
        Sent::tdl("namespace: file\n  owner<@identity>\n", &[]),
        Sent::tdl(
            "namespace: file\n  owner<@identity>\n",
            &[("if-match", "\"1\"")],
        ),
        Sent {
            path: "/namespaces/retire?namespace=file",
            body: String::new(),
            headers: vec![("if-match", "\"1\"")],
        },
    ]
}

#[tokio::test]
async fn a_store_that_fails_is_a_bad_gateway() {
    let app = app(Down, Down, DevClock::default());
    for sent in requests() {
        let (status, reply) = send(&app, sent.path, &sent.body, &sent.headers).await;
        let (path, body) = (sent.path, &sent.body);
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{path}: {body}");
        assert!(
            reply["error"].as_str().unwrap().contains("down"),
            "{path}: {reply}"
        );
    }
}

/// A writer whose clock can't give a revision it trusts.
fn faulted() -> Router {
    let clock = ManualClock::new(Reading {
        revision: Revision(1),
        settled: Revision(1),
    });
    clock.fault(ClockFault::Drifted);
    app(
        Arc::new(MemoryTable::new()),
        Arc::new(MemorySequence::new()),
        clock,
    )
}

#[tokio::test]
async fn a_clock_that_fails_is_a_bad_gateway() {
    let app = faulted();
    let (status, reply) =
        write_namespace(&app, "namespace: file\n  owner<@identity>\n", None).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{reply}");
    assert!(
        reply["error"].as_str().unwrap().contains("drifted"),
        "{reply}"
    );
}

/// A body past the size limit is too large, not malformed.
#[tokio::test]
async fn a_body_past_the_limit_is_too_large() {
    let app = memory();
    let big = format!(
        r#"{{ "add": [], "pad": "{}" }}"#,
        "x".repeat(3 * 1024 * 1024)
    );
    let (status, _) = common::post_raw(&app, "/facts/write", &big, Some("application/json")).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

/// A namespace write body up to the edge's limit reaches the parser, which
/// has none of its own; one byte more is too large.
#[tokio::test]
async fn a_namespace_body_holds_to_the_edges_limit() {
    let app = memory();
    let body = |bytes: usize| {
        let head = "namespace: file\n";
        let pad = " ".repeat(bytes - head.len());
        format!("{head}{pad}")
    };
    let at = body(MAX_NAMESPACE_BODY_BYTES);
    assert_eq!(at.len(), MAX_NAMESPACE_BODY_BYTES);
    let (status, reply) = write_namespace(&app, &at, None).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    let past = body(MAX_NAMESPACE_BODY_BYTES + 1);
    let (status, _) = write_namespace(&app, &past, None).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

/// Running out of ids is Sungma's own limit, not the caller's mistake.
#[tokio::test]
async fn a_pool_with_no_ids_left_is_a_fault() {
    let sequence = Arc::new(MemorySequence::new());
    // The relation pool's counter, run out.
    let relations = Key::new((3u8, 1u8), 0u8);
    sequence.next(&relations, u64::MAX).await.unwrap();
    let app = app(Arc::new(MemoryTable::new()), sequence, DevClock::default());
    let (status, reply) =
        write_namespace(&app, "namespace: file\n  owner<@identity>\n", None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{reply}");
}
