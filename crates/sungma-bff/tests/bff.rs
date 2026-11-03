//! The BFF against the fake identity provider, through a browser that
//! keeps its cookie.

use std::{sync::Arc, time::Duration};

use axum::{
    Extension, Router,
    body::Body,
    http::{Request, Response, StatusCode},
    routing::{get, post},
};
use http_body_util::BodyExt;
use openidconnect::url::Url;
use sungma_bff::{
    Bff, Error, User,
    testkit::{Browser, FakeIdp, ORIGIN, SUBJECT, location},
};
use sungma_clock::ManualTime;
use tower_sessions::{MemoryStore, Session, SessionManagerLayer};

/// 2027-01-15, in nanoseconds since the Unix epoch.
const START: u64 = 1_800_000_000_000_000_000;

struct Setup {
    idp: FakeIdp,
    time: Arc<ManualTime>,
    store: MemoryStore,
    app: Router,
    browser: Browser,
}

/// The fake identity provider, and a browser on an app with three
/// routes behind the BFF: `/` answers the signed-in person's subject,
/// `/token` their access token, and `POST /change` changes nothing.
async fn setup() -> Setup {
    setup_with(|_| {}).await
}

async fn setup_with(before_discovery: impl FnOnce(&FakeIdp)) -> Setup {
    let time = Arc::new(ManualTime::new(START));
    let idp = FakeIdp::start(time.clone()).await;
    before_discovery(&idp);
    let bff = Bff::discover(idp.config(), time.clone()).await.unwrap();
    let store = MemoryStore::default();
    let app = app(&bff, store.clone());
    let browser = Browser::new(app.clone());
    Setup {
        idp,
        time,
        store,
        app,
        browser,
    }
}

fn app(bff: &Arc<Bff>, store: MemoryStore) -> Router {
    let routes = Router::new()
        .route("/", get(|user: User| async move { user.subject }))
        .route("/me", get(|user: User| async move { format!("{user:?}") }))
        .route("/token", get(token))
        .route("/corrupt", get(corrupt))
        .route("/session", get(peek).put(put))
        .route("/change", post(|| async { "changed" }));
    bff.wrap(routes, store)
}

/// The signed-in person as the session stores them.
async fn peek(session: Session) -> axum::Json<serde_json::Value> {
    let stored = session.get("sungma.bff.signed_in").await.unwrap();
    axum::Json(stored.unwrap_or_default())
}

/// Stores `stored` as the signed-in person, as a request that read the
/// session before another saved it would.
async fn put(session: Session, axum::Json(stored): axum::Json<serde_json::Value>) -> StatusCode {
    session
        .insert("sungma.bff.signed_in", stored)
        .await
        .unwrap();
    StatusCode::OK
}

/// Stores a signed-in person in a shape the BFF doesn't read, as an older
/// or newer release would have.
async fn corrupt(session: Session) -> StatusCode {
    let other = serde_json::json!({ "user": "alice" });
    session.insert("sungma.bff.signed_in", other).await.unwrap();
    StatusCode::OK
}

async fn token(Extension(bff): Extension<Arc<Bff>>, session: Session) -> Result<String, Error> {
    Ok(bff.access_token(&session).await?.secret().clone())
}

async fn text(response: Response<Body>) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn header(response: &Response<Body>, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .map(|value| value.to_str().unwrap().to_owned())
}

/// Replaces one query parameter in a path and query, or removes it.
fn with_param(path: &str, name: &str, value: Option<&str>) -> String {
    let mut url = Url::parse(&format!("{ORIGIN}{path}")).unwrap();
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != name)
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    {
        let mut query = url.query_pairs_mut();
        query.clear();
        for (key, value) in &pairs {
            query.append_pair(key, value);
        }
        if let Some(value) = value {
            query.append_pair(name, value);
        }
    }
    format!("{}?{}", url.path(), url.query().unwrap())
}

async fn signed_in(browser: &mut Browser) -> bool {
    browser.get("/").await.status() == StatusCode::OK
}

#[tokio::test]
async fn a_sign_in_lands_home_with_the_person_signed_in() {
    let mut s = setup().await;
    let callback = s.browser.sign_in().await;
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&callback), "/");
    let home = s.browser.get("/").await;
    assert_eq!(home.status(), StatusCode::OK);
    assert_eq!(text(home).await, SUBJECT);
}

