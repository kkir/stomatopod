use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::query::{
    events::EventQuery,
    funnel::{FunnelQuery, FunnelStep},
    pageviews::{Granularity, PageviewsQuery, TimeRange, TopListField},
};

use crate::{middleware::auth::Principal, state::AppState};

// ---- Shared helpers ----

fn parse_range(s: &str) -> TimeRange {
    TimeRange::from_label(s)
}

/// Resolve `{site}` path segment: try ULID first, then domain lookup.
async fn resolve_site_id(state: &AppState, site: &str) -> Option<Ulid> {
    if let Ok(id) = Ulid::from_string(site) {
        return Some(id);
    }
    state
        .meta
        .get_site_by_domain(site)
        .await
        .ok()
        .flatten()
        .map(|s| s.id)
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "site not found"})),
    )
        .into_response()
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({"error": "not authorized for this site"})),
    )
        .into_response()
}

/// Resolve `{site}` and enforce that `principal` may read it. Read API keys
/// are confined to their org (and, if site-bound, that single site); session
/// and user principals are unrestricted (matching pre-existing behavior).
async fn resolve_authorized_site(
    state: &AppState,
    principal: &Principal,
    site: &str,
) -> Result<Ulid, Response> {
    let site_id = resolve_site_id(state, site).await.ok_or_else(not_found)?;
    if let Principal::ApiKey {
        org_id,
        site_id: key_site,
    } = principal
    {
        if let Some(ks) = key_site {
            if *ks != site_id {
                return Err(forbidden());
            }
        }
        let site_org = state
            .meta
            .get_site(site_id)
            .await
            .ok()
            .flatten()
            .map(|s| s.org_id);
        if site_org != Some(*org_id) {
            return Err(forbidden());
        }
    }
    Ok(site_id)
}

// ---- Query params ----

#[derive(Deserialize)]
pub struct RangeParams {
    #[serde(default = "default_range")]
    pub range: String,
}

#[derive(Deserialize)]
pub struct PageviewsParams {
    #[serde(default = "default_range")]
    pub range: String,
    #[serde(default)]
    pub granularity: Granularity,
}

#[derive(Deserialize)]
pub struct TopParams {
    #[serde(default = "default_range")]
    pub range: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

#[derive(Deserialize)]
pub struct EventsParams {
    #[serde(default = "default_range")]
    pub range: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
    pub name: Option<String>,
}

fn default_range() -> String {
    "30d".into()
}

fn default_limit() -> u32 {
    20
}

// ---- Response types ----

#[derive(Serialize)]
struct SiteListItem {
    id: String,
    domain: String,
    name: String,
    public_key: String,
    created_at: String,
}

// ---- Handlers ----

/// GET /api/v1/sites  — list sites. For a read API key, scoped to its org
/// (and its single site if site-bound); otherwise the default org.
pub async fn list_sites(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
) -> impl IntoResponse {
    let org_id = match &principal {
        Principal::ApiKey { org_id, .. } => *org_id,
        _ => {
            let orgs = state.meta.list_orgs().await.unwrap_or_default();
            orgs.first().map(|o| o.id).unwrap_or_default()
        }
    };
    let mut sites = state.meta.list_sites(org_id).await.unwrap_or_default();
    // Site-bound read keys see only their site.
    if let Principal::ApiKey {
        site_id: Some(ks), ..
    } = &principal
    {
        sites.retain(|s| s.id == *ks);
    }
    let items: Vec<SiteListItem> = sites
        .into_iter()
        .map(|s| SiteListItem {
            id: s.id.to_string(),
            domain: s.domain,
            name: s.name,
            public_key: s.public_key,
            created_at: s.created_at.to_rfc3339(),
        })
        .collect();
    Json(serde_json::json!({ "sites": items }))
}

/// GET /api/v1/sites/:site/pageviews
pub async fn pageviews(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    Query(params): Query<PageviewsParams>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let q = PageviewsQuery {
        site_id,
        range: parse_range(&params.range),
        granularity: params.granularity,
        filters: vec![],
    };
    match state.backend.query_pageviews(&q).await {
        Ok(result) => Json(serde_json::to_value(result).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn top_list_response(
    state: &AppState,
    principal: &Principal,
    site: &str,
    range_label: &str,
    limit: u32,
    field: TopListField,
) -> axum::response::Response {
    let site_id = match resolve_authorized_site(state, principal, site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let range = parse_range(range_label);
    match state
        .backend
        .query_top_list(site_id, field, &range, limit)
        .await
    {
        Ok(result) => Json(serde_json::to_value(result).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/v1/sites/:site/top-pages
pub async fn top_pages(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    Query(params): Query<TopParams>,
) -> impl IntoResponse {
    top_list_response(
        &state,
        &principal,
        &site,
        &params.range,
        params.limit,
        TopListField::Page,
    )
    .await
}

/// GET /api/v1/sites/:site/top-referrers
pub async fn top_referrers(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    Query(params): Query<TopParams>,
) -> impl IntoResponse {
    top_list_response(
        &state,
        &principal,
        &site,
        &params.range,
        params.limit,
        TopListField::Referrer,
    )
    .await
}

/// GET /api/v1/sites/:site/events
pub async fn events(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    Query(params): Query<EventsParams>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let q = EventQuery {
        site_id,
        range: parse_range(&params.range),
        event_name: params.name,
        filters: vec![],
        limit: params.limit,
    };
    match state.backend.query_custom_events(&q).await {
        Ok(result) => Json(serde_json::to_value(result).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/v1/sites/:site/funnels  — list funnels
pub async fn list_funnels(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match state.meta.list_funnels(site_id).await {
        Ok(funnels) => Json(serde_json::json!({ "funnels": funnels })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/v1/sites/:site/funnels/:funnel_id  — run a funnel query
pub async fn funnel_result(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path((site, funnel_id)): Path<(String, String)>,
    Query(params): Query<RangeParams>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let funnel_ulid = match Ulid::from_string(&funnel_id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid funnel id"})),
            )
                .into_response()
        }
    };
    let funnel_def = match state.meta.get_funnel(funnel_ulid).await {
        Ok(Some(f)) => f,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "funnel not found"})),
            )
                .into_response()
        }
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let steps: Vec<FunnelStep> = match serde_json::from_str(&funnel_def.definition) {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "invalid funnel definition"})),
            )
                .into_response()
        }
    };
    let q = FunnelQuery {
        site_id,
        range: parse_range(&params.range),
        steps,
        window_secs: 86400,
    };
    match state.backend.query_funnel(&q).await {
        Ok(result) => Json(serde_json::to_value(result).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
