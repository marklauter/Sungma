//! `POST /namespaces/write` and `POST /namespaces/read`, on each store.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};

use common::{post, post_raw, stores};

async fn write(app: &axum::Router, body: Value) -> (StatusCode, Value) {
    post(app, "/namespaces/write", body).await
}

async fn read(app: &axum::Router, body: Value) -> (StatusCode, Value) {
    post(app, "/namespaces/read", body).await
}

#[tokio::test]
async fn a_namespace_is_created_read_versioned_and_retired() {
    for (store, app) in stores().await {
        let (status, created) = write(
            &app,
            json!({ "create": { "file": { "viewer": "owner", "owner": "this" } } }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{store}: {created}");
        let v1 = created["zookie"].clone();

        let (status, reply) = read(&app, json!({ "namespace": "file" })).await;
        assert_eq!(status, StatusCode::OK, "{store}: {reply}");
        assert_eq!(
            reply["document"],
            json!({ "file": { "owner": "this", "viewer": "owner" } }),
            "{store}"
        );
        assert_eq!(reply["version"], v1, "{store}");
        assert_eq!(reply["zookie"], v1, "{store}");

        let (status, versioned) = write(
            &app,
            json!({ "new_version": { "file": { "owner": "this" } }, "based_on": v1 }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{store}: {versioned}");
        let v2 = versioned["zookie"].clone();
        assert!(v2["revision"].as_u64() > v1["revision"].as_u64(), "{store}");

        let (_, reply) = read(&app, json!({ "namespace": "file" })).await;
        assert_eq!(reply["document"], json!({ "file": { "owner": "this" } }));
        assert_eq!(reply["version"], v2, "{store}");

        // A read at an earlier zookie still reads at the latest revision.
        let (_, reply) = read(&app, json!({ "namespace": "file", "zookie": v1 })).await;
        assert_eq!(reply["version"], v2, "{store}");

        let (status, retired) = write(&app, json!({ "retire": "file", "based_on": v2 })).await;
        assert_eq!(status, StatusCode::OK, "{store}: {retired}");

        let (status, reply) = read(&app, json!({ "namespace": "file" })).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{store}: {reply}");
        assert!(reply["error"].as_str().unwrap().contains("file"), "{store}");

        // Created again after its retirement.
        let (status, reply) = write(&app, json!({ "create": { "file": {} } })).await;
        assert_eq!(status, StatusCode::OK, "{store}: {reply}");
    }
}

#[tokio::test]
async fn a_write_not_based_on_the_latest_version_is_a_conflict() {
    for (store, app) in stores().await {
        let (_, created) = write(&app, json!({ "create": { "file": { "owner": "this" } } })).await;
        let v1 = created["zookie"].clone();
        let (_, versioned) = write(
            &app,
            json!({ "new_version": { "file": { "owner": "this", "viewer": "owner" } }, "based_on": v1 }),
        )
        .await;
        let v2 = versioned["zookie"].clone();
        for body in [
            json!({ "new_version": { "file": {} }, "based_on": v1 }),
            json!({ "retire": "file", "based_on": v1 }),
            json!({ "create": { "file": {} } }),
        ] {
            let (status, reply) = write(&app, body.clone()).await;
            assert_eq!(status, StatusCode::CONFLICT, "{store}: {body}");
            assert_eq!(reply["latest"], v2, "{store}: {body}");
            assert!(reply["error"].is_string(), "{store}");
        }
        // A namespace with no version has no latest to name.
        let (status, reply) = write(
            &app,
            json!({ "new_version": { "folder": {} }, "based_on": v1 }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{store}");
        assert_eq!(reply["latest"], Value::Null, "{store}");
    }
}

#[tokio::test]
async fn retiring_a_namespace_with_no_version_is_refused() {
    for (store, app) in stores().await {
        let based_on = json!({ "revision": 1 });
        let (status, _) = write(&app, json!({ "retire": "file", "based_on": based_on })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{store}");

        let (_, created) = write(&app, json!({ "create": { "file": {} } })).await;
        let v1 = created["zookie"].clone();
        let (_, retired) = write(&app, json!({ "retire": "file", "based_on": v1 })).await;
        let (status, _) = write(
            &app,
            json!({ "retire": "file", "based_on": retired["zookie"] }),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{store}");
    }
}

#[tokio::test]
async fn a_refused_document_lists_every_problem() {
    for (store, app) in stores().await {
        let (status, reply) = write(
            &app,
            json!({ "create": { "file": { "viewer": "editor", "owner": "this", "this": "this" } } }),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{store}: {reply}");
        let error = reply["error"].as_str().unwrap();
        assert!(error.contains("editor"), "{store}: {error}");
        assert!(error.contains("this"), "{store}: {error}");
        assert!(error.contains("; "), "{store}: {error}");

        let (status, _) = read(&app, json!({ "namespace": "file" })).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{store}");

        let (_, created) = write(&app, json!({ "create": { "file": {} } })).await;
        let (status, reply) = write(
            &app,
            json!({ "new_version": { "file": { "viewer": "editor" } }, "based_on": created["zookie"] }),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{store}: {reply}");
    }
}

#[tokio::test]
async fn a_body_of_no_known_shape_is_malformed() {
    for (store, app) in stores().await {
        let zookie = json!({ "revision": 1 });
        for body in [
            json!({}),
            json!({ "create": { "file": {} }, "based_on": zookie }),
            json!({ "new_version": { "file": {} } }),
            json!({ "retire": "file" }),
            json!({ "create": { "file": {} }, "retire": "file", "based_on": zookie }),
            json!({ "retire": "no such name!", "based_on": zookie }),
            json!({ "create": { "file": {} }, "extra": 1 }),
        ] {
            let (status, reply) = write(&app, body.clone()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{store}: {body}");
            assert!(reply["request_id"].is_string(), "{store}");
            assert!(reply["error"].is_string(), "{store}");
        }
        let body = r#"{"create":{"file":{}}}"#;
        let (status, _) = post_raw(&app, "/namespaces/write", body, None).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{store}");
    }
}
