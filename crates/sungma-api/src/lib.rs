//! Sungma's web API: JSON over HTTP, on axum. Every route is a `POST`
//! taking and giving JSON, and every reply carries `request_id`.
//!
//! - `POST /check` judges one fact.
//! - `POST /facts/write` and `POST /facts/read` write and read facts.
//! - `POST /namespaces/write` and `POST /namespaces/read` write and read
//!   namespace versions.
//!
//! `POST /check` names the fact in the string form:
//!
//! ```json
//! { "set": "file:design.md#viewer", "identity": "alice", "zookie": { "revision": 12 } }
//! ```
//!
//! The subject is an `identity` or a `subjectset` (`namespace:id#relation`).
//! The zookie is optional, and is plain JSON for now so it stays readable.
//! The reply names the verdict and, once decided, the zookie of the
//! revision it was decided at:
//!
//! ```json
//! { "request_id": "sungma-1", "verdict": "allowed", "zookie": { "revision": 12 } }
//! ```
//!
//! The `x-request-id` and `x-caller` headers fill the audit record's
//! context. Neither is authenticated.
//!
//! Every error reply is JSON, `{"request_id": .., "error": ..}`, with one of
//! these statuses:
//!
//! - 400: a body that isn't well formed, a name that breaks the grammar, or
//!   a zookie ahead of the latest revision.
//! - 404: a namespace read that finds no version.
//! - 409: a namespace write whose `based_on` isn't the latest version.
//! - 413: a body past the size limit.
//! - 415: a body that isn't sent as JSON.
//! - 422: a refused write. A refused namespace document is 422 whatever
//!   its problems, names included, so it lists them all in one reply.
//! - 502: the store or the clock failed.
//! - 500: a fault in Sungma itself, or one of its limits, such as running
//!   out of ids.
//!
//! A check body that isn't well formed is rejected before it is decided,
//! so it isn't audited.

mod facts;
pub mod memory;
mod namespaces;

use std::{
    num::NonZeroU32,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Deserializer, Serialize, de};
use sungma::{
    name::{NameError, SubjectName},
    revision::Revision,
    store::{DocumentReader, NameReader, ReadingDictionary},
};
use sungma_check::{
    CheckRequest, CheckService, Failure, MemoryAuditLog, RequestContext, Verdict, read::ReadError,
};
use sungma_check::{store::FactReader, store::NamespaceReader};
use sungma_lang::{fact, namespace};
use sungma_write::{
    intern::LeasingInterner,
    service::{
        FactWriteService, FactWriteServiceError, NamespaceWriteService, NamespaceWriteServiceError,
    },
    store::{FactWriter, NameWriter, NamespaceWrite, NamespaceWriter},
};
use thiserror::Error;

use memory::Memory;

/// What every route reads through.
pub trait Reader:
    NameReader + FactReader + NamespaceReader + DocumentReader + Send + Sync + 'static
{
}

impl<T> Reader for T where
    T: NameReader + FactReader + NamespaceReader + DocumentReader + Send + Sync + 'static
{
}

/// What every route writes through.
pub trait Writer: NameWriter + FactWriter + NamespaceWriter + Send + Sync + 'static {}

impl<T: NameWriter + FactWriter + NamespaceWriter + Send + Sync + 'static> Writer for T {}

/// How many ids a lease takes at a time.
const LEASE: NonZeroU32 = NonZeroU32::new(64).unwrap();

