use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};

use stomatopod_core::domain::api_key::{ApiKey, ApiKeyScope};
use stomatopod_ingest::handler::{
    handle_ingest_inner, handle_server_ingest, IngestContext, IngestPayload, ServerEventPayload,
};
use tracing::warn;
use ulid::Ulid;

use crate::state::{ApiKeyCacheEntry, AppState};

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

/// `POST /api/v1/ingest` — server-side custom event ingest.
///
/// Authentication: `Authorization: Bearer <ingest_key>` against the
/// `api_keys` table (scope = ingest). Distinct from `/api/v1/event`, which
/// uses a public site key in the body for browser beacons.
pub async fn handle_key_ingest(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ServerEventPayload>,
) -> StatusCode {
    let Some(bearer) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    else {
        return StatusCode::UNAUTHORIZED;
    };

    let key_hash = ApiKey::hash(bearer);
    let entry = match resolve_api_key(&state, &key_hash).await {
        Ok(Some(e)) => e,
        Ok(None) => return StatusCode::UNAUTHORIZED,
        Err(e) => {
            warn!("api key lookup error: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    // Ingest keys must be ingest-scoped and bound to a site.
    let site_id = match (entry.scope, entry.site_id) {
        (ApiKeyScope::Ingest, Some(site_id)) => site_id,
        _ => return StatusCode::UNAUTHORIZED,
    };

    touch_api_key(&state, entry.key_id);

    handle_server_ingest(&state.ingest_tx, site_id, payload).await
}

/// Resolve a key hash via the in-memory cache, falling back to the store
/// and populating the cache on a miss. Shared by the ingest and read paths.
pub async fn resolve_api_key(
    state: &Arc<AppState>,
    key_hash: &str,
) -> Result<Option<ApiKeyCacheEntry>, stomatopod_core::error::StoreError> {
    if let Some(e) = state.api_key_cache.get(key_hash) {
        return Ok(Some(*e));
    }
    match state.meta.get_api_key_by_hash(key_hash).await? {
        Some(key) => {
            let entry = ApiKeyCacheEntry {
                org_id: key.org_id,
                site_id: key.site_id,
                key_id: key.id,
                scope: key.scope,
            };
            state.api_key_cache.insert(key_hash.to_string(), entry);
            Ok(Some(entry))
        }
        None => Ok(None),
    }
}

/// Fire-and-forget `last_used_at` update, including on cache hits.
pub fn touch_api_key(state: &Arc<AppState>, key_id: Ulid) {
    let meta = state.meta.clone();
    tokio::spawn(async move {
        if let Err(e) = meta.touch_api_key(key_id).await {
            warn!("touch_api_key failed: {e}");
        }
    });
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
