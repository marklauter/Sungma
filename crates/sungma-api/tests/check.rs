//! `POST /check` against the docs theories and facts from the core's
//! fixtures.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sungma_api::{AppState, router};
use sungma_core::check::Verdict;
use tower::ServiceExt;

const THEORIES: &str = include_str!("../../sungma-core/tests/fixtures/docs.theories.json");
const FACTS: &str = include_str!("../../sungma-core/tests/fixtures/docs.facts.json");

fn state() -> Arc<AppState> {
    Arc::new(AppState::load(THEORIES, FACTS).unwrap())
}

async fn post(app: Router, body: Value, headers: &[(&str, &str)]) -> (StatusCode, Value) {
    let mut request = Request::post("/check").header("content-type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = request.body(Body::from(body.to_string())).unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn check(state: &Arc<AppState>, body: Value) -> (StatusCode, Value) {
    post(router(state.clone()), body, &[]).await
}

/// Every fixture fact is a new write, so the head is the fact count.
fn head() -> u64 {
    let facts: Vec<Value> = serde_json::from_str(FACTS).unwrap();
    facts.len() as u64
}

#[tokio::test]
async fn allowed_returns_the_zookie_it_was_decided_at() {
    let state = state();
    let head = head();
    let (status, reply) = check(
        &state,
        json!({ "set": "file:design.md#viewer", "identity": "alice" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        reply,
        json!({ "request_id": "sungma-1", "verdict": "allowed", "zookie": { "revision": head } })
    );
}

#[tokio::test]
async fn denied_and_unknown() {
    let state = state();
    let (_, reply) = check(
        &state,
        json!({ "set": "file:design.md#viewer", "identity": "bob" }),
    )
    .await;
    assert_eq!(reply["verdict"], "denied");
    assert!(reply["zookie"].is_object());

    let (status, reply) = check(
        &state,
        json!({ "set": "file:design.md#viewer", "identity": "mallory" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        reply,
        json!({ "request_id": "sungma-2", "verdict": "unknown" })
    );
}

#[tokio::test]
async fn a_subjectset_can_be_the_subject() {
    let state = state();
    let (_, reply) = check(
        &state,
        json!({ "set": "folder:root#viewer", "subjectset": "group:eng#member" }),
    )
    .await;
    assert_eq!(reply["verdict"], "allowed");
}

#[tokio::test]
async fn a_zookie_at_or_behind_the_head_is_accepted() {
    let state = state();
    let head = head();
    let (_, reply) = check(
        &state,
        json!({ "set": "file:design.md#viewer", "identity": "alice", "zookie": { "revision": head } }),
    )
    .await;
    assert_eq!(reply["zookie"]["revision"], head);
}

#[tokio::test]
async fn a_zookie_ahead_of_the_head_fails_and_is_audited() {
    let state = state();
    let ahead = head() + 1;
    let (status, reply) = check(
        &state,
        json!({ "set": "file:design.md#viewer", "identity": "alice", "zookie": { "revision": ahead } }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(reply["error"].as_str().unwrap().contains("ahead"));
    let records = state.audit.records();
    assert!(matches!(records[0].verdict, Verdict::Failed(_)));
}

#[tokio::test]
async fn headers_fill_the_audit_context() {
    let state = state();
    let (_, reply) = post(
        router(state.clone()),
        json!({ "set": "file:design.md#viewer", "identity": "alice" }),
        &[("x-request-id", "r-42"), ("x-caller", "docs-service")],
    )
    .await;
    assert_eq!(reply["request_id"], "r-42");
    let records = state.audit.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].context.request_id, "r-42");
    assert_eq!(records[0].context.caller, "docs-service");
    assert_eq!(records[0].request.resource, "design.md");
}

#[tokio::test]
async fn malformed_names_are_rejected() {
    let state = state();
    for body in [
        json!({ "set": "file:design.md", "identity": "alice" }),
        json!({ "set": "design.md#viewer", "identity": "alice" }),
        json!({ "set": "file:design.md#viewer", "subjectset": "group:eng" }),
    ] {
        let (status, reply) = check(&state, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(reply["error"].as_str().unwrap().starts_with("malformed"));
    }
    assert!(state.audit.records().is_empty());
}

#[test]
fn load_reports_bad_json() {
    assert!(AppState::load("{", FACTS).is_err());
    assert!(AppState::load(THEORIES, "[{}]").is_err());
}

#[tokio::test]
async fn two_subjects_are_rejected() {
    let state = state();
    let (status, reply) = check(
        &state,
        json!({ "set": "folder:root#viewer", "identity": "alice", "subjectset": "group:eng#member" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(reply["request_id"], "sungma-1");
    assert!(reply["error"].is_string());
    assert!(state.audit.records().is_empty());
}

#[tokio::test]
async fn no_subject_is_rejected() {
    let state = state();
    let (status, reply) = check(&state, json!({ "set": "folder:root#viewer" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(reply["error"].is_string());
}

/// Sends a raw body, so the JSON extractor can reject it.
async fn post_raw(
    state: &Arc<AppState>,
    body: &str,
    content_type: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = Request::post("/check");
    if let Some(content_type) = content_type {
        request = request.header("content-type", content_type);
    }
    let request = request.body(Body::from(body.to_owned())).unwrap();
    let response = router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let reply = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&bytes)));
    (status, reply)
}

#[tokio::test]
async fn rejected_bodies_get_json_errors() {
    let state = state();
    let json = Some("application/json");
    let cases = [
        (
            r#"{"set":"file:design.md#viewer","identity":"alice"}"#,
            None,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        ("{", json, StatusCode::BAD_REQUEST),
        (
            r#"{"identity":"alice"}"#,
            json,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            r#"{"set":"file:design.md#viewer","identty":"alice"}"#,
            json,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ];
    for (body, content_type, expected) in cases {
        let (status, reply) = post_raw(&state, body, content_type).await;
        assert_eq!(status, expected, "{body}");
        assert!(reply["request_id"].as_str().unwrap().starts_with("sungma-"));
        assert!(reply["error"].is_string());
    }
}
