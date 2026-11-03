use std::sync::Arc;

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Redirect, Response},
};
use openidconnect::core::CoreIdTokenClaims;
use serde::{Deserialize, Serialize};
use tower_sessions::Session;

use crate::{Bff, Error};

/// The session key of a sign-in in progress.
pub(crate) const PENDING: &str = "sungma.bff.pending";

/// The session key of a signed-in person.
pub(crate) const SIGNED_IN: &str = "sungma.bff.signed_in";

/// What the callback needs to finish a sign-in: the state the identity
/// provider must send back, the nonce the ID token must carry, and the
/// PKCE verifier for the token exchange.
#[derive(Serialize, Deserialize)]
pub(crate) struct Pending {
    pub state: String,
    pub nonce: String,
    pub verifier: String,
}

/// A signed-in person and their tokens.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SignedIn {
    pub user: User,
    pub tokens: Tokens,
}

/// The tokens a session holds. The browser never sees them.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Tokens {
    pub access: String,
    pub refresh: Option<String>,
    /// When the access token expires, in nanoseconds since the Unix
    /// epoch, or `None` when the identity provider didn't say.
    pub expires_at: Option<u64>,
    /// The ID token, sent back as a hint when signing out.
    pub id_token: String,
}

/// The signed-in person, from their ID token.
///
/// As an extractor, it sends a person who isn't signed in to the sign-in
/// page: by a redirect, or by `HX-Redirect` for a request from HTMX.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// The `sub` claim: the identity Sungma's facts hold.
    pub subject: String,
    pub name: Option<String>,
    pub username: Option<String>,
    pub email: Option<String>,
}

impl User {
    pub(crate) fn from_claims(claims: &CoreIdTokenClaims) -> Self {
        Self {
            subject: claims.subject().to_string(),
            name: claims
                .name()
                .and_then(|name| name.get(None))
                .map(|name| name.to_string()),
            username: claims.preferred_username().map(|name| name.to_string()),
            email: claims.email().map(|email| email.to_string()),
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for User {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Response> {
        let Some(bff) = parts.extensions.get::<Arc<Bff>>().cloned() else {
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        };
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        match session.get::<SignedIn>(SIGNED_IN).await {
            Ok(Some(signed_in)) => Ok(signed_in.user),
            Ok(None) => Err(to_sign_in(parts, bff.sign_in_page())),
            Err(error) => Err(Error::from(error).into_response()),
        }
    }
}

/// Sends the browser to the sign-in page. HTMX doesn't follow a redirect
/// for the page, only for the fragment it asked for, so it gets
/// `HX-Redirect` instead.
fn to_sign_in(parts: &Parts, page: &str) -> Response {
    if parts.headers.contains_key("hx-request") {
        ([("hx-redirect", page)], StatusCode::OK).into_response()
    } else {
        Redirect::to(page).into_response()
    }
}
