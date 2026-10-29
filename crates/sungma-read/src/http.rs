//! The Read service's routes, `POST /facts/read` and
//! `POST /namespaces/read`. See `sungma-http` for what every route
//! shares.
//!
//! A fact read names a subjectset by the keys a fact uses, and optionally
//! a `subject`, for the one fact binding it, and a `zookie`:
//!
//! ```json
//! { "namespace": "file", "resource": "readme", "relation": "owner" }
//! ```
//!
//! It gives the facts that count, as a fact document, and the zookie it
//! read at.
//!
//! A namespace read, `{ "namespace": "file" }` with an optional `zookie`,
//! gives the version's document, the zookie of the version as `version`,
//! and the zookie it read at.
//!
//! Beyond the statuses every route shares, a zookie ahead of the latest
//! revision is 400, a namespace read that finds no version is 404, and a
//! store that failed is 502.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, value::RawValue};
use sungma::{
    name::{NamespaceName, RelationName, ResourceId, ResourceName, SubjectName, SubjectsetName},
    revision::Revision,
};
use sungma_http::{Requests, Zookie, accept, failure, named};
use sungma_lang::fact;
use sungma_names::store::Dictionary;
use sungma_table::read::ItemReader;

use crate::{ReadError, facts::FactReadService, namespaces::NamespaceReadService};

/// What every read reads through, shared across them: the names, and the
/// table.
pub struct AppState<D, R> {
    names: D,
    table: R,
    requests: Requests,
}

impl<D, R> AppState<D, R> {
    pub fn new(names: D, table: R) -> Self {
        Self {
            names,
            table,
            requests: Requests::default(),
        }
    }
}

pub fn router<D, R>(state: Arc<AppState<D, R>>) -> Router
where
    D: Dictionary + Send + Sync + 'static,
    R: ItemReader + Send + Sync + 'static,
{
    Router::new()
        .route("/facts/read", post(facts::<D, R>))
        .route("/namespaces/read", post(namespaces::<D, R>))
        .with_state(state)
}

/// A read refused: a zookie ahead of the latest revision is the caller's,
/// and a store failure is the store's.
fn read_failure(request_id: &str, error: ReadError) -> Response {
    let status = match error {
        ReadError::Ahead { .. } => StatusCode::BAD_REQUEST,
        ReadError::Store(_) => StatusCode::BAD_GATEWAY,
    };
    failure(status, request_id, error)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FactsBody {
    #[serde(deserialize_with = "named")]
    namespace: NamespaceName,
    #[serde(deserialize_with = "named")]
    resource: ResourceId,
    #[serde(deserialize_with = "named")]
    relation: RelationName,
    #[serde(default, deserialize_with = "subject")]
    subject: Option<SubjectName>,
    zookie: Option<Zookie>,
}

fn subject<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<SubjectName>, D::Error> {
    fact::subject(deserializer).map(Some)
}

#[derive(Serialize)]
struct FactsReply {
    request_id: String,
    facts: Value,
    zookie: Zookie,
}

async fn facts<D, R>(
    State(state): State<Arc<AppState<D, R>>>,
    headers: HeaderMap,
    body: Result<Json<FactsBody>, JsonRejection>,
) -> Response
where
    D: Dictionary + Sync,
    R: ItemReader + Sync,
{
    let request_id = state.requests.id(&headers);
    let body = match accept(body) {
        Ok(body) => body,
        Err(refused) => return refused.reply(&request_id),
    };
    let set = SubjectsetName::new(
        ResourceName::new(body.namespace, body.resource),
        body.relation,
    );
    let service = FactReadService::new(&state.names, &state.table);
    match service
        .read(&set, body.subject.as_ref(), body.zookie.map(Revision::from))
        .await
    {
        Ok(read) => Json(FactsReply {
            request_id,
            facts: fact::document(&read.facts),
            zookie: read.revision.into(),
        })
        .into_response(),
        Err(error) => read_failure(&request_id, error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamespacesBody {
    #[serde(deserialize_with = "named")]
    namespace: NamespaceName,
    zookie: Option<Zookie>,
}

#[derive(Serialize)]
struct NamespacesReply {
    request_id: String,
    /// The stored document as it is.
    document: Box<RawValue>,
    version: Zookie,
    zookie: Zookie,
}

async fn namespaces<D, R>(
    State(state): State<Arc<AppState<D, R>>>,
    headers: HeaderMap,
    body: Result<Json<NamespacesBody>, JsonRejection>,
) -> Response
where
    D: Dictionary + Sync,
    R: ItemReader + Sync,
{
    let request_id = state.requests.id(&headers);
    let body = match accept(body) {
        Ok(body) => body,
        Err(refused) => return refused.reply(&request_id),
    };
    let service = NamespaceReadService::new(&state.names, &state.table);
    let read = match service
        .read(&body.namespace, body.zookie.map(Revision::from))
        .await
    {
        Ok(read) => read,
        Err(error) => return read_failure(&request_id, error),
    };
    let Some(version) = read.version else {
        let error = format!(
            "namespace '{}' has no version at revision {}",
            body.namespace, read.revision.0
        );
        return failure(StatusCode::NOT_FOUND, &request_id, error);
    };
    // The store gives the text it stored; a store whose text isn't JSON
    // has failed.
    match RawValue::from_string(version.document) {
        Ok(document) => Json(NamespacesReply {
            request_id,
            document,
            version: version.revision.into(),
            zookie: read.revision.into(),
        })
        .into_response(),
        Err(error) => failure(
            StatusCode::BAD_GATEWAY,
            &request_id,
            format!("the stored document isn't JSON: {error}"),
        ),
    }
}
