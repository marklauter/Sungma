//! The shared pieces, on a route that echoes a namespace name.

use std::sync::Arc;

use axum::{
    Json, Router,
    body::Body,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, HeaderValue, Request, StatusCode},
    response::{IntoResponse, Response},
    routing,
};
use http_body_util::BodyExt;
use serde::Deserialize;
use serde_json::{Value, json};
use sungma::{name::NamespaceName, revision::Revision};
use sungma_http::{Refused, Requests, Zookie, accept, header, named};
use tower::ServiceExt;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EchoBody {
    #[serde(deserialize_with = "named")]
    namespace: NamespaceName,
    zookie: Option<Zookie>,
}

/// Echoes the namespace and the zookie's revision; refuses the namespace
/// `refused` as malformed.
async fn echo(
    State(requests): State<Arc<Requests>>,
    headers: HeaderMap,
    body: Result<Json<EchoBody>, JsonRejection>,
) -> Response {
    let request_id = requests.id(&headers);
    let body = match accept(body) {
        Ok(body) => body,
        Err(refused) => return refused.reply(&request_id),
    };
    if body.namespace.to_string() == "refused" {
        return Refused::malformed("refused by the route").reply(&request_id);
    }
    let revision = body.zookie.map(|zookie| Revision::from(zookie).0);
    Json(
        json!({ "request_id": request_id, "namespace": body.namespace.to_string(),
                 "revision": revision }),
    )
    .into_response()
}

fn app() -> Router {
    Router::new()
        .route("/echo", routing::post(echo))
        .with_state(Arc::new(Requests::default()))
}

async fn post_raw(app: &Router, body: &str, headers: &[(&str, &str)]) -> (StatusCode, Value) {
    let mut request = Request::post("/echo");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = request.body(Body::from(body.to_owned())).unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let reply = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&bytes)));
    (status, reply)
}

const JSON: (&str, &str) = ("content-type", "application/json");

async fn post(app: &Router, body: Value) -> (StatusCode, Value) {
    post_raw(app, &body.to_string(), &[JSON]).await
}

#[tokio::test]
async fn a_body_is_accepted() {
    let (status, reply) = post(
        &app(),
        json!({ "namespace": "file", "zookie": { "revision": 7 } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        reply,
        json!({ "request_id": "sungma-1", "namespace": "file", "revision": 7 })
    );
}

#[tokio::test]
async fn requests_without_an_id_are_numbered() {
    let app = app();
    for n in 1..=3 {
        let (_, reply) = post(&app, json!({ "namespace": "file" })).await;
        assert_eq!(reply["request_id"], format!("sungma-{n}"));
    }
}

#[tokio::test]
async fn a_request_keeps_its_own_id() {
    let app = app();
    let body = json!({ "namespace": "file" }).to_string();
    let (_, reply) = post_raw(&app, &body, &[JSON, ("x-request-id", "r-42")]).await;
    assert_eq!(reply["request_id"], "r-42");
    // An id the request brings doesn't use up a number.
    let (_, reply) = post(&app, json!({ "namespace": "file" })).await;
    assert_eq!(reply["request_id"], "sungma-1");
}

#[test]
fn a_header_is_read_only_when_it_is_visible_ascii() {
    let mut headers = HeaderMap::new();
    headers.insert("x-caller", HeaderValue::from_static("docs-service"));
    headers.insert("x-other", HeaderValue::from_bytes(b"caf\xe9").unwrap());
    assert_eq!(
        header(&headers, "x-caller").as_deref(),
        Some("docs-service")
    );
    assert_eq!(header(&headers, "x-other"), None);
    assert_eq!(header(&headers, "x-missing"), None);
}

#[tokio::test]
async fn rejected_bodies_get_json_errors() {
    let app = app();
    let big = format!(
        r#"{{ "namespace": "file", "pad": "{}" }}"#,
        "x".repeat(3 * 1024 * 1024)
    );
    let cases = [
        ("{".to_owned(), vec![JSON], StatusCode::BAD_REQUEST),
        (
            r#"{"zookie":null}"#.to_owned(),
            vec![JSON],
            StatusCode::BAD_REQUEST,
        ),
        (
            r#"{"namespace":"file","extra":1}"#.to_owned(),
            vec![JSON],
            StatusCode::BAD_REQUEST,
        ),
        (
            r#"{"namespace":"file","zookie":{"revision":1,"extra":1}}"#.to_owned(),
            vec![JSON],
            StatusCode::BAD_REQUEST,
        ),
        (
            r#"{"namespace":"file"}"#.to_owned(),
            vec![],
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (big, vec![JSON], StatusCode::PAYLOAD_TOO_LARGE),
    ];
    for (body, headers, expected) in cases {
        let (status, reply) = post_raw(&app, &body, &headers).await;
        let shown = &body[..body.len().min(40)];
        assert_eq!(status, expected, "{shown}");
        assert!(
            reply["request_id"].as_str().unwrap().starts_with("sungma-"),
            "{shown}"
        );
        assert!(!reply["error"].as_str().unwrap().is_empty(), "{shown}");
    }
}

#[tokio::test]
async fn a_name_is_refused_as_the_grammar_refuses_it() {
    let (status, reply) = post(&app(), json!({ "namespace": "File!" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let expected = "File!".parse::<NamespaceName>().unwrap_err().to_string();
    let error = reply["error"].as_str().unwrap();
    assert!(error.contains(&expected), "{error}");
}

#[tokio::test]
async fn a_malformed_request_is_a_bad_request() {
    let (status, reply) = post(&app(), json!({ "namespace": "refused" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply,
        json!({ "request_id": "sungma-1", "error": "refused by the route" })
    );
}

#[test]
fn a_zookie_is_a_revision() {
    assert_eq!(Zookie::from(Revision(7)), Zookie { revision: 7 });
    assert_eq!(Revision::from(Zookie { revision: 7 }), Revision(7));
}
