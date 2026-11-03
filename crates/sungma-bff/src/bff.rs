use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use axum::{
    Extension, Router,
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use openidconnect::{
    AccessToken, AccessTokenHash, AsyncHttpClient, ClaimsVerificationError, ClientId, ClientSecret,
    EndSessionUrl, EndpointMaybeSet, EndpointNotSet, EndpointSet, HttpClientError, HttpRequest,
    HttpResponse, IssuerUrl, JsonWebKeySetUrl, Nonce, NonceVerifier, OAuth2TokenResponse,
    PostLogoutRedirectUrl, ProviderMetadataWithLogout, RedirectUrl, RefreshToken,
    RequestTokenError, Scope, SignatureVerificationError, StandardErrorResponse, TokenResponse,
    core::{
        CoreClient, CoreErrorResponseType, CoreIdToken, CoreIdTokenClaims, CoreIdTokenVerifier,
        CoreJsonWebKeySet, CoreJwsSigningAlgorithm,
    },
    reqwest,
};
use parking_lot::Mutex;
use sungma_clock::TimeSource;
use tower_sessions::{
    Expiry, Session, SessionManagerLayer, SessionStore,
    cookie::{SameSite, time},
};

use crate::{
    Error, User, header, routes,
    session::{self, SIGNED_IN, SignedIn, Tokens},
    shared::Shared,
    signed_out::SignedOut,
    store::Guarded,
};

/// An access token is refreshed this long before it expires, or halfway
/// through its life if that comes later, so it doesn't expire on its way
/// to Sungma.
const SKEW: Duration = Duration::from_secs(60);

/// The signing keys are fetched again, for an ID token they don't verify,
/// at most this often.
const REFETCH: Duration = Duration::from_secs(60);

/// A refresh whose access token has no expiry is shared this long, with
/// requests that read the session before the refresh saved it.
const GRACE: Duration = Duration::from_secs(60);

/// A refresh the identity provider couldn't answer is tried again after
/// this long, not by every request that needs it.
const RETRY: Duration = Duration::from_secs(5);

/// The session cookie's name. The `__Host-` prefix makes the browser
/// refuse the cookie unless it is Secure, for the path `/` and for this
/// host alone, so another host in the domain can't plant a session.
pub(crate) const COOKIE: &str = "__Host-sungma";

/// The OIDC client discovery builds: an authorization endpoint and a
/// token endpoint.
pub(crate) type Client = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointSet,
    EndpointMaybeSet,
>;

/// The error a token request fails with.
type TokenError = RequestTokenError<
    HttpClientError<reqwest::Error>,
    StandardErrorResponse<CoreErrorResponseType>,
>;

/// How the BFF reaches the identity provider and where it sends people.
pub struct Config {
    /// The identity provider's issuer URL, where discovery starts.
    pub issuer: String,
    /// The client the identity provider registered for this BFF.
    ///
    /// The client's ID tokens must carry an `at_hash` made with the hash
    /// of the ID token's own algorithm. Rauthy makes it with the access
    /// token's algorithm instead, so a Rauthy client signs both tokens
    /// with one algorithm, or with two that hash alike.
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
    /// How long a signed-in session lasts without a request.
    pub idle: Duration,
}

/// An OAuth backend-for-frontend for one identity provider.
pub struct Bff {
    pub(crate) client: Client,
    pub(crate) http: reqwest::Client,
    pub(crate) time: Arc<dyn TimeSource>,
    pub(crate) scopes: Vec<Scope>,
    pub(crate) end_session: Option<EndSessionUrl>,
    pub(crate) post_logout: PostLogoutRedirectUrl,
    /// The path of the origin's home, where a sign-in lands.
    pub(crate) home: String,
    sign_in_page: String,
    idle: Duration,
    refreshes: Shared<Result<SignedIn, Failure>>,
    pub(crate) signed_out: Arc<SignedOut>,
    id_tokens: IdTokens,
}

/// What ID tokens are checked against. The signing keys change when the
/// identity provider rotates them.
struct IdTokens {
    issuer: IssuerUrl,
    /// The algorithms the identity provider signs with that use a private
    /// key: an ID token must be signed with a key only the identity
    /// provider holds.
    algs: Vec<CoreJwsSigningAlgorithm>,
    keys_url: JsonWebKeySetUrl,
    keys: Mutex<Arc<CoreJsonWebKeySet>>,
    fetches: Shared<Arc<CoreJsonWebKeySet>>,
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
        let home = post_logout.url().path().to_owned();
        let metadata = ProviderMetadataWithLogout::discover_async(issuer, &http)
            .await
            .map_err(|error| Error::Discovery(error.to_string()))?;
        let end_session = metadata.additional_metadata().end_session_endpoint.clone();
        let token_url = metadata.token_endpoint().cloned().ok_or_else(|| {
            Error::Config("the identity provider names no token endpoint".to_owned())
        })?;
        let algs: Vec<_> = metadata
            .id_token_signing_alg_values_supported()
            .iter()
            .filter(|alg| private_key(alg))
            .cloned()
            .collect();
        if algs.is_empty() {
            return Err(Error::Config(
                "the identity provider signs ID tokens with no private key".to_owned(),
            ));
        }
        let id_tokens = IdTokens {
            issuer: metadata.issuer().clone(),
            algs,
            keys_url: metadata.jwks_uri().clone(),
            keys: Mutex::new(Arc::new(metadata.jwks().clone())),
            fetches: Shared::new(),
        };
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(config.client_id),
            Some(ClientSecret::new(config.client_secret)),
        )
        .set_token_uri(token_url)
        .set_redirect_uri(redirect);
        Ok(Arc::new(Self {
            client,
            http,
            time,
            scopes: config.scopes.into_iter().map(Scope::new).collect(),
            end_session,
            post_logout,
            home,
            sign_in_page: config.sign_in_page,
            idle: config.idle,
            refreshes: Shared::new(),
            signed_out: Arc::new(SignedOut::new(config.timeout)),
            id_tokens,
        }))
    }

    /// Adds the BFF's routes to `app`, and its layers around both: the
    /// session, the BFF itself for the [`User`](crate::User) extractor,
    /// the header check, and the sign-in page for a session that ended.
    /// `store` keeps the sessions.
    ///
    /// The layers cover the routes `app` holds when it is wrapped, and no
    /// route added after, so wrap the app last.
    ///
    /// Run one BFF on a store. A refresh in progress and a sign-out are
    /// known only to the BFF that saw them, so two BFFs on one store spend
    /// a refresh token twice and miss each other's sign-outs. A session
    /// lasts [`Config::idle`] without a request, and a sign-in in progress
    /// ten minutes; a store that doesn't delete expired sessions keeps
    /// them, unread.
    pub fn wrap<S: SessionStore + Clone>(self: &Arc<Self>, app: Router, store: S) -> Router {
        let idle = time::Duration::try_from(self.idle).unwrap_or(time::Duration::MAX);
        let store = Guarded {
            inner: store,
            signed_out: Arc::clone(&self.signed_out),
            time: Arc::clone(&self.time),
        };
        let sessions = SessionManagerLayer::new(store)
            .with_name(COOKIE)
            .with_secure(true)
            .with_http_only(true)
            // Lax, not Strict: the identity provider's redirect to the
            // callback is a cross-site navigation, and it must carry the
            // cookie to find the sign-in it finishes.
            .with_same_site(SameSite::Lax)
            .with_expiry(Expiry::OnInactivity(idle));
        let auth = Router::new()
            .route("/auth/login", post(routes::login))
            .route("/auth/callback", get(routes::callback))
            .route("/auth/logout", post(routes::logout))
            .with_state(Arc::clone(self));
        app.merge(auth)
            .layer(axum::middleware::from_fn(header::require_header))
            .layer(axum::middleware::from_fn_with_state(
                Arc::clone(self),
                session::to_sign_in_when_ended,
            ))
            .layer(Extension(Arc::clone(self)))
            .layer(sessions)
    }

    /// The signed-in person's access token, for a call to Sungma's APIs.
    /// A token about to expire is refreshed first, once for all the
    /// requests that find it so at the same time, and the session saved
    /// at once.
    ///
    /// A session with no sign-in, or whose refresh the identity provider
    /// refuses, is ended, and the call gets [`Error::SignedOut`]. An
    /// identity provider that can't be reached, or answers with anything
    /// but a refusal, leaves the session as it was: the call gets the
    /// access token it holds while that is good, and
    /// [`Error::Unavailable`] after.
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
        let held = signed_in.tokens.clone();
        let bff = Arc::clone(self);
        let saving = session.clone();
        let spent = refresh.clone();
        let refreshed = self
            .refreshes
            .run(&refresh, now, move || async move {
                let refreshed = bff.refresh(spent, &signed_in).await;
                if let Ok(refreshed) = &refreshed {
                    // Saved here, not when the request ends: a request that
                    // fails or goes away would leave the session holding
                    // the spent refresh token. If this save fails, the
                    // request's own save at its end is left.
                    let _ = keep(&saving, refreshed).await;
                }
                // Shared until it is due for a refresh of its own, and a
                // while when it never is. A failure to reach the identity
                // provider is shared a few seconds, and a refusal only
                // with the requests waiting on it.
                let until = match &refreshed {
                    Ok(refreshed) => Some(
                        refreshed
                            .tokens
                            .refresh_at
                            .unwrap_or_else(|| now.saturating_add(nanos(GRACE))),
                    ),
                    Err(Failure::Unavailable(_)) => Some(now.saturating_add(nanos(RETRY))),
                    Err(Failure::SignedOut) => None,
                };
                (refreshed, until)
            })
            .await
            .unwrap_or_else(|| Err(Failure::Unavailable("the refresh ended".to_owned())));
        // The person may have signed out while the refresh was out.
        let id = session.id().map(|id| id.to_string()).unwrap_or_default();
        if self.signed_out.contains(&id, self.time.wall()) {
            return Err(Error::SignedOut);
        }
        match refreshed {
            Ok(refreshed) => {
                session.insert(SIGNED_IN, &refreshed).await?;
                Ok(AccessToken::new(refreshed.tokens.access))
            }
            Err(Failure::SignedOut) => {
                session.flush().await?;
                Err(Error::SignedOut)
            }
            // Refreshed early, the held token may still be good.
            Err(Failure::Unavailable(_))
                if held.expires_at.is_some_and(|at| self.time.wall() < at) =>
            {
                Ok(AccessToken::new(held.access))
            }
            Err(Failure::Unavailable(reason)) => Err(Error::Unavailable(reason)),
        }
    }

    /// Trades `refresh` for new tokens. An ID token that comes back must
    /// name the same person, and a forged one ends the session; one that
    /// fails another check leaves the old ID token in place, since the
    /// identity provider has spent `refresh` and the new tokens are the
    /// only good ones.
    async fn refresh(
        self: &Arc<Self>,
        refresh: String,
        old: &SignedIn,
    ) -> Result<SignedIn, Failure> {
        let response = self
            .client
            .exchange_refresh_token(&RefreshToken::new(refresh.clone()))
            .request_async(&Backchannel(&self.http))
            .await
            .map_err(|error| {
                if refused(&error) {
                    Failure::SignedOut
                } else {
                    Failure::Unavailable(error.to_string())
                }
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
                expires_at: self.expires_at(response.expires_in()),
                id_token,
                generation: old.tokens.generation + 1,
            },
        })
    }

    /// Checks an ID token and returns its claims: its signature, issuer,
    /// audience and expiry, its nonce by `nonce`, and that `access` is the
    /// access token it was issued with, not another person's swapped in.
    /// A token the signing keys don't verify fetches them again, as the
    /// identity provider may have rotated them, under a new key id or the
    /// same one: once for all the requests that need it at the same time,
    /// and at most once a [`REFETCH`].
    pub(crate) async fn claims(
        self: &Arc<Self>,
        id_token: &CoreIdToken,
        access: &AccessToken,
        nonce: impl NonceVerifier + Copy,
    ) -> Result<CoreIdTokenClaims, Check> {
        let keys = Arc::clone(&self.id_tokens.keys.lock());
        match self.check(id_token, access, nonce, &keys) {
            Err(Check::NoKey | Check::Forged(_)) => {
                let keys = self.fetch_keys().await;
                self.check(id_token, access, nonce, &keys)
            }
            checked => checked,
        }
    }

    /// The signing keys, fetched again unless they were less than a
    /// [`REFETCH`] ago, in which case that fetch's keys. A fetch that fails
    /// leaves the keys as they were.
    async fn fetch_keys(self: &Arc<Self>) -> Arc<CoreJsonWebKeySet> {
        let now = self.time.wall();
        let bff = Arc::clone(self);
        self.id_tokens
            .fetches
            .run("keys", now, move || async move {
                let fetched =
                    CoreJsonWebKeySet::fetch_async(&bff.id_tokens.keys_url, &bff.http).await;
                if let Ok(keys) = fetched {
                    *bff.id_tokens.keys.lock() = Arc::new(keys);
                }
                let keys = Arc::clone(&bff.id_tokens.keys.lock());
                (keys, Some(now.saturating_add(nanos(REFETCH))))
            })
            .await
            .unwrap_or_else(|| Arc::clone(&self.id_tokens.keys.lock()))
    }

    fn check(
        &self,
        id_token: &CoreIdToken,
        access: &AccessToken,
        nonce: impl NonceVerifier,
        keys: &CoreJsonWebKeySet,
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
    /// before it expires, or halfway through its life if that comes later.
    pub(crate) fn refresh_at(&self, lifetime: Option<Duration>) -> Option<u64> {
        lifetime.map(|lifetime| {
            let early = SKEW.min(lifetime / 2);
            self.time.wall().saturating_add(nanos(lifetime - early))
        })
    }

    /// When a token that lasts `lifetime` from now expires.
    pub(crate) fn expires_at(&self, lifetime: Option<Duration>) -> Option<u64> {
        lifetime.map(|lifetime| self.time.wall().saturating_add(nanos(lifetime)))
    }

    /// An ID token verifier for `keys`, on the BFF's clock. It holds no
    /// client secret, so it can't check a signature made with one.
    fn verifier(&self, keys: &CoreJsonWebKeySet) -> CoreIdTokenVerifier<'static> {
        let time = Arc::clone(&self.time);
        // The verifier takes the keys by value.
        CoreIdTokenVerifier::new_public_client(
            self.client.client_id().clone(),
            self.id_tokens.issuer.clone(),
            keys.clone(),
        )
        .set_allowed_algs(self.id_tokens.algs.clone())
        .set_time_fn(move || utc(time.wall()))
    }
}

