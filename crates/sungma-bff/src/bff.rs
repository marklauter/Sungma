use std::{sync::Arc, time::Duration};

use axum::{
    Extension, Router,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use openidconnect::{
    AccessToken, ClientId, ClientSecret, EndSessionUrl, EndpointMaybeSet, EndpointNotSet,
    EndpointSet, IssuerUrl, OAuth2TokenResponse, PostLogoutRedirectUrl, ProviderMetadataWithLogout,
    RedirectUrl, RefreshToken, Scope,
    core::{CoreClient, CoreIdTokenVerifier},
    reqwest,
};
use sungma_clock::TimeSource;
use tower_sessions::{Session, SessionManagerLayer, SessionStore, cookie::SameSite};

use crate::{
    Error, header, routes,
    session::{SIGNED_IN, SignedIn},
};

/// An access token this close to expiring is refreshed before it's
/// handed out, so it doesn't expire on its way to Sungma.
const SKEW: Duration = Duration::from_secs(60);

/// The session cookie's name.
const COOKIE: &str = "sungma";

/// The OIDC client discovery builds: an authorization endpoint, and a
/// token endpoint if the identity provider names one.
pub(crate) type Client = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

/// How the BFF reaches the identity provider and where it sends people.
pub struct Config {
    /// The identity provider's issuer URL, where discovery starts.
    pub issuer: String,
    /// The client the identity provider registered for this BFF.
    pub client_id: String,
    pub client_secret: String,
    /// This service's origin, such as `https://author.example.com`. The
    /// identity provider sends the browser back to `/auth/callback` on it,
    /// and to `/` after signing out.
    pub origin: String,
    /// The scopes asked for besides `openid`, such as `profile` and
    /// `email`.
    pub scopes: Vec<String>,
    /// Where a person who isn't signed in is sent, such as `/signin`.
    pub sign_in_page: String,
}

/// An OAuth backend-for-frontend for one identity provider.
pub struct Bff {
    pub(crate) client: Client,
    pub(crate) http: reqwest::Client,
    pub(crate) time: Arc<dyn TimeSource>,
    pub(crate) scopes: Vec<Scope>,
    pub(crate) end_session: Option<EndSessionUrl>,
    pub(crate) post_logout: PostLogoutRedirectUrl,
    sign_in_page: String,
}

impl Bff {
    /// Reads the identity provider's discovery document and builds the
    /// client. `time` is the clock tokens are checked against.
    pub async fn discover(config: Config, time: Arc<dyn TimeSource>) -> Result<Arc<Self>, Error> {
        // A redirect from the identity provider's back channel is refused,
        // not followed: the BFF talks only to the URLs discovery gave it.
        let http = reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| Error::Config(error.to_string()))?;
        let issuer =
            IssuerUrl::new(config.issuer).map_err(|error| Error::Config(error.to_string()))?;
        let redirect = RedirectUrl::new(format!("{}/auth/callback", config.origin))
            .map_err(|error| Error::Config(error.to_string()))?;
        let post_logout = PostLogoutRedirectUrl::new(format!("{}/", config.origin))
            .map_err(|error| Error::Config(error.to_string()))?;
        let metadata = ProviderMetadataWithLogout::discover_async(issuer, &http)
            .await
            .map_err(|error| Error::Discovery(error.to_string()))?;
        let end_session = metadata.additional_metadata().end_session_endpoint.clone();
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(config.client_id),
            Some(ClientSecret::new(config.client_secret)),
        )
        .set_redirect_uri(redirect);
        Ok(Arc::new(Self {
            client,
            http,
            time,
            scopes: config.scopes.into_iter().map(Scope::new).collect(),
            end_session,
            post_logout,
            sign_in_page: config.sign_in_page,
        }))
    }

    /// Adds the BFF's routes to `app`, and its layers around both: the
    /// session, the BFF itself for the [`User`](crate::User) extractor,
    /// and the header check. `store` keeps the sessions.
    pub fn wrap<S: SessionStore + Clone>(self: &Arc<Self>, app: Router, store: S) -> Router {
        let sessions = SessionManagerLayer::new(store)
            .with_name(COOKIE)
            .with_secure(true)
            .with_http_only(true)
            // Lax, not Strict: the identity provider's redirect to the
            // callback is a cross-site navigation, and it must carry the
            // cookie to find the sign-in it finishes.
            .with_same_site(SameSite::Lax);
        let auth = Router::new()
            .route("/auth/login", get(routes::login))
            .route("/auth/callback", get(routes::callback))
            .route("/auth/logout", post(routes::logout))
            .with_state(Arc::clone(self));
        app.merge(auth)
            .layer(axum::middleware::from_fn(header::require_header))
            .layer(Extension(Arc::clone(self)))
            .layer(sessions)
    }

    /// The signed-in person's access token, for a call to Sungma's APIs.
    /// A token about to expire is refreshed first. A session with no
    /// sign-in, or whose refresh fails, is ended, and the call gets
    /// [`Error::SignedOut`].
    pub async fn access_token(&self, session: &Session) -> Result<AccessToken, Error> {
        let mut signed_in: SignedIn = session.get(SIGNED_IN).await?.ok_or(Error::SignedOut)?;
        let now = self.time.wall();
        let fresh = signed_in
            .tokens
            .expires_at
            .is_none_or(|at| now < at.saturating_sub(nanos(SKEW)));
        if fresh {
            return Ok(AccessToken::new(signed_in.tokens.access));
        }
        let Some(refresh) = signed_in.tokens.refresh.clone() else {
            session.flush().await?;
            return Err(Error::SignedOut);
        };
        let Ok(response) = self.refresh(refresh).await else {
            session.flush().await?;
            return Err(Error::SignedOut);
        };
        signed_in.tokens.access = response.access_token().secret().clone();
        signed_in.tokens.expires_at = self.expires_at(response.expires_in());
        if let Some(refresh) = response.refresh_token() {
            signed_in.tokens.refresh = Some(refresh.secret().clone());
        }
        session.insert(SIGNED_IN, &signed_in).await?;
        Ok(response.access_token().clone())
    }

    async fn refresh(
        &self,
        refresh: String,
    ) -> Result<openidconnect::core::CoreTokenResponse, Error> {
        self.client
            .exchange_refresh_token(&RefreshToken::new(refresh))
            .map_err(|error| Error::Token(error.to_string()))?
            .request_async(&self.http)
            .await
            .map_err(|error| Error::Token(error.to_string()))
    }

    /// Where a person who isn't signed in is sent.
    pub fn sign_in_page(&self) -> &str {
        &self.sign_in_page
    }

    /// When a token that lasts `lifetime` from now expires.
    pub(crate) fn expires_at(&self, lifetime: Option<Duration>) -> Option<u64> {
        lifetime.map(|lifetime| self.time.wall().saturating_add(nanos(lifetime)))
    }

    /// The ID token verifier, on the BFF's clock.
    pub(crate) fn verifier(&self) -> CoreIdTokenVerifier<'_> {
        let time = Arc::clone(&self.time);
        self.client
            .id_token_verifier()
            .set_time_fn(move || utc(time.wall()))
    }
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn utc(nanos: u64) -> DateTime<Utc> {
    DateTime::from_timestamp_nanos(i64::try_from(nanos).unwrap_or(i64::MAX))
}
