//! A fake identity provider and a browser that keeps a cookie, so the
//! BFF's tests, and its users' tests, sign in without an identity
//! provider running outside them.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::{
    Form, Json, Router,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, Request, Response, StatusCode, header},
    response::{IntoResponse, Redirect},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, TimeDelta};
use openidconnect::{
    AccessToken, Audience, EmptyAdditionalClaims, EndUserEmail, EndUserName, EndUserUsername,
    IssuerUrl, JsonWebKeyId, LocalizedClaim, Nonce, PkceCodeChallenge, PkceCodeVerifier,
    PrivateSigningKey, StandardClaims, SubjectIdentifier,
    core::{
        CoreEdDsaPrivateSigningKey, CoreIdToken, CoreIdTokenClaims, CoreJsonWebKeySet,
        CoreJwsSigningAlgorithm,
    },
    reqwest,
    url::Url,
};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::json;
use sungma_clock::TimeSource;
use tower::ServiceExt;

use crate::Config;

/// The client the fake identity provider knows.
pub const CLIENT_ID: &str = "author";
pub const CLIENT_SECRET: &str = "author-secret";

/// The origin the fake sends the browser back to.
pub const ORIGIN: &str = "http://localhost:8090";

/// The person the fake signs in, unless a test says otherwise.
pub const SUBJECT: &str = "3f6c1a52-alice";

/// How the fake identity provider behaves. A test changes it between
/// steps with [`FakeIdp::behave`].
#[derive(Clone, Debug)]
pub struct Behavior {
    /// Whom the fake signs in.
    pub subject: String,
    /// Refuse the sign-in at the authorize endpoint, as a person who
    /// declines would.
    pub deny: bool,
    /// Sign ID tokens with a key the JWKS doesn't hold, under the same
    /// key id.
    pub wrong_key: bool,
    /// Put this nonce in ID tokens instead of the one asked for.
    pub nonce: Option<String>,
    /// Issue ID tokens for this audience instead of the client.
    pub audience: Option<String>,
    /// How long an ID token lasts, in seconds; negative issues one already
    /// expired.
    pub id_token_lifetime: i64,
    /// How long an access token lasts, in seconds, or `None` to leave
    /// `expires_in` out.
    pub access_token_lifetime: Option<u64>,
    /// Issue refresh tokens.
    pub refresh_tokens: bool,
    /// Refuse every refresh.
    pub refuse_refresh: bool,
    /// Put another access token's hash in ID tokens.
    pub wrong_access_hash: bool,
    /// Leave the ID token out of the token response.
    pub no_id_token: bool,
    /// Name an end-session endpoint in discovery.
    pub end_session: bool,
    /// Answer every refresh with a 500 and no OAuth error, as a provider
    /// that's down does.
    pub refresh_server_error: bool,
    /// Never answer a refresh.
    pub hang_refresh: bool,
    /// Issue a new ID token with each refresh.
    pub id_token_on_refresh: bool,
    /// Spend a refresh token on use, as Rauthy does. Off, a refresh token
    /// stays good, and the fake issues no new one unless `refresh_tokens`
    /// is set.
    pub rotate_refresh: bool,
    /// Spend the refresh token, then hold the answer until the test calls
    /// [`FakeIdp::release`].
    pub hold_refresh: bool,
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            subject: SUBJECT.to_owned(),
            deny: false,
            wrong_key: false,
            nonce: None,
            audience: None,
            id_token_lifetime: 300,
            access_token_lifetime: Some(300),
            refresh_tokens: true,
            refuse_refresh: false,
            wrong_access_hash: false,
            no_id_token: false,
            end_session: true,
            refresh_server_error: false,
            hang_refresh: false,
            id_token_on_refresh: false,
            rotate_refresh: true,
            hold_refresh: false,
        }
    }
}

/// A sign-in the authorize endpoint granted: what the token endpoint
/// checks the code's exchange against.
struct Grant {
    challenge: String,
    nonce: String,
    redirect_uri: String,
}

struct Fake {
    issuer: String,
    keys: Mutex<Keys>,
    release: tokio::sync::Notify,
    time: Arc<dyn TimeSource>,
    behavior: Mutex<Behavior>,
    grants: Mutex<HashMap<String, Grant>>,
    refresh_tokens: Mutex<HashSet<String>>,
    issued: AtomicUsize,
    token_requests: AtomicUsize,
    last_id_token: Mutex<Option<String>>,
}

/// A fake identity provider on a random local port, serving discovery,
/// the signing keys, authorize, token and end session. Authorize grants
/// at once, with no sign-in page.
pub struct FakeIdp {
    fake: Arc<Fake>,
}

