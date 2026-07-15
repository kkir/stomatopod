use axum::http::{header, Method};
use tower_http::cors::{AllowOrigin, CorsLayer};

/// CORS for the ingest endpoint - the tracking script sends beacons from any
/// origin. `navigator.sendBeacon` is hardwired to credentials-mode "include",
/// and browsers reject a wildcard `Access-Control-Allow-Origin: *` (and require
/// `Access-Control-Allow-Credentials: true`) for any credentialed request. So
/// we reflect the request's Origin rather than emitting `*`. tower-http panics
/// if `allow_credentials` is combined with any `Any` wildcard, hence the
/// explicit method/header lists.
pub fn ingest_cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_methods([Method::POST, Method::GET, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE])
        .allow_credentials(true)
}
