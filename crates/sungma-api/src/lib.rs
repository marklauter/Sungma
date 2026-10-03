//! The Check web API: JSON over HTTP, on axum.
//!
//! `POST /check` judges one fact, named in the fixture's string form:
//!
//! ```json
//! { "set": "file:design.md#viewer", "identity": "alice", "zookie": { "revision": 12 } }
//! ```
//!
//! The subject is an `identity` or a `subjectset` (`theory:id#relation`).
//! The zookie is optional, and is plain JSON for now so it stays readable.
//! The reply names the verdict and, once decided, the zookie of the
//! revision it was decided at:
//!
//! ```json
//! { "request_id": "sungma-1", "verdict": "allowed", "zookie": { "revision": 12 } }
//! ```
//!
//! The `x-request-id` and `x-caller` headers fill the audit record's
//! context. Neither is authenticated yet.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use sungma_core::{
    check::{CheckRequest, CheckService, MemoryAuditLog, RequestContext, SubjectName, Verdict},
    fixture::{self, FixtureError},
    memory::{MemoryDictionary, MemoryFactStore},
    model::Revision,
    theory::Theories,
};

/// What every request reads, shared across them.
#[derive(Debug, Default)]
pub struct AppState {
    pub dictionary: MemoryDictionary,
    pub theories: Theories,
    pub facts: MemoryFactStore,
    pub audit: MemoryAuditLog,
    /// Numbers requests that arrive without an `x-request-id`.
    requests: AtomicU64,
}

impl AppState {
    /// Loads theory and fact JSON in the [`fixture`] forms.
    pub fn load(theories: &str, facts: &str) -> Result<Self, FixtureError> {
        let mut state = Self::default();
        fixture::load_theories(theories, &mut state.dictionary, &mut state.theories)?;
        fixture::load_facts(facts, &mut state.dictionary, &mut state.facts)?;
        Ok(state)
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

pub fn router(state: Arc<AppState>) -> Router {
    Router::new().route("/check", post(check)).with_state(state)
}

/// A revision a caller has seen. Plain JSON for now.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Zookie {
    pub revision: u64,
}

#[derive(Deserialize)]
struct CheckBody {
    set: String,
    #[serde(flatten)]
    subject: SubjectBody,
    zookie: Option<Zookie>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SubjectBody {
    Identity(String),
    Subjectset(String),
}

#[derive(Serialize)]
struct CheckReply {
    request_id: String,
    verdict: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    zookie: Option<Zookie>,
}

#[derive(Serialize)]
struct ErrorReply {
    request_id: String,
    error: String,
}

async fn check(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CheckBody>,
) -> Response {
    let context = state.context(&headers);
    let request_id = context.request_id.clone();
    let request = match body.into_request() {
        Ok(request) => request,
        Err(error) => return failure(StatusCode::BAD_REQUEST, request_id, error),
    };
    let service = CheckService::new(
        &state.dictionary,
        &state.theories,
        &state.facts,
        &state.audit,
    );
    let (verdict, zookie) = match service.check(context, request).await {
        Ok(Verdict::Decided(decision)) => {
            let verdict = if decision.outcome.is_allowed() {
                "allowed"
            } else {
                "denied"
            };
            let zookie = Zookie {
                revision: decision.revision.0,
            };
            (verdict, Some(zookie))
        }
        Ok(Verdict::Unknown) => ("unknown", None),
        Ok(Verdict::Failed(error)) => {
            return failure(StatusCode::INTERNAL_SERVER_ERROR, request_id, error);
        }
        Err(error) => {
            return failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                request_id,
                error.to_string(),
            );
        }
    };
    let reply = CheckReply {
        request_id,
        verdict,
        zookie,
    };
    Json(reply).into_response()
}

fn failure(status: StatusCode, request_id: String, error: String) -> Response {
    (status, Json(ErrorReply { request_id, error })).into_response()
}

impl CheckBody {
    fn into_request(self) -> Result<CheckRequest, String> {
        let (theory, resource, relation) = split_subjectset(&self.set)?;
        let subject = match self.subject {
            SubjectBody::Identity(identity) => SubjectName::Identity(identity),
            SubjectBody::Subjectset(set) => {
                let (theory, resource, relation) = split_subjectset(&set)?;
                SubjectName::Subjectset {
                    theory,
                    resource,
                    relation,
                }
            }
        };
        Ok(CheckRequest {
            theory,
            resource,
            relation,
            subject,
            zookie: self.zookie.map(|zookie| Revision(zookie.revision)),
        })
    }
}

/// `theory:id#relation`, split at the first `:` and the last `#`.
fn split_subjectset(text: &str) -> Result<(String, String, String), String> {
    let malformed = || format!("malformed subjectset {text:?}, expected theory:id#relation");
    let (resource, relation) = text.rsplit_once('#').ok_or_else(malformed)?;
    let (theory, id) = resource.split_once(':').ok_or_else(malformed)?;
    Ok((theory.to_owned(), id.to_owned(), relation.to_owned()))
}
