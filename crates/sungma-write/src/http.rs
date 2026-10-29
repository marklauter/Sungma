//! The Write service's routes, `POST /facts/write` and
//! `POST /namespaces/write`. See `sungma-http` for what every route
//! shares.
//!
//! A fact write takes the write request in `docs/specs/fact-documents.md`:
//!
//! ```json
//! { "add": [ { "namespace": "file", "resource": "readme", "relation": "owner",
//!              "subject": { "identity": "alice" } } ],
//!   "retire": [] }
//! ```
//!
//! and gives the zookie of its revision.
//!
//! A namespace write is one of:
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
//! Beyond the statuses every route shares:
//!
//! - 409: a namespace write whose `based_on` isn't the latest version.
//! - 413: a fact write past what one write holds.
//! - 422: a refused write. A refused namespace document is 422 whatever
//!   its problems, names included, so it lists them all in one reply.
//! - 502: the store or the clock failed.
//! - 500: one of Sungma's limits, such as running out of ids.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sungma::{name::NamespaceName, revision::Revision};
use sungma_http::{Refused, Requests, Zookie, accept, failure};
use sungma_lang::{
    fact::WriteRequest,
    namespace::{self, Refusal},
};
use sungma_names::store::{InternError, Interner};

use crate::{
    service::{
        FactWriteService, FactWriteServiceError, NamespaceWriteService, NamespaceWriteServiceError,
    },
    store::{FactWriter, NamespaceWrite, NamespaceWriteError, NamespaceWriter},
};

/// What every write writes through, shared across them: the interner, and
/// the writer.
pub struct AppState<I, W> {
    interner: I,
    writer: W,
    requests: Requests,
}

impl<I, W> AppState<I, W> {
    pub fn new(interner: I, writer: W) -> Self {
        Self {
            interner,
            writer,
            requests: Requests::default(),
        }
    }
}

pub fn router<I, W>(state: Arc<AppState<I, W>>) -> Router
where
    I: Interner + Send + Sync + 'static,
    W: FactWriter + NamespaceWriter + Send + Sync + 'static,
{
    Router::new()
        .route("/facts/write", post(facts::<I, W>))
        .route("/namespaces/write", post(namespaces::<I, W>))
        .with_state(state)
}

/// A reply that gives only the zookie of a write.
#[derive(Serialize)]
struct Written {
    request_id: String,
    zookie: Zookie,
}

async fn facts<I, W>(
    State(state): State<Arc<AppState<I, W>>>,
    headers: HeaderMap,
    body: Result<Json<WriteRequest>, JsonRejection>,
) -> Response
where
    I: Interner + Sync,
    W: FactWriter + Sync,
{
    let request_id = state.requests.id(&headers);
    let request = match accept(body) {
        Ok(request) => request,
        Err(refused) => return refused.reply(&request_id),
    };
    let service = FactWriteService::new(&state.interner, &state.writer);
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
struct NamespacesBody {
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

impl NamespacesBody {
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

async fn namespaces<I, W>(
    State(state): State<Arc<AppState<I, W>>>,
    headers: HeaderMap,
    body: Result<Json<NamespacesBody>, JsonRejection>,
) -> Response
where
    I: Interner + Sync,
    W: NamespaceWriter + Sync,
{
    let request_id = state.requests.id(&headers);
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
    let service = NamespaceWriteService::new(&state.interner, &state.writer);
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
        Write(NamespaceWriteError::NothingToRetire) => StatusCode::UNPROCESSABLE_ENTITY,
        Write(NamespaceWriteError::Conflict { .. }) => StatusCode::CONFLICT,
        Intern(InternError::Exhausted(_)) => StatusCode::INTERNAL_SERVER_ERROR,
        Intern(InternError::Store(_))
        | Write(NamespaceWriteError::Clock(_) | NamespaceWriteError::Store(_)) => {
            StatusCode::BAD_GATEWAY
        }
    }
}
