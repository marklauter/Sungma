use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Refuses a request that changes state unless it carries
/// `HX-Request: true`. The browser sends the session cookie with every
/// request, a cross-site one included, but a cross-site form can't add a
/// header.
pub(crate) async fn require_header(request: Request, next: Next) -> Response {
    let htmx = request
        .headers()
        .get("hx-request")
        .is_some_and(|value| value == "true");
    if request.method().is_safe() || htmx {
        next.run(request).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}
