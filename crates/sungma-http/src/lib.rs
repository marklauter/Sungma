//! What Sungma's HTTP services share. Each is JSON over HTTP, on axum:
//! every route is a `POST` taking and giving JSON, and every reply carries
//! `request_id`, the request's `x-request-id` header or, without one, a
//! number the service gives it.
//!
//! Every error reply is JSON, `{"request_id": .., "error": ..}`. A body
//! that isn't well formed, or a name that breaks the grammar, is 400; a
//! body past the size limit is 413; a body that isn't sent as JSON is 415.

use std::{
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
};

use axum::{
    Json,
    extract::rejection::JsonRejection,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Deserializer, Serialize, de};
use sungma::{name::NameError, revision::Revision};

/// Numbers requests that arrive without an `x-request-id`.
#[derive(Debug, Default)]
pub struct Requests(AtomicU64);

impl Requests {
    /// The request's `x-request-id`, or else the next number.
    pub fn id(&self, headers: &HeaderMap) -> String {
        header(headers, "x-request-id").unwrap_or_else(|| {
            let n = self.0.fetch_add(1, Ordering::Relaxed) + 1;
            format!("sungma-{n}")
        })
    }
}

/// A header's value, `None` when it's missing or isn't visible ASCII.
pub fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
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

// TODO: move to RFC 9457 Problem Details (application/problem+json), with
// request_id as an extension member.
#[derive(Serialize)]
struct ErrorReply {
    request_id: String,
    error: String,
}

/// A request refused before it reached a service: its status and why.
#[derive(Debug)]
pub struct Refused(pub StatusCode, pub String);

impl Refused {
    pub fn malformed(error: impl ToString) -> Self {
        Self(StatusCode::BAD_REQUEST, error.to_string())
    }

    pub fn reply(self, request_id: &str) -> Response {
        failure(self.0, request_id, self.1)
    }
}

/// The body, or why it's refused. JSON that isn't well formed, or isn't
/// the body the route takes, is 400; any other rejection keeps axum's
/// status, such as 415 for a body not sent as JSON and 413 for one past
/// the size limit.
pub fn accept<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Refused> {
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

/// An error reply.
pub fn failure(status: StatusCode, request_id: &str, error: impl ToString) -> Response {
    let reply = ErrorReply {
        request_id: request_id.to_owned(),
        error: error.to_string(),
    };
    (status, Json(reply)).into_response()
}

/// A name read from a JSON string, refused as the grammar refuses it.
pub fn named<'de, D: Deserializer<'de>, T: FromStr<Err = NameError>>(
    deserializer: D,
) -> Result<T, D::Error> {
    String::deserialize(deserializer)?
        .parse()
        .map_err(de::Error::custom)
}
