//! Author, Sungma's web UI: pages that HTMX drives, behind the OAuth
//! backend-for-frontend in `sungma-bff`.
//!
//! [`app`] serves:
//!
//! - `GET /signin`, the sign-in page, whose button starts a sign-in at
//!   the identity provider.
//! - `GET /`, Home, for the signed-in person: the masthead, with sign-out
//!   in their menu, and the You panel.
//! - `GET /static/…`, the stylesheet, scripts and logo the pages use.
//!
//! The BFF adds the sign-in, callback and sign-out routes.

mod assets;
mod pages;
mod settings;

use std::sync::Arc;

use axum::{Router, routing::get};
use sungma_bff::Bff;
use tower_sessions::SessionStore;

pub use settings::{Missing, Settings};

/// Author's pages behind `bff`, with sessions kept in `store`. `provider`
/// names the identity provider on the sign-in button.
pub fn app<S: SessionStore + Clone>(bff: &Arc<Bff>, provider: String, store: S) -> Router {
    let pages = Router::new()
        .route("/", get(pages::home))
        .route("/signin", get(pages::sign_in))
        .with_state(Arc::new(pages::Author { provider }))
        .merge(assets::router());
    bff.wrap(pages, store)
}
