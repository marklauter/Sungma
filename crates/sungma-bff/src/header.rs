use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Whether a request comes from HTMX: it carries `HX-Request: true`.
pub(crate) fn is_htmx(headers: &HeaderMap) -> bool {
    headers
        .get("hx-request")
        .is_some_and(|value| value == "true")
}

/// Refuses a request that changes state unless it comes from HTMX. The
/// browser sends the session cookie with every request, a cross-site one
/// included, but a cross-site form can't add a header.
pub(crate) async fn require_header(request: Request, next: Next) -> Response {
    if request.method().is_safe() || is_htmx(request.headers()) {
        next.run(request).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}
