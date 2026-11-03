use std::{sync::Arc, time::Duration};

use axum::{
    Extension, Router,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use openidconnect::{
    AccessToken, AccessTokenHash, ClaimsVerificationError, ClientId, ClientSecret, EndSessionUrl,
    EndpointMaybeSet, EndpointNotSet, EndpointSet, IssuerUrl, JsonWebKeySetUrl, Nonce,
    NonceVerifier, OAuth2TokenResponse, PostLogoutRedirectUrl, ProviderMetadataWithLogout,
    RedirectUrl, RefreshToken, RequestTokenError, Scope, SignatureVerificationError, TokenResponse,
    core::{
        CoreClient, CoreErrorResponseType, CoreIdToken, CoreIdTokenClaims, CoreIdTokenVerifier,
        CoreJsonWebKeySet, CoreJwsSigningAlgorithm,
    },
    reqwest,
};
use parking_lot::Mutex;
use sungma_clock::TimeSource;
use tower_sessions::{Session, SessionManagerLayer, SessionStore, cookie::SameSite};

use crate::{
    Error, User, header,
    refreshes::Refreshes,
    routes,
    session::{self, SIGNED_IN, SignedIn, Tokens},
};

/// An access token is refreshed this long before it expires, or halfway
/// through its life if that comes first, so it doesn't expire on its way
/// to Sungma.
const SKEW: Duration = Duration::from_secs(60);

/// The signing keys are fetched again, for a key id they don't hold, at
/// most this often.
const REFETCH: Duration = Duration::from_secs(60);

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
    /// How long a call to the identity provider may take.
    pub timeout: Duration,
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
    refreshes: Refreshes<Result<SignedIn, Failure>>,
    id_tokens: IdTokens,
}

/// What ID tokens are checked against. The signing keys change when the
/// identity provider rotates them.
struct IdTokens {
    issuer: IssuerUrl,
    secret: ClientSecret,
    algs: Vec<CoreJwsSigningAlgorithm>,
    keys_url: JsonWebKeySetUrl,
    keys: Mutex<Keys>,
}

struct Keys {
    set: CoreJsonWebKeySet,
    /// When the keys were last fetched again, in nanoseconds since the
    /// Unix epoch.
    refetched: Option<u64>,
}

/// Why a refresh failed. A refresh's result is shared among the requests
/// waiting on it, so it is cloned, which [`Error`] isn't.
#[derive(Clone)]
enum Failure {
    SignedOut,
    Unavailable(String),
}

/// Why an ID token didn't pass.
pub(crate) enum Check {
    /// Its signature or its access token's hash doesn't hold: a forgery,
    /// or a token swapped in.
    Forged(String),
    /// No signing key under its key id.
    NoKey,
    /// Another check failed, such as its expiry or its audience.
    Failed(String),
}

impl Check {
    pub(crate) fn reason(self) -> String {
        match self {
            Self::Forged(reason) | Self::Failed(reason) => reason,
            Self::NoKey => "no signing key under its key id".to_owned(),
        }
    }
}

