use axum::{
    Router,
    http::header::CONTENT_TYPE,
    response::{IntoResponse, Response},
    routing::get,
};

/// HTMX, as published, unchanged.
const HTMX: &str = include_str!("../static/htmx.min.js");
const SCRIPT: &str = include_str!("../static/author.js");
const STYLE: &str = include_str!("../static/author.css");
const LOGO: &[u8] = include_bytes!("../../../assets/logo/sungma.png");

/// The files the pages link to, under `/static`.
pub(crate) fn router() -> Router {
    Router::new()
        .route("/static/htmx.min.js", get(|| file("text/javascript", HTMX)))
        .route("/static/author.js", get(|| file("text/javascript", SCRIPT)))
        .route("/static/author.css", get(|| file("text/css", STYLE)))
        .route("/static/sungma.png", get(|| file("image/png", LOGO)))
}

async fn file(content_type: &'static str, body: impl IntoResponse) -> Response {
    ([(CONTENT_TYPE, content_type)], body).into_response()
}
