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

static DASHBOARD_CSS: &str = include_str!("../../../../assets/dashboard.css");

/// Shared dashboard stylesheet. Short max-age (vs the tracker's immutable
/// day) so a binary upgrade doesn't leave browsers on stale styles for long.
pub async fn dashboard_css() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=600"),
        ],
        DASHBOARD_CSS,
    )
}
