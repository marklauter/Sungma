//! `POST /facts/write` and `POST /facts/read`, on each store.

mod common;

use axum::{Router, http::StatusCode};
use serde_json::{Value, json};

use common::{post, post_raw, stores};

/// `file` with an owner, a parent folder and viewers; `folder` with
/// viewers; `group` with members. Gives the zookie of the last version.
async fn declare(app: &Router) -> Value {
    let mut zookie = Value::Null;
    for document in [
        json!({ "file": { "owner": "this", "parent": "this", "viewer": "this | owner" } }),
        json!({ "folder": { "viewer": "this" } }),
        json!({ "group": { "member": "this" } }),
    ] {
        let (status, reply) = post(app, "/namespaces/write", json!({ "create": document })).await;
        assert_eq!(status, StatusCode::OK, "{reply}");
        zookie = reply["zookie"].clone();
    }
    zookie
}

fn owner(identity: &str) -> Value {
    json!({ "namespace": "file", "resource": "readme", "relation": "owner",
            "subject": { "identity": identity } })
}

async fn write(app: &Router, body: Value) -> (StatusCode, Value) {
    post(app, "/facts/write", body).await
}

async fn read(app: &Router, body: Value) -> (StatusCode, Value) {
    post(app, "/facts/read", body).await
}

fn owners() -> Value {
    json!({ "namespace": "file", "resource": "readme", "relation": "owner" })
}