/// Why the seed data didn't load.
#[derive(Debug, Error)]
pub enum LoadError {
    #[error(transparent)]
    Refused(#[from] namespace::Refusal),
    #[error(transparent)]
    Namespace(#[from] NamespaceWriteServiceError),
    #[error(transparent)]
    Facts(#[from] serde_json::Error),
    #[error(transparent)]
    Fact(#[from] FactWriteServiceError),
}

/// What every request reads and writes through, shared across them.
pub struct AppState<R = Memory, W = Memory> {
    reader: Arc<R>,
    writer: Arc<W>,
    /// Mints the names namespace writes declare.
    interner: LeasingInterner<Arc<R>, Arc<W>>,
    pub audit: MemoryAuditLog,
    /// Numbers requests that arrive without an `x-request-id`.
    requests: AtomicU64,
}

impl AppState {
    /// A state on an empty in-memory store.
    pub fn memory() -> Self {
        let memory = Arc::new(Memory::default());
        Self::new(Arc::clone(&memory), memory)
    }
}

impl<R: Reader, W: Writer> AppState<R, W> {
    pub fn new(reader: Arc<R>, writer: Arc<W>) -> Self {
        Self {
            interner: LeasingInterner::new(Arc::clone(&reader), Arc::clone(&writer), LEASE),
            reader,
            writer,
            audit: MemoryAuditLog::default(),
            requests: AtomicU64::new(0),
        }
    }

    /// Creates each namespace in the documents, then adds the facts in the
    /// fact document, through the write services, as a caller would. Every
    /// document is parsed before anything is written. Meant for an empty
    /// store: creating a namespace that has a version is refused.
    pub async fn seed(&self, namespaces: &[&str], facts: &str) -> Result<(), LoadError> {
        let documents = namespaces
            .iter()
            .map(|document| namespace::parse(document).map_err(namespace::Refusal))
            .collect::<Result<Vec<_>, _>>()?;
        let facts = fact::parse(facts)?;
        let writer = NamespaceWriteService::new(&self.interner, &*self.writer);
        for document in documents {
            writer
                .write(
                    &document.namespace,
                    NamespaceWrite::Create(document.relations),
                )
                .await?;
        }
        if !facts.is_empty() {
            FactWriteService::new(&self.interner, &*self.writer)
                .write(&facts, &[])
                .await?;
        }
        Ok(())
    }

    fn names(&self) -> ReadingDictionary<&R> {
        ReadingDictionary(&*self.reader)
    }

    fn context(&self, headers: &HeaderMap) -> RequestContext {
        let header = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        RequestContext {
            request_id: header("x-request-id").unwrap_or_else(|| {
                let n = self.requests.fetch_add(1, Ordering::Relaxed) + 1;
                format!("sungma-{n}")
            }),
            caller: header("x-caller").unwrap_or_else(|| "anonymous".to_owned()),
            received_at: SystemTime::now(),
        }
    }
}

pub fn router<R: Reader, W: Writer>(state: Arc<AppState<R, W>>) -> Router {
    Router::new()
        .route("/check", post(check::<R, W>))
        .route("/facts/write", post(facts::write::<R, W>))
        .route("/facts/read", post(facts::read::<R, W>))
        .route("/namespaces/write", post(namespaces::write::<R, W>))
        .route("/namespaces/read", post(namespaces::read::<R, W>))
        .with_state(state)
}

/// A revision a caller has seen. Plain JSON for now.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zookie {
    pub revision: u64,
}

impl From<Revision> for Zookie {
    fn from(revision: Revision) -> Self {
        Self {
            revision: revision.0,
        }
    }
}

impl From<Zookie> for Revision {
    fn from(zookie: Zookie) -> Self {
        Self(zookie.revision)
    }
}

/// A reply that gives only the zookie of a write.
#[derive(Serialize)]
struct Written {
    request_id: String,
    zookie: Zookie,
}

/// The subject is two optional keys rather than a flattened enum, because
/// `deny_unknown_fields` doesn't work with `flatten` and a flattened enum
/// silently keeps the first of two subjects. There is no resource key: a
/// fact like `parent` relates resources, but access is only ever checked
/// for an identity.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckBody {
    set: String,
    identity: Option<String>,
    subjectset: Option<String>,
    zookie: Option<Zookie>,
}

#[derive(Serialize)]
struct CheckReply {
    request_id: String,
    verdict: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    zookie: Option<Zookie>,
}

// TODO: move to RFC 9457 Problem Details (application/problem+json), with
// request_id as an extension member.
#[derive(Serialize)]
struct ErrorReply {
    request_id: String,
    error: String,
}

async fn check<R: Reader, W: Writer>(
    State(state): State<Arc<AppState<R, W>>>,
    headers: HeaderMap,
    body: Result<Json<CheckBody>, JsonRejection>,
) -> Response {
    let context = state.context(&headers);
    let request_id = context.request_id.clone();
    let request =
        match accept(body).and_then(|body| body.into_request().map_err(Refused::malformed)) {
            Ok(request) => request,
            Err(refused) => return refused.reply(&request_id),
        };
    let names = state.names();
    let service = CheckService::new(&names, &*state.reader, &*state.reader, &state.audit);
    let (verdict, zookie) = match service.check(context, request).await {
        Ok(Verdict::Decided(decision)) => {
            let verdict = if decision.outcome.is_allowed() {
                "allowed"
            } else {
                "denied"
            };
            (verdict, Some(decision.revision.into()))
        }
        Ok(Verdict::Unknown) => ("unknown", None),
        Ok(Verdict::Failed(failed)) => {
            let status = match failed {
                Failure::Ahead { .. } => StatusCode::BAD_REQUEST,
                Failure::Store(_) => StatusCode::BAD_GATEWAY,
                Failure::Evaluation(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            return failure(status, &request_id, failed);
        }
        Err(error) => return failure(StatusCode::BAD_GATEWAY, &request_id, error),
    };
    let reply = CheckReply {
        request_id,
        verdict,
        zookie,
    };
    Json(reply).into_response()
}

/// A request refused before it reached a service: its status and why.
struct Refused(StatusCode, String);

impl Refused {
    fn malformed(error: impl ToString) -> Self {
        Self(StatusCode::BAD_REQUEST, error.to_string())
    }

    fn reply(self, request_id: &str) -> Response {
        failure(self.0, request_id, self.1)
    }
}

/// The body, or why it's refused. JSON that isn't well formed, or isn't
/// the body the route takes, is 400; any other rejection keeps axum's
/// status, such as 415 for a body not sent as JSON and 413 for one past
/// the size limit.
fn accept<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Refused> {
    body.map(|Json(body)| body).map_err(|rejection| {
        let status = match rejection {
            JsonRejection::JsonDataError(_) | JsonRejection::JsonSyntaxError(_) => {
                StatusCode::BAD_REQUEST
            }
            _ => rejection.status(),
        };
        Refused(status, rejection.body_text())
    })
}

fn failure(status: StatusCode, request_id: &str, error: impl ToString) -> Response {
    let reply = ErrorReply {
        request_id: request_id.to_owned(),
        error: error.to_string(),
    };
    (status, Json(reply)).into_response()
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

impl CheckBody {
    fn into_request(self) -> Result<CheckRequest, String> {
        let set = parse(&self.set)?;
        let subject = match (self.identity, self.subjectset) {
            (Some(identity), None) => SubjectName::Identity(parse(&identity)?),
            (None, Some(set)) => SubjectName::Subjectset(parse(&set)?),
            _ => return Err("expected exactly one of identity or subjectset".to_owned()),
        };
        Ok(CheckRequest {
            set,
            subject,
            zookie: self.zookie.map(Revision::from),
        })
    }
}

fn parse<T: FromStr<Err = NameError>>(text: &str) -> Result<T, String> {
    text.parse().map_err(|error: NameError| error.to_string())
}

/// A name read from a JSON string, refused as the grammar refuses it.
fn named<'de, D: Deserializer<'de>, T: FromStr<Err = NameError>>(
    deserializer: D,
) -> Result<T, D::Error> {
    String::deserialize(deserializer)?
        .parse()
        .map_err(de::Error::custom)
}
