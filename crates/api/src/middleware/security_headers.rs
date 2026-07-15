use axum::{
    extract::Request,
    http::{header, HeaderValue},
    middleware::Next,
    response::Response,
};

/// Attach a baseline set of browser security headers to every response.
///
/// CSP is intentionally modest: the dashboard and share pages use inline
/// scripts/styles in a few places, so we avoid a strict script-src that
/// would break the UI. Frame denial covers clickjacking on login and share
/// surfaces; tighten CSP further when those pages drop inline scripts.
pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();

    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    // frame-ancestors reinforces X-Frame-Options for modern browsers.
    headers.insert(
        header::HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static("frame-ancestors 'none'; base-uri 'self'; form-action 'self'"),
    );
    headers.insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );

    resp
}