#[tokio::test]
async fn facts_are_written_and_read_back() {
    for (store, app) in stores().await {
        let declared = declare(&app).await;
        let viewer = json!({ "namespace": "file", "resource": "readme", "relation": "viewer",
            "subject": { "namespace": "group", "resource": "eng", "relation": "member" } });
        let parent = json!({ "namespace": "file", "resource": "readme", "relation": "parent",
            "subject": { "namespace": "folder", "resource": "root" } });
        let (status, written) = write(
            &app,
            json!({ "add": [owner("bob"), owner("alice"), viewer, parent] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{store}: {written}");
        let zookie = written["zookie"].clone();
        assert!(zookie["revision"].as_u64() > declared["revision"].as_u64());

        let (status, reply) = read(&app, owners()).await;
        assert_eq!(status, StatusCode::OK, "{store}: {reply}");
        assert_eq!(
            reply["facts"],
            json!([owner("alice"), owner("bob")]),
            "{store}"
        );
        assert_eq!(reply["zookie"], zookie, "{store}");

        for (relation, fact) in [("viewer", viewer), ("parent", parent)] {
            let set = json!({ "namespace": "file", "resource": "readme", "relation": relation });
            let (_, reply) = read(&app, set).await;
            assert_eq!(reply["facts"], json!([fact]), "{store}: {relation}");
        }

        let mut point = owners();
        point["subject"] = json!({ "identity": "bob" });
        let (_, reply) = read(&app, point.clone()).await;
        assert_eq!(reply["facts"], json!([owner("bob")]), "{store}");
        point["subject"] = json!({ "identity": "carol" });
        let (_, reply) = read(&app, point).await;
        assert_eq!(reply["facts"], json!([]), "{store}");
    }
}

#[tokio::test]
async fn a_write_retires_and_adds_at_one_revision() {
    for (store, app) in stores().await {
        declare(&app).await;
        write(&app, json!({ "add": [owner("alice"), owner("bob")] })).await;
        let (status, moved) = write(
            &app,
            json!({ "retire": [owner("alice"), owner("bob")], "add": [owner("bob"), owner("carol")] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{store}: {moved}");
        let (_, reply) = read(&app, owners()).await;
        assert_eq!(
            reply["facts"],
            json!([owner("bob"), owner("carol")]),
            "{store}"
        );
        assert_eq!(reply["zookie"], moved["zookie"], "{store}");

        // Retiring a fact not held, and adding one held, change nothing.
        let (status, _) = write(
            &app,
            json!({ "retire": [owner("alice")], "add": [owner("bob")] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{store}");
        let (_, reply) = read(&app, owners()).await;
        assert_eq!(
            reply["facts"],
            json!([owner("bob"), owner("carol")]),
            "{store}"
        );
    }
}

#[tokio::test]
async fn a_write_naming_what_no_namespace_declares_is_refused_whole() {
    for (store, app) in stores().await {
        declare(&app).await;
        // Names never interned refuse an addition and a retirement.
        let unknown = [
            (
                json!({ "namespace": "user", "resource": "a", "relation": "owner",
                     "subject": { "identity": "alice" } }),
                "user",
            ),
            (
                json!({ "namespace": "file", "resource": "a", "relation": "editor",
                     "subject": { "identity": "alice" } }),
                "editor",
            ),
            (
                json!({ "namespace": "file", "resource": "a", "relation": "owner",
                     "subject": { "namespace": "file", "resource": "b", "relation": "editor" } }),
                "editor",
            ),
        ];
        for (fact, named) in unknown {
            for body in [
                json!({ "add": [owner("alice"), fact.clone()] }),
                json!({ "add": [owner("alice")], "retire": [fact.clone()] }),
            ] {
                let (status, reply) = write(&app, body.clone()).await;
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{store}: {body}");
                let error = reply["error"].as_str().unwrap();
                assert!(error.contains(named), "{store}: {error}");
            }
        }
        // `owner` is interned by `file`, but `folder` doesn't declare it.
        let undeclared = json!({ "namespace": "folder", "resource": "a", "relation": "owner",
                                 "subject": { "identity": "alice" } });
        let (status, reply) = write(&app, json!({ "add": [owner("alice"), undeclared] })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{store}: {reply}");
        let error = reply["error"].as_str().unwrap();
        assert!(error.contains("folder:a#owner@alice"), "{store}: {error}");

        let (_, reply) = read(&app, owners()).await;
        assert_eq!(reply["facts"], json!([]), "{store}");
    }
}

#[tokio::test]
async fn a_body_that_is_not_well_formed_is_refused() {
    for (store, app) in stores().await {
        declare(&app).await;
        for body in [
            json!({}),
            json!({ "add": [owner("alice")], "extra": [] }),
            json!({ "add": [{ "namespace": "file", "resource": "a", "relation": "owner" }] }),
            json!({ "add": [{ "namespace": "File!", "resource": "a", "relation": "owner",
                              "subject": { "identity": "alice" } }] }),
            json!({ "add": [{ "namespace": "file", "resource": "a b", "relation": "owner",
                              "subject": { "identity": "alice" } }] }),
            json!({ "add": "alice" }),
        ] {
            let (status, reply) = write(&app, body.clone()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{store}: {body}");
            assert!(reply["error"].is_string(), "{store}");
        }
        let path = "/facts/write";
        let (status, _) = post_raw(&app, path, "{", Some("application/json")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{store}");
        let (status, _) = post_raw(&app, path, "{}", None).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{store}");
    }
}

#[tokio::test]
async fn a_write_of_no_facts_is_refused_and_writes_no_revision() {
    for (store, app) in stores().await {
        let declared = declare(&app).await;
        for body in [
            json!({ "add": [] }),
            json!({ "retire": [] }),
            json!({ "add": [], "retire": [] }),
        ] {
            let (status, reply) = write(&app, body.clone()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{store}: {body}: {reply}");
        }
        let (_, reply) = read(&app, owners()).await;
        assert_eq!(reply["zookie"], declared, "{store}");
    }
}

/// A write past what one write holds is the caller's to split: 413, not a
/// store failure.
#[tokio::test]
async fn a_write_too_large_for_one_write_is_too_large() {
    for (store, app) in stores().await {
        declare(&app).await;
        let add: Vec<Value> = (0..60).map(|i| owner(&format!("p{i}"))).collect();
        let (status, reply) = write(&app, json!({ "add": add })).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{store}: {reply}");
        let (_, read) = read(&app, owners()).await;
        assert_eq!(read["facts"], json!([]), "{store}");
    }
}
