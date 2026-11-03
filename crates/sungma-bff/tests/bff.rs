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
    let browser = Browser::new(app(&bff));
    Setup { idp, time, browser }
}

fn app(bff: &Arc<Bff>) -> Router {
    let routes = Router::new()
        .route("/", get(|user: User| async move { user.subject }))
        .route("/me", get(|user: User| async move { format!("{user:?}") }))
        .route("/token", get(token))
        .route("/change", post(|| async { "changed" }));
    bff.wrap(routes, MemoryStore::default())
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
    let login = s.browser.get("/auth/login").await;
    assert_eq!(login.status(), StatusCode::SEE_OTHER);
    let url = Url::parse(&location(&login)).unwrap();
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
    // The sign-in is spent: the real callback is refused too.
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
    let mut thief = Browser::new(app(&Bff::discover(s.idp.config(), s.time.clone())
        .await
        .unwrap()));
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
    s.browser = Browser::new(app(&bff));
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
