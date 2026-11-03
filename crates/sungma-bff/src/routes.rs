use std::{str::FromStr, sync::Arc};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use openidconnect::{
    AccessTokenHash, AuthorizationCode, CsrfToken, LogoutRequest, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, PkceCodeVerifier, TokenResponse,
    core::{CoreAuthenticationFlow, CoreIdToken},
};
use serde::Deserialize;
use tower_sessions::Session;

use crate::{
    Bff, Error, User,
    session::{PENDING, Pending, SIGNED_IN, SignedIn, Tokens},
};

/// Starts a sign-in: keeps a fresh state, nonce and PKCE verifier in the
/// session, and sends the browser to the identity provider.
pub(crate) async fn login(
    State(bff): State<Arc<Bff>>,
    session: Session,
) -> Result<Redirect, Error> {
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = bff
        .client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .set_pkce_challenge(challenge);
    for scope in &bff.scopes {
        request = request.add_scope(scope.clone());
    }
    let (url, state, nonce) = request.url();
    let pending = Pending {
        state: state.into_secret(),
        nonce: nonce.secret().clone(),
        verifier: verifier.into_secret(),
    };
    session.insert(PENDING, pending).await?;
    Ok(Redirect::to(url.as_str()))
}

/// What the identity provider sends back to the callback.
#[derive(Deserialize)]
pub(crate) struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Finishes a sign-in: checks the state, trades the code and the PKCE
/// verifier for tokens, checks the ID token, and keeps the person and
/// their tokens in a session with a new id.
pub(crate) async fn callback(
    State(bff): State<Arc<Bff>>,
    session: Session,
    Query(callback): Query<Callback>,
) -> Result<Redirect, Error> {
    // Taken, not read: a sign-in's state and verifier are good once.
    let pending: Pending = session.remove(PENDING).await?.ok_or(Error::NoSignIn)?;
    if callback.state.as_deref() != Some(pending.state.as_str()) {
        return Err(Error::StateMismatch);
    }
    if let Some(error) = callback.error {
        return Err(Error::Refused(error));
    }
    let code = callback.code.ok_or(Error::NoCode)?;
    let response = bff
        .client
        .exchange_code(AuthorizationCode::new(code))
        .map_err(|error| Error::Token(error.to_string()))?
        .set_pkce_verifier(PkceCodeVerifier::new(pending.verifier))
        .request_async(&bff.http)
        .await
        .map_err(|error| Error::Token(error.to_string()))?;
    let id_token = response
        .id_token()
        .ok_or_else(|| Error::IdToken("missing".to_owned()))?;
    let verifier = bff.verifier();
    let claims = id_token
        .claims(&verifier, &Nonce::new(pending.nonce))
        .map_err(|error| Error::IdToken(error.to_string()))?;
    // The access token must be the one the ID token was issued with, not
    // another person's swapped in.
    if let Some(expected) = claims.access_token_hash() {
        let signing_alg = id_token
            .signing_alg()
            .map_err(|error| Error::IdToken(error.to_string()))?;
        let signing_key = id_token
            .signing_key(&verifier)
            .map_err(|error| Error::IdToken(error.to_string()))?;
        let actual = AccessTokenHash::from_token(response.access_token(), signing_alg, signing_key)
            .map_err(|error| Error::IdToken(error.to_string()))?;
        if actual != *expected {
            return Err(Error::IdToken(
                "the access token's hash doesn't match".to_owned(),
            ));
        }
    }
    let signed_in = SignedIn {
        user: User::from_claims(claims),
        tokens: Tokens {
            access: response.access_token().secret().clone(),
            refresh: response
                .refresh_token()
                .map(|refresh| refresh.secret().clone()),
            expires_at: bff.expires_at(response.expires_in()),
            id_token: id_token.to_string(),
        },
    };
    // A new session id, so an id planted before sign-in isn't signed in.
    session.cycle_id().await?;
    session.insert(SIGNED_IN, signed_in).await?;
    Ok(Redirect::to("/"))
}

/// Ends the session and sends the browser, by `HX-Redirect`, to the
/// identity provider's end-session endpoint, or home when it has none.
pub(crate) async fn logout(
    State(bff): State<Arc<Bff>>,
    session: Session,
) -> Result<Response, Error> {
    let signed_in: Option<SignedIn> = session.get(SIGNED_IN).await?;
    session.flush().await?;
    let target = match &bff.end_session {
        Some(endpoint) => {
            let mut request = LogoutRequest::from(endpoint.clone())
                .set_client_id(bff.client.client_id().clone())
                .set_post_logout_redirect_uri(bff.post_logout.clone());
            let hint = signed_in.and_then(|s| CoreIdToken::from_str(&s.tokens.id_token).ok());
            if let Some(hint) = hint {
                request = request.set_id_token_hint(&hint);
            }
            request.http_get_url().to_string()
        }
        None => bff.post_logout.to_string(),
    };
    Ok(([("hx-redirect", target)], StatusCode::OK).into_response())
}
