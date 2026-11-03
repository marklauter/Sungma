use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

/// Why the BFF couldn't sign someone in, keep them signed in, or start.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bad configuration: {0}")]
    Config(String),
    #[error("discovery failed: {0}")]
    Discovery(String),
    #[error("no sign-in is in progress in this session")]
    NoSignIn,
    #[error("the callback's state doesn't match the sign-in's")]
    StateMismatch,
    #[error("the identity provider refused the sign-in: {0}")]
    Refused(String),
    #[error("the callback carries no code")]
    NoCode,
    #[error("the token exchange failed: {0}")]
    Token(String),
    #[error("the ID token was refused: {0}")]
    IdToken(String),
    #[error("not signed in")]
    SignedOut,
    #[error("the identity provider is unavailable: {0}")]
    Unavailable(String),
    #[error("session store: {0}")]
    Session(#[from] tower_sessions::session::Error),
}

impl IntoResponse for Error {
    /// A sign-in that fails is a bad request, without its reason: the
    /// browser learns nothing about the tokens or the identity provider.
    fn into_response(self) -> Response {
        match self {
            Self::Config(_) | Self::Discovery(_) | Self::Session(_) => {
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
            Self::SignedOut => {
                let mut response = StatusCode::UNAUTHORIZED.into_response();
                response.extensions_mut().insert(crate::session::Ended);
                response
            }
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
            Self::NoSignIn
            | Self::StateMismatch
            | Self::Refused(_)
            | Self::NoCode
            | Self::Token(_)
            | Self::IdToken(_) => (StatusCode::BAD_REQUEST, "Sign-in failed.").into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(error: Error) -> StatusCode {
        error.into_response().status()
    }

    #[test]
    fn a_failed_sign_in_is_a_bad_request() {
        for error in [
            Error::NoSignIn,
            Error::StateMismatch,
            Error::Refused("access_denied".to_owned()),
            Error::NoCode,
            Error::Token("invalid_grant".to_owned()),
            Error::IdToken("expired".to_owned()),
        ] {
            assert_eq!(status(error), StatusCode::BAD_REQUEST);
        }
    }

    #[test]
    fn a_broken_bff_is_a_server_error() {
        for error in [
            Error::Config("bad URL".to_owned()),
            Error::Discovery("unreachable".to_owned()),
        ] {
            assert_eq!(status(error), StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    #[test]
    fn an_identity_provider_that_can_t_answer_is_unavailable() {
        assert_eq!(
            status(Error::Unavailable("timed out".to_owned())),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn signed_out_is_unauthorized() {
        assert_eq!(status(Error::SignedOut), StatusCode::UNAUTHORIZED);
    }
}
