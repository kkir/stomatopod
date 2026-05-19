use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};

use stomatopod_ingest::span_handler::{handle_span_ingest, SpanIngestContext, SpanIngestPayload};

use crate::state::AppState;

/// `POST /api/v1/spans` — Sentinel sidecar span ingest.
///
/// Authentication: `Authorization: Bearer <token>` against the
/// `sentinel_tokens` table. Distinct from the analytics ingest path
/// (which uses a public site key in the body).
pub async fn handle_span_ingest_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<SpanIngestPayload>,
) -> StatusCode {
    let Some(bearer) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    else {
        return StatusCode::UNAUTHORIZED;
    };

    let ctx = SpanIngestContext {
        meta: state.meta.clone(),
        tx: state.span_ingest_tx.clone(),
        token_cache: state.sentinel_token_cache.clone(),
        redact_keys: state.redact_keys.clone(),
    };
    handle_span_ingest(&ctx, bearer, payload).await
}