/// Saves `signed_in` in `session` now.
async fn keep(session: &Session, signed_in: &SignedIn) -> Result<(), Error> {
    session.insert(SIGNED_IN, signed_in).await?;
    session.save().await?;
    Ok(())
}

/// The back channel token requests go to the identity provider on. An
/// answer of 5xx or 429 says it can't answer now, whatever its body says,
/// so the call fails rather than read it as a refusal.
pub(crate) struct Backchannel<'a>(pub &'a reqwest::Client);

impl<'c> AsyncHttpClient<'c> for Backchannel<'_> {
    type Error = HttpClientError<reqwest::Error>;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Self::Error>> + Send + 'c>>;

    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(async move {
            let response = self.0.call(request).await?;
            let status = response.status();
            if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
                return Err(HttpClientError::Other(format!(
                    "the identity provider answered {status}"
                )));
            }
            Ok(response)
        })
    }
}

/// Whether the identity provider refused the grant a token request
/// spends: `invalid_grant`, or an error of its own, such as Rauthy's
/// `NotFound` for a spent refresh token. The standard errors besides say
/// the BFF is misconfigured, not that the grant is gone.
pub(crate) fn refused(error: &TokenError) -> bool {
    matches!(
        error,
        RequestTokenError::ServerResponse(response)
            if matches!(
                response.error(),
                CoreErrorResponseType::InvalidGrant | CoreErrorResponseType::Extension(_)
            )
    )
}

/// Whether `alg` signs with a private key: RSA, ECDSA or EdDSA.
fn private_key(alg: &CoreJwsSigningAlgorithm) -> bool {
    matches!(
        alg,
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256
            | CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha384
            | CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha512
            | CoreJwsSigningAlgorithm::RsaSsaPssSha256
            | CoreJwsSigningAlgorithm::RsaSsaPssSha384
            | CoreJwsSigningAlgorithm::RsaSsaPssSha512
            | CoreJwsSigningAlgorithm::EcdsaP256Sha256
            | CoreJwsSigningAlgorithm::EcdsaP384Sha384
            | CoreJwsSigningAlgorithm::EcdsaP521Sha512
            | CoreJwsSigningAlgorithm::EdDsa
    )
}

pub(crate) fn nanos(duration: Duration) -> u64 {
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
