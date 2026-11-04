//! Author's pages against the BFF's fake identity provider, through a
//! browser that keeps its cookie.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Response, StatusCode, header},
};
use http_body_util::BodyExt;
use sungma_bff::{
    Bff,
    testkit::{Browser, FakeIdp, SUBJECT, location},
};
use sungma_clock::ManualTime;
use tower_sessions::MemoryStore;

/// 2027-01-15, in nanoseconds since the Unix epoch.
const START: u64 = 1_800_000_000_000_000_000;

/// The fake identity provider, and a browser on Author.
async fn setup() -> (FakeIdp, Browser) {
    let time = Arc::new(ManualTime::new(START));
    let idp = FakeIdp::start(time.clone()).await;
    let bff = Bff::discover(idp.config(), time).await.unwrap();
    let app = sungma_author::app(&bff, "Rauthy".to_owned(), MemoryStore::default());
    (idp, Browser::new(app))
}

async fn text(response: Response<Body>) -> String {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(body.to_vec()).unwrap()
}

fn content_type(response: &Response<Body>) -> &str {
    response.headers()[header::CONTENT_TYPE].to_str().unwrap()
}

#[tokio::test]
async fn home_sends_a_person_who_isnt_signed_in_to_sign_in() {
    let (_idp, mut browser) = setup().await;
    let response = browser.get("/").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&response), "/signin");
}

#[tokio::test]
async fn the_sign_in_page_starts_a_sign_in_at_the_identity_provider() {
    let (_idp, mut browser) = setup().await;
    let response = browser.get("/signin").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(content_type(&response).starts_with("text/html"));
    let page = text(response).await;
    assert!(page.contains(r#"hx-post="/auth/login">Sign in with Rauthy</button>"#));
    assert!(page.contains(r#"<script src="/static/htmx.min.js""#));
    assert!(page.contains(concat!("Author ", env!("CARGO_PKG_VERSION"))));
}

#[tokio::test]
async fn signing_in_lands_on_home() {
    let (_idp, mut browser) = setup().await;
    let callback = browser.sign_in().await;
    assert_eq!(location(&callback), "/");
    let response = browser.get("/").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(content_type(&response).starts_with("text/html"));
    let page = text(response).await;
    assert!(page.contains("<h2>Alice Example</h2>"));
    assert!(page.contains("<summary>alice<svg"));
    assert!(page.contains(&format!(r#"<code title="{SUBJECT}">…alice</code>"#)));
    assert!(page.contains(&format!(r#"data-copy="{SUBJECT}""#)));
}

#[tokio::test]
async fn home_signs_out_at_the_identity_provider() {
    let (idp, mut browser) = setup().await;
    browser.sign_in().await;
    let page = text(browser.get("/").await).await;
    assert!(page.contains(r#"hx-post="/auth/logout">Sign out</button>"#));
    let signed_out = browser.post("/auth/logout", true).await;
    let target = signed_out.headers()["hx-redirect"].to_str().unwrap();
    assert!(target.starts_with(&format!("{}/end_session", idp.issuer())));
    assert_eq!(location(&browser.get("/").await), "/signin");
}

#[tokio::test]
async fn home_escapes_what_the_id_token_holds() {
    let (idp, mut browser) = setup().await;
    idp.behave(|behavior| behavior.subject = r#""><script>x()</script>"#.to_owned());
    browser.sign_in().await;
    let page = text(browser.get("/").await).await;
    assert!(!page.contains("<script>x()"));
    assert!(page.contains(r#"title="&#34;&#62;&#60;script&#62;"#));
}

#[tokio::test]
async fn the_pages_files_are_served_as_what_they_are() {
    let (_idp, mut browser) = setup().await;
    for (path, kind, start) in [
        ("/static/htmx.min.js", "text/javascript", &b"var htmx="[..]),
        ("/static/author.js", "text/javascript", b"// Copies"),
        ("/static/author.css", "text/css", b":root {"),
        ("/static/sungma.png", "image/png", b"\x89PNG"),
    ] {
        let response = browser.get(path).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(content_type(&response), kind, "{path}");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(body.starts_with(start), "{path}");
    }
}