impl FakeIdp {
    /// Starts the fake on the current Tokio runtime. Its ID tokens are
    /// issued and expire on `time`.
    pub async fn start(time: Arc<dyn TimeSource>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a local port");
        let issuer = format!("http://{}", listener.local_addr().expect("an address"));
        let fake = Arc::new(Fake {
            issuer,
            keys: Mutex::new(Keys::new(0)),
            release: tokio::sync::Notify::new(),
            time,
            behavior: Mutex::new(Behavior::default()),
            grants: Mutex::new(HashMap::new()),
            refresh_tokens: Mutex::new(HashSet::new()),
            issued: AtomicUsize::new(0),
            token_requests: AtomicUsize::new(0),
            last_id_token: Mutex::new(None),
        });
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .route("/end_session", get(|| async { "Signed out." }))
            .with_state(Arc::clone(&fake));
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self { fake }
    }

    /// The fake's issuer URL.
    pub fn issuer(&self) -> &str {
        &self.fake.issuer
    }

    /// A BFF configuration for the fake's client.
    pub fn config(&self) -> Config {
        Config {
            issuer: self.fake.issuer.clone(),
            client_id: CLIENT_ID.to_owned(),
            client_secret: CLIENT_SECRET.to_owned(),
            origin: ORIGIN.to_owned(),
            scopes: vec!["profile".to_owned(), "email".to_owned()],
            sign_in_page: "/signin".to_owned(),
            timeout: std::time::Duration::from_secs(30),
        }
    }

    /// Changes how the fake behaves from now on.
    pub fn behave(&self, change: impl FnOnce(&mut Behavior)) {
        change(&mut self.fake.behavior.lock());
    }

    /// The last ID token the fake issued.
    pub fn last_id_token(&self) -> Option<String> {
        self.fake.last_id_token.lock().clone()
    }

    /// Rolls to a new signing key under a new key id. The key set serves
    /// only the new key from now on.
    pub fn rotate_key(&self) {
        let mut keys = self.fake.keys.lock();
        *keys = Keys::new(keys.generation + 1);
    }

    /// Answers the refresh held by [`Behavior::hold_refresh`].
    pub fn release(&self) {
        self.fake.release.notify_one();
    }

    /// How many requests the token endpoint has had.
    pub fn token_requests(&self) -> usize {
        self.fake.token_requests.load(Ordering::SeqCst)
    }
}

/// The fake's signing key, and a key the key set doesn't hold under the
/// same key id.
struct Keys {
    generation: u32,
    current: CoreEdDsaPrivateSigningKey,
    wrong: CoreEdDsaPrivateSigningKey,
}

impl Keys {
    fn new(generation: u32) -> Self {
        let kid = format!("test-{generation}");
        Self {
            generation,
            current: signing_key(&kid),
            wrong: signing_key(&kid),
        }
    }
}

/// A fresh Ed25519 key under the key id `kid`.
fn signing_key(kid: &str) -> CoreEdDsaPrivateSigningKey {
    let pem = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .expect("an Ed25519 key")
        .serialize_pem();
    CoreEdDsaPrivateSigningKey::from_ed25519_pem(&pem, Some(JsonWebKeyId::new(kid.to_owned())))
        .expect("a PKCS #8 key")
}

async fn discovery(State(fake): State<Arc<Fake>>) -> Json<serde_json::Value> {
    let issuer = &fake.issuer;
    let mut metadata = json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "jwks_uri": format!("{issuer}/jwks"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["EdDSA"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic"],
        "code_challenge_methods_supported": ["S256"],
    });
    if fake.behavior.lock().end_session {
        metadata["end_session_endpoint"] = json!(format!("{issuer}/end_session"));
    }
    Json(metadata)
}

async fn jwks(State(fake): State<Arc<Fake>>) -> Json<CoreJsonWebKeySet> {
    let key = fake.keys.lock().current.as_verification_key();
    Json(CoreJsonWebKeySet::new(vec![key]))
}

#[derive(Deserialize)]
struct Authorize {
    client_id: String,
    redirect_uri: String,
    state: String,
    nonce: String,
    code_challenge: String,
    code_challenge_method: String,
}

