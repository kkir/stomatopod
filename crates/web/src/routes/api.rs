use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};

use stomatopod_ingest::handler::{handle_ingest_inner, IngestContext, IngestPayload};

use crate::state::AppState;

/// Ingest endpoint — delegates to the ingest crate using AppState fields.
pub async fn handle_ingest(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<IngestPayload>,
) -> StatusCode {
    let ctx = IngestContext {
        meta: state.meta.clone(),
        tx: state.ingest_tx.clone(),
        geo: state.geo.clone(),
        site_cache: state.site_cache.clone(),
    };
    handle_ingest_inner(&ctx, peer_addr, &headers, payload).await
}

static TRACKER: &str = include_str!("../../../../assets/tracker.js");
/// Compiled marketing CSS (Tailwind output). Embedded at compile time and
/// served from a hash-busted URL by [`crate::routes::marketing`].
pub static MARKETING_CSS: &str = include_str!("../../../../assets/dist/marketing.css");
/// Vendored anime.js bundle, served from a hash-busted URL.
pub static ANIME_JS: &str = include_str!("../../../../assets/vendor/anime.min.js");

// Returning the `&'static str` body directly lets axum wrap the bytes in
// `Bytes::from_static` rather than allocating a fresh `String` per request.

pub async fn tracker_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        TRACKER,
    )
}

pub async fn marketing_css() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        MARKETING_CSS,
    )
}

pub async fn anime_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        ANIME_JS,
    )
}