impl Bff {
    /// Reads the identity provider's discovery document and builds the
    /// client. `time` is the clock tokens are checked against.
    pub async fn discover(config: Config, time: Arc<dyn TimeSource>) -> Result<Arc<Self>, Error> {
        // A redirect from the identity provider's back channel is refused,
        // not followed: the BFF talks only to the URLs discovery gave it.
        let http = reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.timeout)
            .timeout(config.timeout)
            .build()
            .map_err(|error| Error::Config(error.to_string()))?;
        let issuer =
            IssuerUrl::new(config.issuer).map_err(|error| Error::Config(error.to_string()))?;
        let origin = config.origin.trim_end_matches('/');
        let redirect = RedirectUrl::new(format!("{origin}/auth/callback"))
            .map_err(|error| Error::Config(error.to_string()))?;
        let post_logout = PostLogoutRedirectUrl::new(format!("{origin}/"))
            .map_err(|error| Error::Config(error.to_string()))?;
        let metadata = ProviderMetadataWithLogout::discover_async(issuer, &http)
            .await
            .map_err(|error| Error::Discovery(error.to_string()))?;
        let end_session = metadata.additional_metadata().end_session_endpoint.clone();
        let secret = ClientSecret::new(config.client_secret);
        let id_tokens = IdTokens {
            issuer: metadata.issuer().clone(),
            secret: secret.clone(),
            algs: metadata.id_token_signing_alg_values_supported().clone(),
            keys_url: metadata.jwks_uri().clone(),
            keys: Mutex::new(Keys {
                set: metadata.jwks().clone(),
                refetched: None,
            }),
        };
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(config.client_id),
            Some(secret),
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
            refreshes: Refreshes::new(),
            id_tokens,
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
    /// A token about to expire is refreshed first, once for all the
    /// requests that find it so at the same time.
    ///
    /// A session with no sign-in, or whose refresh the identity provider
    /// refuses, is ended, and the call gets [`Error::SignedOut`]. An
    /// identity provider that can't be reached, or answers with anything
    /// but a refusal, gets [`Error::Unavailable`], and the session stays.
    pub async fn access_token(self: &Arc<Self>, session: &Session) -> Result<AccessToken, Error> {
        let Some(signed_in) = session::read::<SignedIn>(session, SIGNED_IN).await? else {
            return Err(Error::SignedOut);
        };
        let now = self.time.wall();
        if signed_in.tokens.refresh_at.is_none_or(|at| now < at) {
            return Ok(AccessToken::new(signed_in.tokens.access));
        }
        let Some(refresh) = signed_in.tokens.refresh.clone() else {
            session.flush().await?;
            return Err(Error::SignedOut);
        };
        let bff = Arc::clone(self);
        let spent = refresh.clone();
        let refreshed = self
            .refreshes
            .run(&refresh, now, move || async move {
                let refreshed = bff.refresh(spent, &signed_in).await;
                // Shared until it is due for a refresh of its own.
                let until = refreshed
                    .as_ref()
                    .ok()
                    .and_then(|refreshed| refreshed.tokens.refresh_at);
                (refreshed, until)
            })
            .await
            .unwrap_or_else(|| Err(Failure::Unavailable("the refresh ended".to_owned())));
        match refreshed {
            Ok(refreshed) => {
                session.insert(SIGNED_IN, &refreshed).await?;
                Ok(AccessToken::new(refreshed.tokens.access))
            }
            Err(Failure::SignedOut) => {
                session.flush().await?;
                Err(Error::SignedOut)
            }
            Err(Failure::Unavailable(reason)) => Err(Error::Unavailable(reason)),
        }
    }

    /// Trades `refresh` for new tokens. An ID token that comes back must
    /// name the same person, and a forged one ends the session; one that
    /// fails another check leaves the old ID token in place, since the
    /// identity provider has spent `refresh` and the new tokens are the
    /// only good ones.
    async fn refresh(&self, refresh: String, old: &SignedIn) -> Result<SignedIn, Failure> {
        let response = self
            .client
            .exchange_refresh_token(&RefreshToken::new(refresh.clone()))
            .map_err(|error| Failure::Unavailable(error.to_string()))?
            .request_async(&self.http)
            .await
            .map_err(|error| match error {
                RequestTokenError::ServerResponse(response)
                    if *response.error() == CoreErrorResponseType::InvalidGrant =>
                {
                    Failure::SignedOut
                }
                error => Failure::Unavailable(error.to_string()),
            })?;
        let mut user = old.user.clone();
        let mut id_token = old.tokens.id_token.clone();
        if let Some(token) = response.id_token() {
            // A refresh's ID token carries no nonce of this sign-in's.
            let no_nonce = |_: Option<&Nonce>| Ok(());
            match self.claims(token, response.access_token(), no_nonce).await {
                Ok(claims) if claims.subject().as_str() == old.user.subject => {
                    user = User::from_claims(&claims);
                    id_token = token.to_string();
                }
                Ok(_) | Err(Check::Forged(_)) => return Err(Failure::SignedOut),
                Err(Check::NoKey | Check::Failed(_)) => {}
            }
        }
        Ok(SignedIn {
            user,
            tokens: Tokens {
                access: response.access_token().secret().clone(),
                refresh: Some(
                    response
                        .refresh_token()
                        .map_or(refresh, |refresh| refresh.secret().clone()),
                ),
                refresh_at: self.refresh_at(response.expires_in()),
                id_token,
            },
        })
    }