#[tokio::test]
async fn the_user_carries_the_id_token_s_claims() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    let me = text(s.browser.get("/me").await).await;
    let expected = User {
        subject: SUBJECT.to_owned(),
        name: Some("Alice Example".to_owned()),
        username: Some("alice".to_owned()),
        email: Some("alice@example.com".to_owned()),
    };
    assert_eq!(me, format!("{expected:?}"));
}

#[tokio::test]
async fn login_asks_for_a_code_with_pkce_and_the_configured_scopes() {
    let mut s = setup().await;
    let login = s.browser.post("/auth/login", true).await;
    assert_eq!(login.status(), StatusCode::OK);
    let url = Url::parse(&header(&login, "hx-redirect").unwrap()).unwrap();
    assert_eq!(
        format!("{}{}", url.origin().ascii_serialization(), url.path()),
        format!("{}/authorize", s.idp.issuer())
    );
    let param = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    assert_eq!(param("response_type").as_deref(), Some("code"));
    assert_eq!(param("scope").as_deref(), Some("openid profile email"));
    assert_eq!(param("code_challenge_method").as_deref(), Some("S256"));
    assert_eq!(
        param("redirect_uri").as_deref(),
        Some("http://localhost:8090/auth/callback")
    );
}

#[tokio::test]
async fn sign_in_gives_the_session_a_new_id() {
    let mut s = setup().await;
    let callback = s.browser.start_sign_in().await;
    let before = s.browser.cookie().unwrap().to_owned();
    s.browser.get(&callback).await;
    let after = s.browser.cookie().unwrap().to_owned();
    assert_ne!(before, after);
}

#[tokio::test]
async fn a_person_not_signed_in_is_sent_to_the_sign_in_page() {
    let mut s = setup().await;
    let home = s.browser.get("/").await;
    assert_eq!(home.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&home), "/signin");
}

#[tokio::test]
async fn htmx_not_signed_in_is_sent_to_the_sign_in_page_by_hx_redirect() {
    let mut s = setup().await;
    let request = Request::get("/")
        .header("hx-request", "true")
        .body(Body::empty())
        .unwrap();
    let home = s.browser.send(request).await;
    assert_eq!(home.status(), StatusCode::OK);
    assert_eq!(header(&home, "hx-redirect").as_deref(), Some("/signin"));
}