async fn authorize(
    State(fake): State<Arc<Fake>>,
    Query(request): Query<Authorize>,
) -> Response<Body> {
    if request.client_id != CLIENT_ID || request.code_challenge_method != "S256" {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mut back = Url::parse(&request.redirect_uri).expect("a redirect URI");
    if fake.behavior.lock().deny {
        back.query_pairs_mut()
            .append_pair("error", "access_denied")
            .append_pair("state", &request.state);
        return Redirect::to(back.as_str()).into_response();
    }
    let code = format!("code-{}", fake.issued.fetch_add(1, Ordering::SeqCst));
    fake.grants.lock().insert(
        code.clone(),
        Grant {
            challenge: request.code_challenge,
            nonce: request.nonce,
            redirect_uri: request.redirect_uri,
        },
    );
    back.query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &request.state);
    Redirect::to(back.as_str()).into_response()
}

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: String,
    code: Option<String>,
    code_verifier: Option<String>,
    redirect_uri: Option<String>,
    refresh_token: Option<String>,
}

async fn token(
    State(fake): State<Arc<Fake>>,
    headers: HeaderMap,
    Form(request): Form<TokenRequest>,
) -> Response<Body> {
    fake.token_requests.fetch_add(1, Ordering::SeqCst);
    let expected = format!(
        "Basic {}",
        STANDARD.encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"))
    );
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        != Some(&expected)
    {
        return refuse("invalid_client");
    }
    match request.grant_type.as_str() {
        "authorization_code" => exchange_code(&fake, request),
        "refresh_token" => {
            let behavior = fake.behavior.lock().clone();
            if behavior.hang_refresh {
                std::future::pending::<()>().await;
            }
            if behavior.refresh_server_error {
                return (StatusCode::INTERNAL_SERVER_ERROR, "down").into_response();
            }
            let answer = exchange_refresh(&fake, request);
            if behavior.hold_refresh {
                fake.release.notified().await;
            }
            answer
        }
        _ => refuse("unsupported_grant_type"),
    }
}

fn exchange_code(fake: &Fake, request: TokenRequest) -> Response<Body> {
    // A code is good once.
    let Some(grant) = request
        .code
        .as_ref()
        .and_then(|code| fake.grants.lock().remove(code))
    else {
        return refuse("invalid_grant");
    };
    let verifier = PkceCodeVerifier::new(request.code_verifier.unwrap_or_default());
    let challenge = PkceCodeChallenge::from_code_verifier_sha256(&verifier);
    if challenge.as_str() != grant.challenge || request.redirect_uri != Some(grant.redirect_uri) {
        return refuse("invalid_grant");
    }
    let behavior = fake.behavior.lock().clone();
    let access = fake.access_token();
    let mut body = tokens(fake, &behavior, &access);
    if !behavior.no_id_token {
        body["id_token"] = json!(id_token(fake, &behavior, Some(grant.nonce), &access));
    }
    Json(body).into_response()
}

fn exchange_refresh(fake: &Fake, request: TokenRequest) -> Response<Body> {
    let behavior = fake.behavior.lock().clone();
    let known = request.refresh_token.is_some_and(|refresh| {
        let mut issued = fake.refresh_tokens.lock();
        if behavior.rotate_refresh {
            issued.remove(&refresh)
        } else {
            issued.contains(&refresh)
        }
    });
    if behavior.refuse_refresh || !known {
        return refuse("invalid_grant");
    }
    let access = fake.access_token();
    let mut body = tokens(fake, &behavior, &access);
    if behavior.id_token_on_refresh {
        body["id_token"] = json!(id_token(fake, &behavior, None, &access));
    }
    Json(body).into_response()
}

impl Fake {
    fn access_token(&self) -> AccessToken {
        AccessToken::new(format!(
            "access-{}",
            self.issued.fetch_add(1, Ordering::SeqCst)
        ))
    }
}

/// A token response without its ID token.
fn tokens(fake: &Fake, behavior: &Behavior, access: &AccessToken) -> serde_json::Value {
    let mut body = json!({ "access_token": access.secret(), "token_type": "Bearer" });
    if let Some(lifetime) = behavior.access_token_lifetime {
        body["expires_in"] = json!(lifetime);
    }
    if behavior.refresh_tokens {
        let refresh = format!("refresh-{}", fake.issued.fetch_add(1, Ordering::SeqCst));
        fake.refresh_tokens.lock().insert(refresh.clone());
        body["refresh_token"] = json!(refresh);
    }
    body
}

