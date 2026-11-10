//! The Write service's routes, `POST /facts/write`,
//! `POST /namespaces/write` and `POST /namespaces/retire`. See
//! `sungma-http` for what every route shares.
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
//! A namespace write's body is a theory document, in TDL, that declares
//! the one namespace written, sent as `application/vnd.sungma.tdl`. A browser can't send that
//! type across sites without a CORS preflight, so a page on another site
//! can't write a namespace with a caller's cookies:
//!
//! ```text
//! POST /namespaces/write
//! Content-Type: application/vnd.sungma.tdl
//! If-Match: "12"
//!
//! namespace: file
//!   owner<@identity>
//!   viewer: owner
//! ```
//!
//! Without `If-Match` it creates the namespace; with it, it writes a new
//! version based on the version whose revision `If-Match` names, a strong
//! entity tag holding the revision of that version's zookie. A retire
//! names the namespace in its query and the version it retires in
//! `If-Match`, and has no body:
//!
//! ```text
//! POST /namespaces/retire?namespace=file
//! If-Match: "14"
//! ```
//!
//! Each gives the zookie of its revision. A write that doesn't follow the
//! latest version is refused with the zookie of the latest version as
//! `latest`, `null` when there's none, so the caller can read it and try
//! again.
//!
//! Beyond the statuses every route shares:
//!
//! - 400: a namespace write whose body isn't UTF-8, or an `If-Match` that
//!   isn't one revision as a strong entity tag.
//! - 409: a namespace write or retire that doesn't follow the latest
//!   version.
//! - 413: a fact write past what one write holds, a namespace write whose
//!   body is past [`MAX_NAMESPACE_BODY_BYTES`], or a namespace version
//!   naming more namespaces than one write checks.
//! - 415: a namespace write not sent as `application/vnd.sungma.tdl`.
//! - 422: a refused write. A refused namespace document is 422 whatever
//!   its problems, names included, so it lists them all in one reply, and
//!   so is a document that doesn't declare exactly one namespace, a
//!   version naming a relation another namespace doesn't declare, a fact
//!   whose subject its relation's constraints don't admit, or a retire of
//!   a namespace with no version.
//! - 428: a retire without `If-Match`.
//! - 502: the store or the clock failed.
//! - 500: one of Sungma's limits, such as running out of ids.

use std::sync::Arc;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, Query, State,
        rejection::{BytesRejection, JsonRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode, header::IF_MATCH},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use sungma::{
    name::{NamespaceName, RelationName},
    revision::Revision,
};
use sungma_http::{Refused, Requests, Zookie, accept, failure, header, named};
use sungma_lang::{
    fact::WriteRequest,
    tdl::{self, Refusal},
};
use sungma_names::store::{InternError, Interner};

use crate::{
    service::{
        FactWriteService, FactWriteServiceError, NamespaceWriteService, NamespaceWriteServiceError,
    },
    store::{FactWriter, NamespaceWrite, NamespaceWriteError, NamespaceWriter},
};

/// The largest namespace write body, in bytes. The parser has no size
/// limit, so the edge bounds what a caller can send it.
pub const MAX_NAMESPACE_BODY_BYTES: usize = 2 * 1024 * 1024;

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
        .route(
            "/namespaces/write",
            post(namespaces::<I, W>).layer(DefaultBodyLimit::max(MAX_NAMESPACE_BODY_BYTES)),
        )
        .route("/namespaces/retire", post(retire::<I, W>))
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
                FactWriteServiceError::Unknown(_)
                | FactWriteServiceError::Undeclared(_)
                | FactWriteServiceError::Unsatisfied(_) => StatusCode::UNPROCESSABLE_ENTITY,
                FactWriteServiceError::TooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
                FactWriteServiceError::Clock(_) | FactWriteServiceError::Store(_) => {
                    StatusCode::BAD_GATEWAY
                }
            };
            failure(status, &request_id, error)
        }
    }
}

/// A namespace write as the caller names it.
type Named = NamespaceWrite<RelationName, NamespaceName>;

/// The media type of a namespace write's body, the TDL text of one
/// namespace.
pub const TDL_MEDIA_TYPE: &str = "application/vnd.sungma.tdl";

/// Whether the content type is `application/vnd.sungma.tdl`, with no parameter but an
/// optional `charset=utf-8`, case aside.
fn is_tdl(headers: &HeaderMap) -> bool {
    let Some(value) = header(headers, "content-type") else {
        return false;
    };
    let mut parts = value.split(';').map(str::trim);
    let essence = parts.next().unwrap_or_default();
    essence.eq_ignore_ascii_case(TDL_MEDIA_TYPE)
        && parts.all(|parameter| {
            parameter.split_once('=').is_some_and(|(name, value)| {
                let value = value.trim();
                let value = value
                    .strip_prefix('"')
                    .and_then(|value| value.strip_suffix('"'))
                    .unwrap_or(value);
                name.trim().eq_ignore_ascii_case("charset") && value.eq_ignore_ascii_case("utf-8")
            })
        })
}