#[tokio::test]
async fn a_tampered_cookie_is_not_signed_in() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    let real = s.browser.cookie().unwrap().to_owned();
    let mut forged = real.clone();
    let last = forged.pop().unwrap();
    forged.push(if last == 'A' { 'B' } else { 'A' });
    s.browser.set_cookie(Some(forged));
    assert!(!signed_in(&mut s.browser).await);
    s.browser.set_cookie(None);
    assert!(!signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn a_callback_without_a_sign_in_is_refused() {
    let mut s = setup().await;
    let callback = s.browser.get("/auth/callback?code=code-0&state=x").await;
    assert_eq!(callback.status(), StatusCode::BAD_REQUEST);
    assert!(!signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn a_callback_with_another_state_is_refused() {
    let mut s = setup().await;
    let callback = s.browser.start_sign_in().await;
    let forged = with_param(&callback, "state", Some("forged"));
    assert_eq!(
        s.browser.get(&forged).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert!(!signed_in(&mut s.browser).await);
    // A forged callback doesn't spend the real sign-in.
    assert_eq!(
        s.browser.get(&callback).await.status(),
        StatusCode::SEE_OTHER
    );
    assert!(signed_in(&mut s.browser).await);
    // A finished sign-in is spent.
    assert_eq!(
        s.browser.get(&callback).await.status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn a_callback_without_a_state_is_refused() {
    let mut s = setup().await;
    let callback = s.browser.start_sign_in().await;
    let bare = with_param(&callback, "state", None);
    assert_eq!(s.browser.get(&bare).await.status(), StatusCode::BAD_REQUEST);
    assert_eq!(s.idp.token_requests(), 0);
}

#[tokio::test]
async fn a_callback_without_a_code_is_refused() {
    let mut s = setup().await;
    let callback = s.browser.start_sign_in().await;
    let bare = with_param(&callback, "code", None);
    assert_eq!(s.browser.get(&bare).await.status(), StatusCode::BAD_REQUEST);
    assert_eq!(s.idp.token_requests(), 0);
}

#[tokio::test]
async fn a_stolen_code_is_refused_by_pkce() {
    let mut s = setup().await;
    let victim = s.browser.start_sign_in().await;
    let code = Url::parse(&format!("{ORIGIN}{victim}"))
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "code")
        .unwrap()
        .1
        .into_owned();
    // The thief starts a sign-in of their own, so the state matches, and
    // swaps in the victim's code; their verifier doesn't match its
    // challenge.
    let mut thief = Browser::new(app(
        &Bff::discover(s.idp.config(), s.time.clone()).await.unwrap(),
        MemoryStore::default(),
    ));
    let own = thief.start_sign_in().await;
    let swapped = with_param(&own, "code", Some(&code));
    assert_eq!(thief.get(&swapped).await.status(), StatusCode::BAD_REQUEST);
    assert!(!signed_in(&mut thief).await);
}

#[tokio::test]
async fn a_declined_sign_in_is_refused() {
    let mut s = setup().await;
    s.idp.behave(|b| b.deny = true);
    assert_eq!(s.browser.sign_in().await.status(), StatusCode::BAD_REQUEST);
    assert!(!signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn a_wrong_client_secret_is_refused() {
    let mut s = setup().await;
    let mut config = s.idp.config();
    config.client_secret = "wrong".to_owned();
    let bff = Bff::discover(config, s.time.clone()).await.unwrap();
    s.browser = Browser::new(app(&bff, MemoryStore::default()));
    assert_eq!(s.browser.sign_in().await.status(), StatusCode::BAD_REQUEST);
    assert!(!signed_in(&mut s.browser).await);
}

/// Signs in with the fake changed by `change`, and says whether the
/// person ended up signed in.
async fn signs_in_when(change: impl FnOnce(&mut sungma_bff::testkit::Behavior)) -> bool {
    let mut s = setup().await;
    s.idp.behave(change);
    let callback = s.browser.sign_in().await;
    let accepted = callback.status() == StatusCode::SEE_OTHER;
    assert_eq!(accepted, signed_in(&mut s.browser).await);
    accepted
}

#[tokio::test]
async fn an_id_token_signed_with_another_key_is_refused() {
    assert!(!signs_in_when(|b| b.wrong_key = true).await);
}

#[tokio::test]
async fn an_expired_id_token_is_refused() {
    assert!(!signs_in_when(|b| b.id_token_lifetime = -1).await);
}

#[tokio::test]
async fn an_id_token_with_another_nonce_is_refused() {
    assert!(!signs_in_when(|b| b.nonce = Some("replayed".to_owned())).await);
}

#[tokio::test]
async fn an_id_token_for_another_client_is_refused() {
    assert!(!signs_in_when(|b| b.audience = Some("someone-else".to_owned())).await);
}

#[tokio::test]
async fn an_access_token_the_id_token_wasn_t_issued_with_is_refused() {
    assert!(!signs_in_when(|b| b.wrong_access_hash = true).await);
}

#[tokio::test]
async fn a_token_response_without_an_id_token_is_refused() {
    assert!(!signs_in_when(|b| b.no_id_token = true).await);
}

#[tokio::test]
async fn an_access_token_is_reused_until_a_minute_before_it_expires() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    let first = text(s.browser.get("/token").await).await;
    // Issued for 300 s: at 239 s it has more than a minute left.
    s.time.advance(Duration::from_secs(239));
    assert_eq!(text(s.browser.get("/token").await).await, first);
    assert_eq!(s.idp.token_requests(), 1);
    // At 240 s it has a minute left, and is refreshed.
    s.time.advance(Duration::from_secs(1));
    let second = text(s.browser.get("/token").await).await;
    assert_ne!(second, first);
    assert_eq!(s.idp.token_requests(), 2);
    // The refreshed token lasts 300 s from its refresh.
    s.time.advance(Duration::from_secs(239));
    assert_eq!(text(s.browser.get("/token").await).await, second);
    assert_eq!(s.idp.token_requests(), 2);
}

#[tokio::test]
async fn a_refresh_uses_the_newest_refresh_token() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    for refreshes in 1..=3 {
        s.time.advance(Duration::from_secs(300));
        let token = s.browser.get("/token").await;
        assert_eq!(token.status(), StatusCode::OK);
        assert_eq!(s.idp.token_requests(), 1 + refreshes);
    }
}

#[tokio::test]
async fn a_refresh_that_returns_no_refresh_token_keeps_the_old_one() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    s.idp.behave(|b| b.refresh_tokens = false);
    s.time.advance(Duration::from_secs(300));
    assert_eq!(s.browser.get("/token").await.status(), StatusCode::OK);
    // The fake spent the old refresh token, so a second refresh with it is
    // refused; what matters is that the BFF tried it.
    s.time.advance(Duration::from_secs(300));
    assert_eq!(
        s.browser.get("/token").await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(s.idp.token_requests(), 3);
}

#[tokio::test]
async fn an_access_token_without_an_expiry_is_never_refreshed() {
    let mut s = setup().await;
    s.idp.behave(|b| b.access_token_lifetime = None);
    s.browser.sign_in().await;
    s.time.advance(Duration::from_secs(86_400));
    assert_eq!(s.browser.get("/token").await.status(), StatusCode::OK);
    assert_eq!(s.idp.token_requests(), 1);
}

#[tokio::test]
async fn a_refused_refresh_signs_the_person_out() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    s.idp.behave(|b| b.refuse_refresh = true);
    s.time.advance(Duration::from_secs(300));
    assert_eq!(
        s.browser.get("/token").await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(!signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn an_expired_access_token_without_a_refresh_token_signs_the_person_out() {
    let mut s = setup().await;
    s.idp.behave(|b| b.refresh_tokens = false);
    s.browser.sign_in().await;
    s.time.advance(Duration::from_secs(300));
    assert_eq!(
        s.browser.get("/token").await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(s.idp.token_requests(), 1);
    assert!(!signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn no_access_token_without_a_sign_in() {
    let mut s = setup().await;
    assert_eq!(
        s.browser.get("/token").await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_change_without_the_htmx_header_is_refused() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    assert_eq!(
        s.browser.post("/change", false).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        s.browser.post("/auth/logout", false).await.status(),
        StatusCode::FORBIDDEN
    );
    assert!(signed_in(&mut s.browser).await);
    let request = Request::post("/change")
        .header("hx-request", "false")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        s.browser.send(request).await.status(),
        StatusCode::FORBIDDEN
    );
    let change = s.browser.post("/change", true).await;
    assert_eq!(change.status(), StatusCode::OK);
    assert_eq!(text(change).await, "changed");
}

#[tokio::test]
async fn sign_out_ends_the_session_at_the_identity_provider() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    let logout = s.browser.post("/auth/logout", true).await;
    assert_eq!(logout.status(), StatusCode::OK);
    let target = Url::parse(&header(&logout, "hx-redirect").unwrap()).unwrap();
    assert_eq!(
        format!("{}{}", target.origin().ascii_serialization(), target.path()),
        format!("{}/end_session", s.idp.issuer())
    );
    let param = |name: &str| {
        target
            .query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    assert_eq!(param("client_id").as_deref(), Some("author"));
    assert_eq!(
        param("post_logout_redirect_uri").as_deref(),
        Some("http://localhost:8090/")
    );
    assert!(param("id_token_hint").is_some());
    assert!(!signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn sign_out_without_a_sign_in_skips_the_hint() {
    let mut s = setup().await;
    let logout = s.browser.post("/auth/logout", true).await;
    let target = Url::parse(&header(&logout, "hx-redirect").unwrap()).unwrap();
    assert!(target.query_pairs().all(|(key, _)| key != "id_token_hint"));
}

#[tokio::test]
async fn sign_out_without_an_end_session_endpoint_goes_home() {
    let mut s = setup_with(|idp| idp.behave(|b| b.end_session = false)).await;
    s.browser.sign_in().await;
    let logout = s.browser.post("/auth/logout", true).await;
    assert_eq!(
        header(&logout, "hx-redirect").as_deref(),
        Some("http://localhost:8090/")
    );
    assert!(!signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn discovery_refuses_a_bad_issuer() {
    let s = setup().await;
    let mut config = s.idp.config();
    config.issuer = "not a url".to_owned();
    let error = Bff::discover(config, s.time.clone()).await.err().unwrap();
    assert!(matches!(error, Error::Config(_)), "{error}");
}

#[tokio::test]
async fn discovery_refuses_a_bad_origin() {
    let s = setup().await;
    let mut config = s.idp.config();
    config.origin = "not a url".to_owned();
    let error = Bff::discover(config, s.time.clone()).await.err().unwrap();
    assert!(matches!(error, Error::Config(_)), "{error}");
}

#[tokio::test]
async fn discovery_refuses_an_issuer_that_names_another() {
    let s = setup().await;
    let mut config = s.idp.config();
    // The document is found under this issuer, but names the fake's.
    config.issuer = format!("{}/", s.idp.issuer());
    let error = Bff::discover(config, s.time.clone()).await.err().unwrap();
    assert!(matches!(error, Error::Discovery(_)), "{error}");
}

#[tokio::test]
async fn the_user_extractor_needs_the_bff() {
    let app = Router::new()
        .route("/", get(|user: User| async move { user.subject }))
        .layer(SessionManagerLayer::new(MemoryStore::default()));
    let mut browser = Browser::new(app);
    assert_eq!(
        browser.get("/").await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn the_bff_names_its_sign_in_page() {
    let s = setup().await;
    let bff = Bff::discover(s.idp.config(), s.time.clone()).await.unwrap();
    assert_eq!(bff.sign_in_page(), "/signin");
}

#[tokio::test]
async fn concurrent_requests_share_one_refresh() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    s.time.advance(Duration::from_secs(300));
    // Three fragments of one page ask for the token at once. The fake
    // spends a refresh token on first use, as Rauthy does.
    let cookie = format!("sungma={}", s.browser.cookie().unwrap());
    let request = || {
        let app = s.app.clone();
        let request = Request::get("/token")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        async move { tower::ServiceExt::oneshot(app, request).await.unwrap() }
    };
    let (a, b, c) = tokio::join!(request(), request(), request());
    assert_eq!([a.status(), b.status(), c.status()], [StatusCode::OK; 3]);
    let (a, b, c) = (text(a).await, text(b).await, text(c).await);
    assert_eq!((&a, &b), (&c, &c));
    assert_eq!(s.idp.token_requests(), 2);
    assert!(signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn an_identity_provider_that_is_down_doesn_t_sign_the_person_out() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    s.idp.behave(|b| b.refresh_server_error = true);
    s.time.advance(Duration::from_secs(300));
    assert_eq!(
        s.browser.get("/token").await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(signed_in(&mut s.browser).await);
    // Back up, the refresh token still works.
    s.idp.behave(|b| b.refresh_server_error = false);
    assert_eq!(s.browser.get("/token").await.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_refresh_that_never_answers_times_out() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    // A second BFF on the same sessions, with a short timeout. The timeout
    // bounds only a call that never answers, so it can't race one that
    // does.
    let mut config = s.idp.config();
    config.timeout = Duration::from_millis(200);
    let impatient = Bff::discover(config, s.time.clone()).await.unwrap();
    let mut browser = Browser::new(app(&impatient, s.store.clone()));
    browser.set_cookie(s.browser.cookie().map(str::to_owned));
    s.idp.behave(|b| b.hang_refresh = true);
    s.time.advance(Duration::from_secs(300));
    // Without a timeout the call waits forever; the guard turns that into
    // a failure.
    let token = tokio::time::timeout(Duration::from_secs(30), browser.get("/token"))
        .await
        .expect("the refresh timed out");
    assert_eq!(token.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn two_sign_ins_in_one_session_both_finish() {
    let mut s = setup().await;
    let first = s.browser.start_sign_in().await;
    let second = s.browser.start_sign_in().await;
    assert_eq!(s.browser.get(&first).await.status(), StatusCode::SEE_OTHER);
    assert_eq!(s.browser.get(&second).await.status(), StatusCode::SEE_OTHER);
    assert!(signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn a_session_in_a_shape_the_bff_no_longer_reads_is_signed_out() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    s.browser.get("/corrupt").await;
    let home = s.browser.get("/").await;
    assert_eq!(home.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&home), "/signin");
    s.browser.sign_in().await;
    s.browser.get("/corrupt").await;
    assert_eq!(
        s.browser.get("/token").await.status(),
        StatusCode::UNAUTHORIZED
    );
    s.browser.sign_in().await;
    s.browser.get("/corrupt").await;
    let logout = s.browser.post("/auth/logout", true).await;
    assert_eq!(logout.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_request_whose_htmx_header_is_false_gets_a_plain_redirect() {
    let mut s = setup().await;
    let request = Request::get("/")
        .header("hx-request", "false")
        .body(Body::empty())
        .unwrap();
    let home = s.browser.send(request).await;
    assert_eq!(home.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&home), "/signin");
}

#[tokio::test]
async fn an_origin_with_a_trailing_slash_makes_the_same_urls() {
    let s = setup().await;
    let mut config = s.idp.config();
    config.origin = format!("{ORIGIN}/");
    let bff = Bff::discover(config, s.time.clone()).await.unwrap();
    let mut browser = Browser::new(app(&bff, MemoryStore::default()));
    let login = browser.post("/auth/login", true).await;
    let login = Url::parse(&header(&login, "hx-redirect").unwrap()).unwrap();
    let redirect = login
        .query_pairs()
        .find(|(key, _)| key == "redirect_uri")
        .unwrap()
        .1
        .into_owned();
    assert_eq!(redirect, "http://localhost:8090/auth/callback");
    let logout = browser.post("/auth/logout", true).await;
    let target = Url::parse(&header(&logout, "hx-redirect").unwrap()).unwrap();
    let back = target
        .query_pairs()
        .find(|(key, _)| key == "post_logout_redirect_uri")
        .unwrap()
        .1
        .into_owned();
    assert_eq!(back, "http://localhost:8090/");
}

#[tokio::test]
async fn a_short_lived_access_token_is_refreshed_halfway_through_its_life() {
    let mut s = setup().await;
    s.idp.behave(|b| b.access_token_lifetime = Some(60));
    s.browser.sign_in().await;
    let first = text(s.browser.get("/token").await).await;
    s.time.advance(Duration::from_secs(29));
    assert_eq!(text(s.browser.get("/token").await).await, first);
    assert_eq!(s.idp.token_requests(), 1);
    s.time.advance(Duration::from_secs(1));
    assert_ne!(text(s.browser.get("/token").await).await, first);
    assert_eq!(s.idp.token_requests(), 2);
}

#[tokio::test]
async fn sign_out_hints_with_the_id_token_from_the_last_refresh() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    s.idp.behave(|b| b.id_token_on_refresh = true);
    s.time.advance(Duration::from_secs(300));
    assert_eq!(s.browser.get("/token").await.status(), StatusCode::OK);
    let logout = s.browser.post("/auth/logout", true).await;
    let target = Url::parse(&header(&logout, "hx-redirect").unwrap()).unwrap();
    let hint = target
        .query_pairs()
        .find(|(key, _)| key == "id_token_hint")
        .unwrap()
        .1
        .into_owned();
    assert_eq!(Some(hint), s.idp.last_id_token());
}

#[tokio::test]
async fn a_refreshed_id_token_for_someone_else_signs_the_person_out() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    s.idp.behave(|b| {
        b.id_token_on_refresh = true;
        b.subject = "mallory".to_owned();
    });
    s.time.advance(Duration::from_secs(300));
    assert_eq!(
        s.browser.get("/token").await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(!signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn a_refreshed_id_token_with_a_bad_signature_signs_the_person_out() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    s.idp.behave(|b| {
        b.id_token_on_refresh = true;
        b.wrong_key = true;
    });
    s.time.advance(Duration::from_secs(300));
    assert_eq!(
        s.browser.get("/token").await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(!signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn a_session_keeps_the_three_newest_sign_ins() {
    let mut s = setup().await;
    let oldest = s.browser.start_sign_in().await;
    let second = s.browser.start_sign_in().await;
    s.browser.start_sign_in().await;
    s.browser.start_sign_in().await;
    assert_eq!(
        s.browser.get(&oldest).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(s.browser.get(&second).await.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn a_refresh_refused_for_the_client_doesn_t_sign_the_person_out() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    // A second BFF on the same sessions, with a secret the identity
    // provider doesn't know: it answers invalid_client, which says the BFF
    // is misconfigured, not that the person's grant is gone.
    let mut config = s.idp.config();
    config.client_secret = "wrong".to_owned();
    let misconfigured = Bff::discover(config, s.time.clone()).await.unwrap();
    let mut browser = Browser::new(app(&misconfigured, s.store.clone()));
    browser.set_cookie(s.browser.cookie().map(str::to_owned));
    s.time.advance(Duration::from_secs(300));
    assert_eq!(
        browser.get("/token").await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(s.browser.get("/token").await.status(), StatusCode::OK);
}

/// A GET of `uri` on `app` with the session cookie `cookie`, as one of
/// several requests a page sends at once.
fn fetch(app: &Router, cookie: &str, uri: &str) -> impl Future<Output = Response<Body>> {
    let app = app.clone();
    let request = Request::get(uri)
        .header("cookie", format!("sungma={cookie}"))
        .body(Body::empty())
        .unwrap();
    async move { tower::ServiceExt::oneshot(app, request).await.unwrap() }
}

/// Yields until `done` holds, failing after many yields rather than
/// waiting forever when it never will.
async fn until(done: impl Fn() -> bool) {
    for _ in 0..100_000 {
        if done() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("never came to pass");
}

#[tokio::test]
async fn a_refresh_token_that_stays_good_is_refreshed_again_when_due() {
    let mut s = setup().await;
    s.idp.behave(|b| {
        b.rotate_refresh = false;
        b.access_token_lifetime = Some(120);
    });
    s.browser.sign_in().await;
    // From now on the fake issues no new refresh token, so the BFF keeps
    // refreshing with the first.
    s.idp.behave(|b| b.refresh_tokens = false);
    let first = text(s.browser.get("/token").await).await;
    s.time.advance(Duration::from_secs(60));
    let second = text(s.browser.get("/token").await).await;
    assert_ne!(second, first);
    assert_eq!(s.idp.token_requests(), 2);
    s.time.advance(Duration::from_secs(60));
    let third = text(s.browser.get("/token").await).await;
    assert_ne!(third, second);
    assert_eq!(s.idp.token_requests(), 3);
}

#[tokio::test]
async fn a_refresh_whose_caller_goes_away_still_lands() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    s.idp.behave(|b| b.hold_refresh = true);
    s.time.advance(Duration::from_secs(300));
    let cookie = s.browser.cookie().unwrap().to_owned();
    {
        // The first request waits until the fake has spent the refresh
        // token, then goes away, as a browser that navigates away does.
        let first = std::pin::pin!(fetch(&s.app, &cookie, "/token"));
        let idp = &s.idp;
        tokio::select! {
            _ = first => panic!("the fake holds the refresh"),
            () = until(|| idp.token_requests() == 2) => {}
        }
    }
    s.idp.behave(|b| b.hold_refresh = false);
    s.idp.release();
    assert_eq!(s.browser.get("/token").await.status(), StatusCode::OK);
    assert_eq!(s.idp.token_requests(), 2);
    assert!(signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn requests_waiting_on_a_failing_refresh_share_its_failure() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    let mut config = s.idp.config();
    config.timeout = Duration::from_millis(200);
    let impatient = Bff::discover(config, s.time.clone()).await.unwrap();
    let app = app(&impatient, s.store.clone());
    s.idp.behave(|b| b.hang_refresh = true);
    s.time.advance(Duration::from_secs(300));
    let cookie = s.browser.cookie().unwrap().to_owned();
    let (a, b, c) = tokio::join!(
        fetch(&app, &cookie, "/token"),
        fetch(&app, &cookie, "/token"),
        fetch(&app, &cookie, "/token"),
    );
    assert_eq!(
        [a.status(), b.status(), c.status()],
        [StatusCode::SERVICE_UNAVAILABLE; 3]
    );
    assert_eq!(s.idp.token_requests(), 2);
    // A failure isn't kept: the next request tries again.
    s.idp.behave(|b| b.hang_refresh = false);
    assert_eq!(
        fetch(&app, &cookie, "/token").await.status(),
        StatusCode::OK
    );
    assert_eq!(s.idp.token_requests(), 3);
}

#[tokio::test]
async fn a_sign_in_after_the_identity_provider_rotates_its_key_finishes() {
    let mut s = setup().await;
    s.idp.rotate_key();
    assert_eq!(s.browser.sign_in().await.status(), StatusCode::SEE_OTHER);
    assert!(signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn a_refresh_after_the_identity_provider_rotates_its_key_checks_the_new_id_token() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    s.idp.rotate_key();
    s.idp.behave(|b| b.id_token_on_refresh = true);
    s.time.advance(Duration::from_secs(300));
    assert_eq!(s.browser.get("/token").await.status(), StatusCode::OK);
    let logout = s.browser.post("/auth/logout", true).await;
    let target = Url::parse(&header(&logout, "hx-redirect").unwrap()).unwrap();
    let hint = target
        .query_pairs()
        .find(|(key, _)| key == "id_token_hint")
        .map(|(_, value)| value.into_owned());
    assert_eq!(hint, s.idp.last_id_token());
}

#[tokio::test]
async fn the_key_set_is_fetched_again_at_most_once_a_minute() {
    let mut s = setup().await;
    s.idp.rotate_key();
    assert_eq!(s.browser.sign_in().await.status(), StatusCode::SEE_OTHER);
    s.idp.rotate_key();
    assert_eq!(s.browser.sign_in().await.status(), StatusCode::BAD_REQUEST);
    s.time.advance(Duration::from_secs(59));
    assert_eq!(s.browser.sign_in().await.status(), StatusCode::BAD_REQUEST);
    s.time.advance(Duration::from_secs(1));
    assert_eq!(s.browser.sign_in().await.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn a_refreshed_id_token_that_fails_a_check_besides_its_signature_keeps_the_new_tokens() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    let first = text(s.browser.get("/token").await).await;
    let signed_in_with = s.idp.last_id_token();
    // The identity provider's clock runs behind, so its ID token looks
    // expired; it has already spent the refresh token.
    s.idp.behave(|b| {
        b.id_token_on_refresh = true;
        b.id_token_lifetime = -1;
    });
    s.time.advance(Duration::from_secs(300));
    let token = s.browser.get("/token").await;
    assert_eq!(token.status(), StatusCode::OK);
    assert_ne!(text(token).await, first);
    assert!(signed_in(&mut s.browser).await);
    let logout = s.browser.post("/auth/logout", true).await;
    let target = Url::parse(&header(&logout, "hx-redirect").unwrap()).unwrap();
    let hint = target
        .query_pairs()
        .find(|(key, _)| key == "id_token_hint")
        .map(|(_, value)| value.into_owned());
    assert_eq!(hint, signed_in_with);
}

#[tokio::test]
async fn a_refresh_that_lands_after_sign_out_doesn_t_sign_the_person_back_in() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    let cookie = s.browser.cookie().unwrap().to_owned();
    s.idp.behave(|b| b.hold_refresh = true);
    s.time.advance(Duration::from_secs(300));
    let refreshing = fetch(&s.app, &cookie, "/token");
    let idp = &s.idp;
    let mut browser = Browser::new(s.app.clone());
    browser.set_cookie(Some(cookie.clone()));
    let (_, ()) = tokio::join!(refreshing, async {
        until(|| idp.token_requests() == 2).await;
        // The person signs out while the refresh is out.
        let logout = browser.post("/auth/logout", true).await;
        assert_eq!(logout.status(), StatusCode::OK);
        idp.release();
    });
    let mut stale = Browser::new(s.app.clone());
    stale.set_cookie(Some(cookie));
    assert!(!signed_in(&mut stale).await);
}

#[tokio::test]
async fn a_refresh_without_an_expiry_is_shared_with_requests_that_read_the_session_before_it() {
    let mut s = setup().await;
    s.browser.sign_in().await;
    let before = text(s.browser.get("/session").await).await;
    s.idp.behave(|b| b.access_token_lifetime = None);
    s.time.advance(Duration::from_secs(300));
    let refreshed = text(s.browser.get("/token").await).await;
    assert_eq!(s.idp.token_requests(), 2);
    // A request that read the session before the refresh saved it still
    // holds the spent refresh token.
    let request = Request::put("/session")
        .header("hx-request", "true")
        .header("content-type", "application/json")
        .body(Body::from(before))
        .unwrap();
    s.browser.send(request).await;
    let token = s.browser.get("/token").await;
    assert_eq!(token.status(), StatusCode::OK);
    assert_eq!(text(token).await, refreshed);
    assert_eq!(s.idp.token_requests(), 2);
}

#[tokio::test]
async fn discovery_refuses_an_identity_provider_without_a_token_endpoint() {
    let s = setup().await;
    s.idp.behave(|b| b.token_endpoint = false);
    let error = Bff::discover(s.idp.config(), s.time.clone()).await.err();
    assert!(matches!(error, Some(Error::Config(_))), "{error:?}");
}

#[tokio::test]
async fn an_identity_provider_that_signs_only_with_a_shared_secret_is_refused() {
    let s = setup().await;
    s.idp.behave(|b| b.hmac = true);
    let error = Bff::discover(s.idp.config(), s.time.clone()).await.err();
    assert!(matches!(error, Some(Error::Config(_))), "{error:?}");
}

#[tokio::test]
async fn an_id_token_signed_with_a_shared_secret_is_refused() {
    let mut s = setup().await;
    s.idp.behave(|b| b.hmac = true);
    assert_eq!(s.browser.sign_in().await.status(), StatusCode::BAD_REQUEST);
    assert!(!signed_in(&mut s.browser).await);
}

#[tokio::test]
async fn sign_in_starts_from_an_htmx_post() {
    let mut s = setup().await;
    assert_eq!(
        s.browser.get("/auth/login").await.status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        s.browser.post("/auth/login", false).await.status(),
        StatusCode::FORBIDDEN
    );
    let login = s.browser.post("/auth/login", true).await;
    assert_eq!(login.status(), StatusCode::OK);
    let authorize = Url::parse(&header(&login, "hx-redirect").unwrap()).unwrap();
    assert_eq!(
        format!(
            "{}{}",
            authorize.origin().ascii_serialization(),
            authorize.path()
        ),
        format!("{}/authorize", s.idp.issuer())
    );
}