/// A signed ID token, which the fake also keeps as its last. A refresh
/// issues one without a nonce.
fn id_token(
    fake: &Fake,
    behavior: &Behavior,
    nonce: Option<String>,
    access: &AccessToken,
) -> String {
    let now = DateTime::from_timestamp_nanos(i64::try_from(fake.time.wall()).unwrap_or(i64::MAX));
    let audience = behavior
        .audience
        .clone()
        .unwrap_or_else(|| CLIENT_ID.to_owned());
    let mut name = LocalizedClaim::new();
    name.insert(None, EndUserName::new("Alice Example".to_owned()));
    let standard = StandardClaims::new(SubjectIdentifier::new(behavior.subject.clone()))
        .set_name(Some(name))
        .set_preferred_username(Some(EndUserUsername::new("alice".to_owned())))
        .set_email(Some(EndUserEmail::new("alice@example.com".to_owned())));
    let claims = CoreIdTokenClaims::new(
        IssuerUrl::new(fake.issuer.clone()).expect("an issuer"),
        vec![Audience::new(audience)],
        now + TimeDelta::seconds(behavior.id_token_lifetime),
        now,
        standard,
        EmptyAdditionalClaims {},
    )
    .set_nonce(behavior.nonce.clone().or(nonce).map(Nonce::new));
    let keys = fake.keys.lock();
    let key = if behavior.wrong_key {
        &keys.wrong
    } else {
        &keys.current
    };
    let hashed = if behavior.wrong_access_hash {
        AccessToken::new("someone-else".to_owned())
    } else {
        access.clone()
    };
    let token = CoreIdToken::new(
        claims,
        key,
        CoreJwsSigningAlgorithm::EdDsa,
        Some(&hashed),
        None,
    )
    .expect("a signed ID token")
    .to_string();
    *fake.last_id_token.lock() = Some(token.clone());
    token
}

fn refuse(error: &str) -> Response<Body> {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response()
}

/// A browser on an app: it keeps the session cookie the app sets and
/// sends it back, and follows the fake identity provider's redirects.
pub struct Browser {
    app: Router,
    cookie: Option<String>,
    http: reqwest::Client,
}

impl Browser {
    pub fn new(app: Router) -> Self {
        let http = reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("an HTTP client");
        Self {
            app,
            cookie: None,
            http,
        }
    }

    /// The session cookie's value, if the browser holds one.
    pub fn cookie(&self) -> Option<&str> {
        self.cookie.as_deref()
    }

    /// Replaces the session cookie, as a tampering browser would.
    pub fn set_cookie(&mut self, cookie: Option<String>) {
        self.cookie = cookie;
    }

    /// Sends `request` to the app with the session cookie, and keeps the
    /// cookie the response sets.
    pub async fn send(&mut self, mut request: Request<Body>) -> Response<Body> {
        if let Some(cookie) = &self.cookie {
            let value = format!("sungma={cookie}").parse().expect("a cookie header");
            request.headers_mut().insert(header::COOKIE, value);
        }
        let response = self.app.clone().oneshot(request).await.expect("infallible");
        if let Some(set) = response.headers().get(header::SET_COOKIE) {
            let set = set.to_str().expect("an ASCII cookie");
            let pair = set.split(';').next().unwrap_or_default();
            let value = pair.strip_prefix("sungma=").unwrap_or_default();
            let removed = value.is_empty() || set.contains("Max-Age=0");
            self.cookie = (!removed).then(|| value.to_owned());
        }
        response
    }

    /// GETs `uri` from the app.
    pub async fn get(&mut self, uri: &str) -> Response<Body> {
        self.send(Request::get(uri).body(Body::empty()).expect("a request"))
            .await
    }

    /// POSTs to `uri` on the app, as HTMX does when `htmx` is set.
    pub async fn post(&mut self, uri: &str, htmx: bool) -> Response<Body> {
        let mut request = Request::post(uri);
        if htmx {
            request = request.header("hx-request", "true");
        }
        self.send(request.body(Body::empty()).expect("a request"))
            .await
    }

    /// Starts a sign-in at the app and follows it to the identity
    /// provider, returning the callback path and query the identity
    /// provider sends the browser back to.
    pub async fn start_sign_in(&mut self) -> String {
        let login = self.get("/auth/login").await;
        let authorize = location(&login);
        let back = self
            .http
            .get(authorize)
            .send()
            .await
            .expect("the fake answers");
        let back = back
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .expect("a redirect back");
        let back = Url::parse(back).expect("an absolute URL");
        format!("{}?{}", back.path(), back.query().unwrap_or_default())
    }

    /// Signs in through the identity provider, returning the callback's
    /// response.
    pub async fn sign_in(&mut self) -> Response<Body> {
        let callback = self.start_sign_in().await;
        self.get(&callback).await
    }
}

/// A response's `Location` header.
pub fn location(response: &Response<Body>) -> String {
    response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("a Location header")
        .to_owned()
}