/// The revision `If-Match` names, `None` without the header. Its value is
/// one strong entity tag holding a revision, such as `"12"`.
fn if_match(headers: &HeaderMap) -> Result<Option<Revision>, Refused> {
    let mut values = headers.get_all(IF_MATCH).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    let malformed = || {
        Refused::malformed("If-Match must be one revision as a strong entity tag, such as \"12\"")
    };
    if values.next().is_some() {
        return Err(malformed());
    }
    let revision = value
        .to_str()
        .ok()
        .and_then(|value| value.trim().strip_prefix('"')?.strip_suffix('"'))
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|digits| digits.parse().ok())
        .ok_or_else(malformed)?;
    Ok(Some(Revision(revision)))
}

/// The namespace a write names, and the write: a create without
/// `If-Match`, a new version based on the revision it names with one.
fn namespace_write(headers: &HeaderMap, body: &Bytes) -> Result<(NamespaceName, Named), Refused> {
    if !is_tdl(headers) {
        return Err(Refused(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            format!("a namespace write is sent as {TDL_MEDIA_TYPE}"),
        ));
    }
    let based_on = if_match(headers)?;
    let text = std::str::from_utf8(body).map_err(Refused::malformed)?;
    let document = tdl::parse_one(text).map_err(|errors| {
        Refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            Refusal(errors).to_string(),
        )
    })?;
    let write = match based_on {
        None => NamespaceWrite::Create(document.relations),
        Some(based_on) => NamespaceWrite::NewVersion {
            based_on,
            namespace: document.relations,
        },
    };
    Ok((document.namespace, write))
}

/// The query `POST /namespaces/retire` takes.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetireQuery {
    #[serde(deserialize_with = "named")]
    namespace: NamespaceName,
}

/// The namespace a retire names, and the write that retires the version
/// `If-Match` names.
fn retire_write(
    headers: &HeaderMap,
    query: Result<Query<RetireQuery>, QueryRejection>,
) -> Result<(NamespaceName, Named), Refused> {
    let Query(query) = query.map_err(|rejection| Refused::malformed(rejection.body_text()))?;
    let based_on = if_match(headers)?.ok_or_else(|| {
        Refused(
            StatusCode::PRECONDITION_REQUIRED,
            "a retire names the version it retires in If-Match".into(),
        )
    })?;
    Ok((query.namespace, NamespaceWrite::Retire { based_on }))
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
    body: Result<Bytes, BytesRejection>,
) -> Response
where
    I: Interner + Sync,
    W: NamespaceWriter + Sync,
{
    let request_id = state.requests.id(&headers);
    let write = body
        .map_err(|rejection| Refused(rejection.status(), rejection.body_text()))
        .and_then(|body| namespace_write(&headers, &body));
    match write {
        Ok((name, write)) => write_namespace(&state, request_id, &name, write).await,
        Err(refused) => refused.reply(&request_id),
    }
}

async fn retire<I, W>(
    State(state): State<Arc<AppState<I, W>>>,
    headers: HeaderMap,
    query: Result<Query<RetireQuery>, QueryRejection>,
) -> Response
where
    I: Interner + Sync,
    W: NamespaceWriter + Sync,
{
    let request_id = state.requests.id(&headers);
    match retire_write(&headers, query) {
        Ok((name, write)) => write_namespace(&state, request_id, &name, write).await,
        Err(refused) => refused.reply(&request_id),
    }
}

async fn write_namespace<I, W>(
    state: &AppState<I, W>,
    request_id: String,
    name: &NamespaceName,
    write: Named,
) -> Response
where
    I: Interner + Sync,
    W: NamespaceWriter + Sync,
{
    let service = NamespaceWriteService::new(&state.interner, &state.writer);
    match service.write(name, write).await {
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
        Write(NamespaceWriteError::NothingToRetire | NamespaceWriteError::References(_)) => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        Write(NamespaceWriteError::Conflict { .. }) => StatusCode::CONFLICT,
        Write(NamespaceWriteError::TooLarge { .. }) => StatusCode::PAYLOAD_TOO_LARGE,
        Intern(InternError::Exhausted(_)) => StatusCode::INTERNAL_SERVER_ERROR,
        Intern(InternError::Store(_))
        | Write(NamespaceWriteError::Clock(_) | NamespaceWriteError::Store(_)) => {
            StatusCode::BAD_GATEWAY
        }
    }
}