    /// Checks an ID token and returns its claims: its signature, issuer,
    /// audience and expiry, its nonce by `nonce`, and that `access` is the
    /// access token it was issued with, not another person's swapped in.
    /// A key id the signing keys don't hold fetches them again, at most
    /// once a [`REFETCH`], as the identity provider may have rotated them.
    pub(crate) async fn claims(
        &self,
        id_token: &CoreIdToken,
        access: &AccessToken,
        nonce: impl NonceVerifier + Copy,
    ) -> Result<CoreIdTokenClaims, Check> {
        let keys = self.id_tokens.keys.lock().set.clone();
        match self.check(id_token, access, nonce, keys) {
            Err(Check::NoKey) => match self.refetch_keys().await {
                Some(keys) => self.check(id_token, access, nonce, keys),
                None => Err(Check::NoKey),
            },
            checked => checked,
        }
    }

    /// The signing keys, fetched again, or `None` if they were fetched
    /// less than a [`REFETCH`] ago or can't be.
    async fn refetch_keys(&self) -> Option<CoreJsonWebKeySet> {
        let now = self.time.wall();
        {
            let mut keys = self.id_tokens.keys.lock();
            if keys
                .refetched
                .is_some_and(|at| now < at.saturating_add(nanos(REFETCH)))
            {
                return None;
            }
            keys.refetched = Some(now);
        }
        let set = CoreJsonWebKeySet::fetch_async(&self.id_tokens.keys_url, &self.http)
            .await
            .ok()?;
        self.id_tokens.keys.lock().set = set.clone();
        Some(set)
    }

    fn check(
        &self,
        id_token: &CoreIdToken,
        access: &AccessToken,
        nonce: impl NonceVerifier,
        keys: CoreJsonWebKeySet,
    ) -> Result<CoreIdTokenClaims, Check> {
        let verifier = self.verifier(keys);
        let claims = id_token
            .claims(&verifier, nonce)
            .map_err(|error| match error {
                ClaimsVerificationError::SignatureVerification(
                    SignatureVerificationError::NoMatchingKey,
                ) => Check::NoKey,
                ClaimsVerificationError::SignatureVerification(error) => {
                    Check::Forged(error.to_string())
                }
                error => Check::Failed(error.to_string()),
            })?;
        if let Some(expected) = claims.access_token_hash() {
            let forged = |error: &dyn std::fmt::Display| Check::Forged(error.to_string());
            let alg = id_token.signing_alg().map_err(|error| forged(&error))?;
            let key = id_token
                .signing_key(&verifier)
                .map_err(|error| forged(&error))?;
            let actual =
                AccessTokenHash::from_token(access, alg, key).map_err(|error| forged(&error))?;
            if actual != *expected {
                return Err(forged(&"the access token's hash doesn't match"));
            }
        }
        Ok(claims.clone())
    }

    /// Where a person who isn't signed in is sent.
    pub fn sign_in_page(&self) -> &str {
        &self.sign_in_page
    }

    /// When to refresh a token that lasts `lifetime` from now: [`SKEW`]
    /// before it expires, or halfway through its life if that comes first.
    pub(crate) fn refresh_at(&self, lifetime: Option<Duration>) -> Option<u64> {
        lifetime.map(|lifetime| {
            let early = SKEW.min(lifetime / 2);
            self.time.wall().saturating_add(nanos(lifetime - early))
        })
    }

    /// An ID token verifier for `keys`, on the BFF's clock.
    fn verifier(&self, keys: CoreJsonWebKeySet) -> CoreIdTokenVerifier<'static> {
        let time = Arc::clone(&self.time);
        CoreIdTokenVerifier::new_confidential_client(
            self.client.client_id().clone(),
            self.id_tokens.secret.clone(),
            self.id_tokens.issuer.clone(),
            keys,
        )
        .set_allowed_algs(self.id_tokens.algs.clone())
        .set_time_fn(move || utc(time.wall()))
    }
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn utc(nanos: u64) -> DateTime<Utc> {
    DateTime::from_timestamp_nanos(i64::try_from(nanos).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_gives_its_reason() {
        assert_eq!(
            Check::Forged("bad signature".to_owned()).reason(),
            "bad signature"
        );
        assert_eq!(Check::Failed("expired".to_owned()).reason(), "expired");
        assert_eq!(Check::NoKey.reason(), "no signing key under its key id");
    }
}
