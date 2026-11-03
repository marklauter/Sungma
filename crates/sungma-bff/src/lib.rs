//! An OAuth backend-for-frontend (BFF) on axum.
//!
//! The BFF signs a person in at the organization's identity provider with
//! the authorization code flow and PKCE, keeps their tokens in a
//! server-side session, and gives the browser only a session cookie. A
//! handler takes the signed-in [`User`] as an extractor, and
//! [`Bff::access_token`] hands it a token for calls to Sungma's APIs,
//! refreshed over the back channel when it is about to expire.
//!
//! [`Bff::wrap`] adds the BFF's routes and layers to an app:
//!
//! - `GET /auth/login` sends the browser to the identity provider.
//! - `GET /auth/callback` takes it back, trades the code for tokens and
//!   checks the ID token.
//! - `POST /auth/logout` ends the session and sends the browser to the
//!   identity provider's end-session endpoint.
//!
//! Every request that isn't GET, HEAD, OPTIONS or TRACE must carry
//! `HX-Request: true`, which HTMX adds and a cross-site form can't, or
//! the BFF refuses it.

mod bff;
mod error;
mod header;
mod refreshes;
mod routes;
mod session;
#[cfg(feature = "testkit")]
pub mod testkit;

pub use bff::{Bff, Config};
pub use error::Error;
pub use openidconnect::AccessToken;
pub use session::User;
