//! `POST /namespaces/write` and `POST /namespaces/read`.
//!
//! A write is one of:
//!
//! ```json
//! { "create": { "file": { "owner": "this" } } }
//! { "new_version": { "file": { "owner": "this", "viewer": "owner" } }, "based_on": { "revision": 12 } }
//! { "retire": "file", "based_on": { "revision": 14 } }
//! ```
//!
//! and gives the zookie of its revision. `based_on` is the zookie of the
//! version the write follows. A write that doesn't follow the latest
//! version is refused with the zookie of the latest version as `latest`,
//! `null` when there's none, so the caller can read it and try again.
//!
//! A read, `{ "namespace": "file" }` with an optional `zookie`, gives the
//! version's document, the zookie of the version as `version`, and the
//! zookie it read at.

use std::sync::Arc;

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sungma::{name::NamespaceName, revision::Revision, version::HistoryError};
use sungma_lang::namespace::{self, Refusal};
use sungma_read::namespaces::NamespaceReadService;
use sungma_write::{
    service::{NamespaceWriteService, NamespaceWriteServiceError},
    store::{InternError, NamespaceWrite, NamespaceWriteError},
};

use crate::{AppState, Edge, Ids, Refused, Stamps, Written, Zookie, accept, failure, read_failure};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WriteBody {
    create: Option<Box<RawValue>>,
    new_version: Option<Box<RawValue>>,
    retire: Option<String>,
    based_on: Option<Zookie>,
}

/// A write as the body names it, its document not yet parsed.
enum Write {
    Create(Box<RawValue>),
    NewVersion(Box<RawValue>, Revision),
    Retire(NamespaceName, Revision),
}

impl WriteBody {
    fn into_write(self) -> Result<Write, String> {
        match (self.create, self.new_version, self.retire, self.based_on) {
            (Some(document), None, None, None) => Ok(Write::Create(document)),
            (None, Some(document), None, Some(based_on)) => {
                Ok(Write::NewVersion(document, based_on.into()))
            }
            (None, None, Some(name), Some(based_on)) => Ok(Write::Retire(
                name.parse().map_err(|error| format!("{error}"))?,
                based_on.into(),
            )),
            _ => Err("expected create, new_version with based_on, or retire with based_on".into()),
        }
    }
}

#[derive(Serialize)]
struct Conflict {
    request_id: String,
    error: String,
    latest: Option<Zookie>,
}

pub(crate) async fn write<E: Edge, S: Ids, C: Stamps>(
    State(state): State<Arc<AppState<E, S, C>>>,
    headers: HeaderMap,
    body: Result<Json<WriteBody>, JsonRejection>,
) -> Response {
    let request_id = state.context(&headers).request_id;
    let write = match accept(body).and_then(|body| body.into_write().map_err(Refused::malformed)) {
        Ok(write) => write,
        Err(refused) => return refused.reply(&request_id),
    };
    let parsed = |document: Box<RawValue>| {
        namespace::parse(document.get()).map_err(|errors| {
            Refused(
                StatusCode::UNPROCESSABLE_ENTITY,
                Refusal(errors).to_string(),
            )
        })
    };
    let (name, write) = match write {
        Write::Retire(name, based_on) => (name, NamespaceWrite::Retire { based_on }),
        Write::Create(document) => match parsed(document) {
            Ok(document) => (
                document.namespace,
                NamespaceWrite::Create(document.relations),
            ),
            Err(refused) => return refused.reply(&request_id),
        },
        Write::NewVersion(document, based_on) => match parsed(document) {
            Ok(document) => (
                document.namespace,
                NamespaceWrite::NewVersion {
                    based_on,
                    namespace: document.relations,
                },
            ),
            Err(refused) => return refused.reply(&request_id),
        },
    };
    let service = NamespaceWriteService::new(state.node.interner(), state.node.writer());
    match service.write(&name, write).await {
        Ok(revision) => Json(Written {
            request_id,
            zookie: revision.into(),
        })
        .into_response(),
        Err(error) => match (status(&error), error) {
            // A conflict names the latest version, so the caller can read it.
            (
                status,
                error @ NamespaceWriteServiceError::Write(NamespaceWriteError::Conflict { latest }),
            ) => {
                let reply = Conflict {
                    request_id,
                    error: error.to_string(),
                    latest: latest.map(Zookie::from),
                };
                (status, Json(reply)).into_response()
            }
            (status, error) => failure(status, &request_id, error),
        },
    }
}

fn status(error: &NamespaceWriteServiceError) -> StatusCode {
    use NamespaceWriteServiceError::{Intern, Write};
    match error {
        Write(NamespaceWriteError::History(HistoryError::NothingToRetire)) => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        Write(NamespaceWriteError::Conflict { .. }) => StatusCode::CONFLICT,
        Intern(InternError::Exhausted(_))
        | Write(NamespaceWriteError::History(HistoryError::NotLater { .. })) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        Intern(InternError::Store(_))
        | Write(NamespaceWriteError::Clock(_) | NamespaceWriteError::Store(_)) => {
            StatusCode::BAD_GATEWAY
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadBody {
    #[serde(deserialize_with = "crate::named")]
    namespace: NamespaceName,
    zookie: Option<Zookie>,
}

#[derive(Serialize)]
struct ReadReply {
    request_id: String,
    /// The stored document as it is.
    document: Box<RawValue>,
    version: Zookie,
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
    let node = &state.node;
    let service = NamespaceReadService::new(node.names(), node.edge());
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
        Ok(document) => Json(ReadReply {
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
