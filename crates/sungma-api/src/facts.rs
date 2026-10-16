//! `POST /facts/write` and `POST /facts/read`.
//!
//! A write takes the write request in `docs/specs/fact-documents.md`:
//!
//! ```json
//! { "add": [ { "namespace": "file", "resource": "readme", "relation": "owner",
//!              "subject": { "identity": "alice" } } ],
//!   "retire": [] }
//! ```
//!
//! and gives the zookie of its revision. A read names a subjectset by the
//! keys a fact uses, and optionally a `subject`, for the one fact binding
//! it, and a `zookie`:
//!
//! ```json
//! { "namespace": "file", "resource": "readme", "relation": "owner" }
//! ```
//!
//! It gives the facts that count, as a fact document, and the zookie it
//! read at.

use std::sync::Arc;

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use sungma::{
    name::{NamespaceName, RelationName, ResourceId, ResourceName, SubjectName, SubjectsetName},
    revision::Revision,
};
use sungma_lang::fact::{self, WriteRequest};
use sungma_read::facts::FactReadService;
use sungma_write::service::{FactWriteService, FactWriteServiceError};

use crate::{AppState, Edge, Ids, Stamps, Written, Zookie, accept, failure, named, read_failure};

pub(crate) async fn write<E: Edge, S: Ids, C: Stamps>(
    State(state): State<Arc<AppState<E, S, C>>>,
    headers: HeaderMap,
    body: Result<Json<WriteRequest>, JsonRejection>,
) -> Response {
    let request_id = state.context(&headers).request_id;
    let request = match accept(body) {
        Ok(request) => request,
        Err(refused) => return refused.reply(&request_id),
    };
    let service = FactWriteService::new(state.node.interner(), state.node.writer());
    match service.write(&request.add, &request.retire).await {
        Ok(revision) => Json(Written {
            request_id,
            zookie: revision.into(),
        })
        .into_response(),
        Err(error) => {
            let status = match error {
                FactWriteServiceError::Unknown(_) | FactWriteServiceError::Undeclared(_) => {
                    StatusCode::UNPROCESSABLE_ENTITY
                }
                FactWriteServiceError::TooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
                FactWriteServiceError::Clock(_) | FactWriteServiceError::Store(_) => {
                    StatusCode::BAD_GATEWAY
                }
            };
            failure(status, &request_id, error)
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadBody {
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
struct ReadReply {
    request_id: String,
    facts: Value,
    zookie: Zookie,
}

pub(crate) async fn read<E: Edge, S: Ids, C: Stamps>(
    State(state): State<Arc<AppState<E, S, C>>>,
    headers: HeaderMap,
    body: Result<Json<ReadBody>, JsonRejection>,
) -> Response {
    let request_id = state.context(&headers).request_id;
    let body = match accept(body) {
        Ok(body) => body,
        Err(refused) => return refused.reply(&request_id),
    };
    let set = SubjectsetName::new(
        ResourceName::new(body.namespace, body.resource),
        body.relation,
    );
    let node = &state.node;
    let service = FactReadService::new(node.names(), node.edge());
    match service
        .read(&set, body.subject.as_ref(), body.zookie.map(Revision::from))
        .await
    {
        Ok(read) => Json(ReadReply {
            request_id,
            facts: fact::document(&read.facts),
            zookie: read.revision.into(),
        })
        .into_response(),
        Err(error) => read_failure(&request_id, error),
    }
}
