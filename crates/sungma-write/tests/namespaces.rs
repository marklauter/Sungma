//! `POST /namespaces/write`, `POST /namespaces/retire` and
//! `POST /namespaces/read`, on each store.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};

use common::{post, retire_namespace, send, stores, write_namespace};

async fn read(app: &axum::Router, body: Value) -> (StatusCode, Value) {
    post(app, "/namespaces/read", body).await
}

#[tokio::test]
async fn a_namespace_is_created_read_versioned_and_retired() {
    for (store, app) in stores().await {
        let (status, created) = write_namespace(
            &app,
            "namespace: file\n  viewer: owner\n  owner<@identity>\n",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{store}: {created}");
        let v1 = created["zookie"].clone();

        let (status, reply) = read(&app, json!({ "namespace": "file" })).await;
        assert_eq!(status, StatusCode::OK, "{store}: {reply}");
        assert_eq!(
            reply["document"],
            json!("namespace: file\n  owner<@identity>\n  viewer: owner\n"),
            "{store}"
        );
        assert_eq!(reply["version"], v1, "{store}");
        assert_eq!(reply["zookie"], v1, "{store}");

        let (status, versioned) =
            write_namespace(&app, "namespace: file\n  owner<@identity>\n", Some(&v1)).await;
        assert_eq!(status, StatusCode::OK, "{store}: {versioned}");
        let v2 = versioned["zookie"].clone();
        assert!(v2["revision"].as_u64() > v1["revision"].as_u64(), "{store}");

        let (_, reply) = read(&app, json!({ "namespace": "file" })).await;
        assert_eq!(
            reply["document"],
            json!("namespace: file\n  owner<@identity>\n")
        );
        assert_eq!(reply["version"], v2, "{store}");

        // A read at an earlier zookie still reads at the latest revision.
        let (_, reply) = read(&app, json!({ "namespace": "file", "zookie": v1 })).await;
        assert_eq!(reply["version"], v2, "{store}");

        let (status, retired) = retire_namespace(&app, "file", &v2).await;
        assert_eq!(status, StatusCode::OK, "{store}: {retired}");

        let (status, reply) = read(&app, json!({ "namespace": "file" })).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{store}: {reply}");
        assert!(reply["error"].as_str().unwrap().contains("file"), "{store}");

        // Created again after its retirement.
        let (status, reply) = write_namespace(&app, "namespace: file\n", None).await;
        assert_eq!(status, StatusCode::OK, "{store}: {reply}");
    }
}

#[tokio::test]
async fn a_write_not_based_on_the_latest_version_is_a_conflict() {
    for (store, app) in stores().await {
        let (_, created) =
            write_namespace(&app, "namespace: file\n  owner<@identity>\n", None).await;
        let v1 = created["zookie"].clone();
        let (_, versioned) = write_namespace(
            &app,
            "namespace: file\n  owner<@identity>\n  viewer: owner\n",
            Some(&v1),
        )
        .await;
        let v2 = versioned["zookie"].clone();
        let stale = [
            write_namespace(&app, "namespace: file\n", Some(&v1)).await,
            retire_namespace(&app, "file", &v1).await,
            write_namespace(&app, "namespace: file\n", None).await,
        ];
        for (write, (status, reply)) in ["new version", "retire", "create"].iter().zip(stale) {
            assert_eq!(status, StatusCode::CONFLICT, "{store}: {write}");
            assert_eq!(reply["latest"], v2, "{store}: {write}");
            assert!(reply["error"].is_string(), "{store}");
        }
        // A namespace with no version has no latest to name.
        let (status, reply) = write_namespace(&app, "namespace: folder\n", Some(&v1)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{store}");
        assert_eq!(reply["latest"], Value::Null, "{store}");
    }
}

#[tokio::test]
async fn retiring_a_namespace_with_no_version_is_refused() {
    for (store, app) in stores().await {
        let based_on = json!({ "revision": 1 });
        let (status, _) = retire_namespace(&app, "file", &based_on).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{store}");

        let (_, created) = write_namespace(&app, "namespace: file\n", None).await;
        let v1 = created["zookie"].clone();
        let (_, retired) = retire_namespace(&app, "file", &v1).await;
        let (status, _) = retire_namespace(&app, "file", &retired["zookie"]).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{store}");
    }
}

#[tokio::test]
async fn a_refused_document_lists_every_problem() {
    for (store, app) in stores().await {
        let (status, reply) = write_namespace(
            &app,
            "namespace: file\n  viewer: editor\n  owner<@identity>\n  this<@identity>\n",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{store}: {reply}");
        let error = reply["error"].as_str().unwrap();
        assert!(error.contains("editor"), "{store}: {error}");
        assert!(error.contains("this"), "{store}: {error}");
        assert!(error.contains("; "), "{store}: {error}");

        let (status, _) = read(&app, json!({ "namespace": "file" })).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{store}");

        let (_, created) = write_namespace(&app, "namespace: file\n", None).await;
        let (status, reply) = write_namespace(
            &app,
            "namespace: file\n  viewer: editor\n",
            Some(&created["zookie"]),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{store}: {reply}");
    }
}

/// A namespace write is TDL sent as `text/plain`, with no parameter but
/// an optional `charset=utf-8`; any other content type is unsupported.
#[tokio::test]
async fn a_namespace_write_is_sent_as_utf_8_text() {
    for (store, app) in stores().await {
        let document = "namespace: file\n";
        for content_type in [
            None,
            Some("application/json"),
            Some("text/html"),
            Some("text/plain; charset=iso-8859-1"),
            Some("text/plain; format=flowed"),
            Some("text/plain; charset"),
            Some("text/plain; charset=\"utf-8"),
            Some("text/plain-ish"),
        ] {
            let headers: Vec<_> = content_type
                .map(|c| ("content-type", c))
                .into_iter()
                .collect();
            let (status, reply) = send(&app, "/namespaces/write", document, &headers).await;
            assert_eq!(
                status,
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "{store}: {content_type:?}"
            );
            assert!(
                reply["error"].as_str().unwrap().contains("text/plain"),
                "{store}"
            );
        }
        for (namespace, content_type) in [
            ("a", "text/plain"),
            ("b", "text/plain; charset=utf-8"),
            ("c", "Text/Plain ; Charset=\"UTF-8\""),
        ] {
            let document = format!("namespace: {namespace}\n");
            let headers = [("content-type", content_type)];
            let (status, reply) = send(&app, "/namespaces/write", &document, &headers).await;
            assert_eq!(status, StatusCode::OK, "{store}: {content_type}: {reply}");
        }
    }
}

#[tokio::test]
async fn a_body_that_is_not_utf_8_is_malformed() {
    for (store, app) in stores().await {
        let headers = [("content-type", "text/plain; charset=utf-8")];
        let body = b"namespace: \xff\n";
        let (status, reply) = send(&app, "/namespaces/write", body, &headers).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{store}: {reply}");
        assert!(reply["request_id"].is_string(), "{store}");
    }
}

/// `If-Match` is one revision as a strong entity tag.
#[tokio::test]
async fn an_if_match_that_is_not_one_revision_is_malformed() {
    for (store, app) in stores().await {
        let (_, created) = write_namespace(&app, "namespace: file\n", None).await;
        let revision = created["zookie"]["revision"].as_u64().unwrap();
        for tag in [
            format!("{revision}"),
            format!("W/\"{revision}\""),
            format!("\"{revision}\", \"{revision}\""),
            format!("\"+{revision}\""),
            format!("\"{revision}"),
            "*".to_owned(),
            "\"\"".to_owned(),
            "\"1x\"".to_owned(),
            "\"18446744073709551616\"".to_owned(),
        ] {
            let headers = [("content-type", "text/plain"), ("if-match", tag.as_str())];
            let (status, reply) =
                send(&app, "/namespaces/write", "namespace: file\n", &headers).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{store}: {tag}: {reply}");
            assert!(
                reply["error"].as_str().unwrap().contains("If-Match"),
                "{store}"
            );
            let path = "/namespaces/retire?namespace=file";
            let (status, _) = send(&app, path, "", &[("if-match", tag.as_str())]).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{store}: {tag}");
        }
        let tag = format!("\"{revision}\"");
        let headers = [
            ("content-type", "text/plain"),
            ("if-match", tag.as_str()),
            ("if-match", tag.as_str()),
        ];
        let (status, _) = send(&app, "/namespaces/write", "namespace: file\n", &headers).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{store}: two headers");
        // Space around the tag is allowed.
        let spaced = format!(" {tag} ");
        let headers = [
            ("content-type", "text/plain"),
            ("if-match", spaced.as_str()),
        ];
        let (status, reply) = send(&app, "/namespaces/write", "namespace: file\n", &headers).await;
        assert_eq!(status, StatusCode::OK, "{store}: {reply}");
    }
}

/// A namespace write declares exactly one namespace.
#[tokio::test]
async fn a_document_of_other_than_one_namespace_is_refused() {
    for (store, app) in stores().await {
        for document in ["", "namespace: file\nnamespace: folder\n"] {
            let (status, reply) = write_namespace(&app, document, None).await;
            assert_eq!(
                status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{store}: {document:?}: {reply}"
            );
        }
    }
}

/// A retire names its namespace in the query and the version it retires
/// in `If-Match`.
#[tokio::test]
async fn a_retire_names_its_namespace_and_version() {
    for (store, app) in stores().await {
        let (_, created) = write_namespace(&app, "namespace: file\n", None).await;
        let tag = common::if_match(&created["zookie"]);
        let headers = [("if-match", tag.as_str())];
        for path in [
            "/namespaces/retire",
            "/namespaces/retire?namespace=File!",
            "/namespaces/retire?namespace=file&extra=1",
            "/namespaces/retire?name=file",
        ] {
            let (status, reply) = send(&app, path, "", &headers).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{store}: {path}: {reply}");
            assert!(reply["request_id"].is_string(), "{store}");
        }
        let (status, reply) = send(&app, "/namespaces/retire?namespace=file", "", &[]).await;
        assert_eq!(
            status,
            StatusCode::PRECONDITION_REQUIRED,
            "{store}: {reply}"
        );
        assert!(
            reply["error"].as_str().unwrap().contains("If-Match"),
            "{store}"
        );

        let (status, reply) = read(&app, json!({ "namespace": "file" })).await;
        assert_eq!(status, StatusCode::OK, "{store}: still there: {reply}");
    }
}

/// A version naming a relation another namespace doesn't declare is
/// refused as unprocessable, naming the problem, and so is one naming the
/// relations of more namespaces than one write checks, as too large.
#[tokio::test]
async fn a_version_whose_references_fail_is_refused() {
    for (store, app) in stores().await {
        let (status, reply) =
            write_namespace(&app, "namespace: file\n  owner<group#member>\n", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{store}: {reply}");
        let error = reply["error"].as_str().unwrap();
        assert!(error.contains("'group#member'"), "{store}: {error}");

        let relation = |name: &str, namespaces: std::ops::Range<usize>| {
            let constraints: Vec<_> = namespaces.map(|i| format!("n{i}#member")).collect();
            format!("  {name}<{}>\n", constraints.join(", "))
        };
        let mut wide = "namespace: wide\n".to_owned();
        for r in 0..4 {
            wide.push_str(&relation(&format!("r{r}"), 0..32));
        }
        wide.push_str(&relation("s", 32..64));
        wide.push_str(&relation("t", 64..96));
        wide.push_str(&relation("u", 96..128));
        let (status, reply) = write_namespace(&app, &wide, None).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{store}: {reply}");
    }
}
