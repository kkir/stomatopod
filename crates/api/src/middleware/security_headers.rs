use axum::{
    extract::Request,
    http::{header, HeaderValue},
    middleware::Next,
    response::Response,
};

/// Header name for crawler indexing directives (`X-Robots-Tag`).
pub const X_ROBOTS_TAG: header::HeaderName = header::HeaderName::from_static("x-robots-tag");

/// Appliance-wide crawler directive: never index or follow links.
pub const X_ROBOTS_TAG_VALUE: HeaderValue = HeaderValue::from_static("noindex, nofollow");

/// Attach a baseline set of browser security headers to every response.
///
/// CSP is intentionally modest: the dashboard and share pages use inline
/// scripts/styles in a few places, so we avoid a strict script-src that
/// would break the UI. Frame denial covers clickjacking on login and share
/// surfaces; tighten CSP further when those pages drop inline scripts.
///
/// `X-Robots-Tag: noindex, nofollow` is applied to every appliance response
/// (login, auth redirects, errors, dashboard HTML, JSON) so the private
/// host is not indexed even when a crawler already knows a URL.
pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    apply_security_headers(resp.headers_mut());
    resp
}

/// Write the appliance security + noindex headers onto an existing response.
/// Used by the middleware and by paths that return before that layer runs.
pub fn apply_security_headers(headers: &mut axum::http::HeaderMap) {
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
    headers.insert(X_ROBOTS_TAG, X_ROBOTS_TAG_VALUE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_security_headers_sets_robots_and_baseline() {
        let mut headers = axum::http::HeaderMap::new();
        apply_security_headers(&mut headers);
        assert_eq!(
            headers.get("x-robots-tag").and_then(|v| v.to_str().ok()),
            Some("noindex, nofollow")
        );
        assert_eq!(
            headers
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .and_then(|v| v.to_str().ok()),
            Some("nosniff")
        );
        assert_eq!(
            headers
                .get(header::X_FRAME_OPTIONS)
                .and_then(|v| v.to_str().ok()),
            Some("DENY")
        );
        assert!(headers.get("content-security-policy").is_some());
        assert_eq!(
            headers
                .get(header::REFERRER_POLICY)
                .and_then(|v| v.to_str().ok()),
            Some("strict-origin-when-cross-origin")
        );
        assert_eq!(
            headers
                .get("permissions-policy")
                .and_then(|v| v.to_str().ok()),
            Some("camera=(), microphone=(), geolocation=()")
        );
    }
}
