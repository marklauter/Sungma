//! A store that fails is 502.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::json;
use sungma_read::http::{AppState, router};
use sungma_table::{
    TableError,
    item::Item,
    key::Key,
    read::{Consistency, ItemReader, Query},
};

use common::post;

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

#[tokio::test]
async fn a_store_that_fails_is_a_bad_gateway() {
    let app = router(Arc::new(AppState::new(sungma_node::names(&Down), Down)));
    for (path, body) in [
        (
            "/facts/read",
            json!({ "namespace": "file", "resource": "readme", "relation": "owner" }),
        ),
        ("/namespaces/read", json!({ "namespace": "file" })),
    ] {
        let (status, reply) = post(&app, path, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{path}: {body}");
        assert!(
            reply["error"].as_str().unwrap().contains("down"),
            "{path}: {reply}"
        );
    }
}
