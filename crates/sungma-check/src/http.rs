//! The Check service's route, `POST /check`, which judges one fact. See
//! `sungma-http` for what every route shares.
//!
//! It names the fact in the string form:
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
//! Beyond the statuses every route shares, a zookie ahead of the latest
//! revision is 400, a store that failed is 502, and a check past one of
//! Sungma's limits is 500. A body that isn't well formed is rejected
//! before it is decided, so it isn't audited.

use std::{str::FromStr, sync::Arc, time::SystemTime};

use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use sungma::{
    name::{NameError, SubjectName},
    revision::Revision,
};
use sungma_http::{Refused, Requests, Zookie, accept, failure, header};
use sungma_names::store::Dictionary;

use crate::{
    CheckRequest, CheckService, Failure, MemoryAuditLog, RequestContext, Verdict,
    store::{FactReader, NamespaceReader},
};

/// What every check reads through, shared across them: the names, and
/// the facts and namespace versions.
pub struct AppState<D, R> {
    names: D,
    reader: R,
    pub audit: MemoryAuditLog,
    requests: Requests,
}

impl<D, R> AppState<D, R> {
    pub fn new(names: D, reader: R) -> Self {
        Self {
            names,
            reader,
            audit: MemoryAuditLog::default(),
            requests: Requests::default(),
        }
    }

    fn context(&self, headers: &HeaderMap) -> RequestContext {
        RequestContext {
            request_id: self.requests.id(headers),
            caller: header(headers, "x-caller").unwrap_or_else(|| "anonymous".to_owned()),
            received_at: SystemTime::now(),
        }
    }
}

pub fn router<D, R>(state: Arc<AppState<D, R>>) -> Router
where
    D: Dictionary + Send + Sync + 'static,
    R: NamespaceReader + FactReader + Send + Sync + 'static,
{
    Router::new()
        .route("/check", post(check::<D, R>))
        .with_state(state)
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

async fn check<D, R>(
    State(state): State<Arc<AppState<D, R>>>,
    headers: HeaderMap,
    body: Result<Json<CheckBody>, JsonRejection>,
) -> Response
where
    D: Dictionary + Sync,
    R: NamespaceReader + FactReader + Sync,
{
    let context = state.context(&headers);
    let request_id = context.request_id.clone();
    let request =
        match accept(body).and_then(|body| body.into_request().map_err(Refused::malformed)) {
            Ok(request) => request,
            Err(refused) => return refused.reply(&request_id),
        };
    let service = CheckService::new(&state.names, &state.reader, &state.reader, &state.audit);
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
