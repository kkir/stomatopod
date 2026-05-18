use axum::http::Method;
use tower_http::cors::{Any, CorsLayer};

/// Permissive CORS for the ingest endpoint — the tracking script sends
/// beacons from any origin.
pub fn ingest_cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::POST, Method::GET, Method::OPTIONS])
        .allow_headers(Any)
}
