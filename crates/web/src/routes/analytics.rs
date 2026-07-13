use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use axum_extra::extract::Query as FormQuery;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::{
    domain::{
        analytics_alert::{AnalyticsAlert, AnalyticsAlertConfig, AnalyticsAlertKind},
        org::Funnel,
    },
    query::{
        events::EventQuery,
        funnel::{FunnelQuery, FunnelStep},
        pageviews::{Filter, Granularity, PageviewsQuery, TimeRange, TopListField},
    },
};

use crate::{middleware::auth::Principal, state::AppState};

// ---- Shared helpers ----

fn parse_range(s: &str) -> TimeRange {
    TimeRange::from_label(s)
}

/// Resolve the effective time range from the preset/`from`/`to` triple: a
/// valid `from`+`to` pair wins, otherwise the preset label (default 30d).
fn resolve_range(range: Option<&str>, from: Option<&str>, to: Option<&str>) -> TimeRange {
    if let (Some(f), Some(t)) = (from, to) {
        if let Some(r) = TimeRange::parse_dates(f, t) {
            return r;
        }
    }
    TimeRange::from_label(range.unwrap_or("30d"))
}

/// Parse `filter=field:op:value` params into validated filters, dropping
/// malformed ones and capping the count to bound query cost.
fn parse_filters(raw: &[String]) -> Vec<Filter> {
    raw.iter()
        .filter_map(|s| Filter::parse(s))
        .take(10)
        .collect()
}

fn compare_enabled(v: Option<&str>) -> bool {
    matches!(v, Some("1" | "true" | "on"))
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

/// Unified query params for the analytics read endpoints. Parsed with
/// `axum_extra`'s `Query` so repeated `filter=` keys collect into a `Vec`.
/// Supports preset ranges (`range=30d`), custom windows (`from`/`to`),
/// dimension filters, and the period-comparison toggle.
#[derive(Deserialize, Default, utoipa::IntoParams, utoipa::ToSchema)]
#[into_params(parameter_in = Query)]
pub struct AnalyticsParams {
    /// Preset window: `7d`, `30d` (default), `90d`, `12m`. Overridden when both `from` and `to` are set.
    pub range: Option<String>,
    /// Custom range start (`YYYY-MM-DD`), inclusive. Requires `to`.
    pub from: Option<String>,
    /// Custom range end (`YYYY-MM-DD`), inclusive. Requires `from`.
    pub to: Option<String>,
    /// Period comparison: `1`, `true`, or `on` attaches a prior-window `comparison` object (where supported).
    pub compare: Option<String>,
    /// Bucket size for timeseries (`hour`, `day`, `week`, `month`). Pageviews only.
    #[serde(default)]
    #[param(value_type = String)]
    #[schema(value_type = String)]
    pub granularity: Granularity,
    /// Max rows for top-N endpoints (default 20 for JSON).
    pub limit: Option<u32>,
    /// Repeatable dimension filter as `field:op:value` (e.g. `country:eq:US`).
    /// Fields: url, referrer, country, region, browser, os, device_type, utm_*, event_name.
    /// Ops: eq, not_eq, contains, starts_with. Malformed entries are ignored; max 10.
    #[serde(default)]
    pub filter: Vec<String>,
    /// `csv` triggers a CSV download; anything else (or absent) is JSON.
    pub format: Option<String>,
}

impl AnalyticsParams {
    fn range(&self) -> TimeRange {
        resolve_range(
            self.range.as_deref(),
            self.from.as_deref(),
            self.to.as_deref(),
        )
    }
    fn filters(&self) -> Vec<Filter> {
        parse_filters(&self.filter)
    }
    fn limit_or_default(&self) -> u32 {
        self.limit.unwrap_or(20)
    }
    fn wants_csv(&self) -> bool {
        matches!(self.format.as_deref(), Some("csv"))
    }
}

#[derive(Deserialize, Default, utoipa::IntoParams, utoipa::ToSchema)]
#[into_params(parameter_in = Query)]
pub struct EventsParams {
    /// Preset window; overridden when both `from` and `to` are set.
    pub range: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<u32>,
    /// Filter to a single custom event name.
    pub name: Option<String>,
    /// Repeatable dimension filter (`field:op:value`). `event_name:eq:…`
    /// is treated like `name=` when `name` is absent.
    #[serde(default)]
    pub filter: Vec<String>,
}

fn default_range() -> String {
    "30d".into()
}

// ---- Response / OpenAPI schema types ----

#[derive(Serialize, utoipa::ToSchema)]
pub struct SiteListItem {
    pub id: String,
    pub domain: String,
    pub name: String,
    /// IANA timezone (stored on the site; digests currently run in UTC).
    pub timezone: String,
    pub public_key: String,
    pub created_at: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct SitesResponse {
    pub sites: Vec<SiteListItem>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ErrorBody {
    pub error: String,
}

/// OpenAPI mirror of pageviews JSON (runtime still serializes core types).
#[derive(Serialize, utoipa::ToSchema)]
pub struct PageviewsResultSchema {
    pub buckets: Vec<TimeBucketSchema>,
    pub total_pageviews: u64,
    pub total_sessions: u64,
    pub bounce_rate: f64,
    pub avg_duration_secs: f64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct TimeBucketSchema {
    pub ts: String,
    pub pageviews: u64,
    pub sessions: u64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct TopListSchema {
    pub rows: Vec<TopRowSchema>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct TopRowSchema {
    pub value: String,
    pub pageviews: u64,
    pub sessions: u64,
    pub pct: f64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct FunnelStepSchema {
    pub name: String,
    pub event_name: String,
    /// Property filters; empty array for none. Wire form may vary by client.
    pub filters: Vec<serde_json::Value>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct FunnelSchema {
    pub id: String,
    pub site_id: String,
    pub name: String,
    /// JSON-serialized steps.
    pub definition: String,
    pub created_at: String,
}

// ---- Handlers ----

/// GET /api/v1/sites  — list sites. For a read API key, scoped to its org
/// (and its single site if site-bound); otherwise the default org.
#[utoipa::path(
    get,
    path = "/api/v1/sites",
    tag = "analytics",
    security(("read_key" = [])),
    responses(
        (status = 200, description = "Sites visible to the credential", body = SitesResponse),
        (status = 401, description = "Unauthenticated")
    )
)]
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
            timezone: s.timezone,
            public_key: s.public_key,
            created_at: s.created_at.to_rfc3339(),
        })
        .collect();
    Json(serde_json::json!({ "sites": items }))
}

/// GET /api/v1/sites/:site/pageviews
#[utoipa::path(
    get,
    path = "/api/v1/sites/{site}/pageviews",
    tag = "analytics",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain"),
        AnalyticsParams
    ),
    responses(
        (status = 200, description = "Pageview/session timeseries", body = PageviewsResultSchema),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site", body = ErrorBody)
    )
)]
pub async fn pageviews(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let q = PageviewsQuery {
        site_id,
        range: params.range(),
        granularity: params.granularity,
        filters: params.filters(),
    };
    let result = match state.backend.query_pageviews(&q).await {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    if params.wants_csv() {
        let mut csv = String::from("ts,pageviews,sessions\n");
        for b in &result.buckets {
            csv.push_str(&format!(
                "{},{},{}\n",
                b.ts.to_rfc3339(),
                b.pageviews,
                b.sessions
            ));
        }
        return csv_response("pageviews.csv", csv);
    }

    // Period-over-period comparison: when requested, attach the prior
    // equal-length window's totals under `comparison`.
    if compare_enabled(params.compare.as_deref()) {
        let prev = PageviewsQuery {
            range: q.range.previous(),
            filters: q.filters.clone(),
            ..q
        };
        match state.backend.query_pageviews(&prev).await {
            Ok(cmp) => {
                let mut body = serde_json::to_value(&result).unwrap();
                body["comparison"] = serde_json::to_value(&cmp).unwrap();
                Json(body).into_response()
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    } else {
        Json(serde_json::to_value(result).unwrap()).into_response()
    }
}

async fn top_list_response(
    state: &AppState,
    principal: &Principal,
    site: &str,
    params: &AnalyticsParams,
    field: TopListField,
) -> axum::response::Response {
    let site_id = match resolve_authorized_site(state, principal, site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let range = params.range();
    let filters = params.filters();
    // CSV export has no row cap; the JSON view keeps the dashboard's default.
    let limit = if params.wants_csv() {
        params.limit.unwrap_or(100_000)
    } else {
        params.limit_or_default()
    };
    match state
        .backend
        .query_top_list(site_id, field, &range, limit, &filters)
        .await
    {
        Ok(result) => {
            if params.wants_csv() {
                let mut csv = String::from("value,pageviews,sessions,pct\n");
                for r in &result.rows {
                    csv.push_str(&format!(
                        "{},{},{},{:.2}\n",
                        csv_field(&r.value),
                        r.pageviews,
                        r.sessions,
                        r.pct
                    ));
                }
                csv_response(&format!("{}.csv", field_filename(field)), csv)
            } else {
                // Attach per-row sparklines for the dashboard Trend column.
                // Failure is non-fatal: rows still return without `spark`.
                let spark_by_value: std::collections::HashMap<String, Vec<f64>> = match state
                    .backend
                    .query_top_sparklines(site_id, field, &range, limit, &filters)
                    .await
                {
                    Ok(s) => s
                        .rows
                        .into_iter()
                        .map(|r| (r.value, r.points.into_iter().map(|p| p as f64).collect()))
                        .collect(),
                    Err(_) => std::collections::HashMap::new(),
                };
                let rows: Vec<serde_json::Value> = result
                    .rows
                    .into_iter()
                    .map(|r| {
                        let spark = spark_by_value.get(&r.value).cloned();
                        serde_json::json!({
                            "value": r.value,
                            "pageviews": r.pageviews,
                            "sessions": r.sessions,
                            "pct": r.pct,
                            "spark": spark,
                        })
                    })
                    .collect();
                Json(serde_json::json!({ "rows": rows })).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/v1/sites/:site/top-pages
#[utoipa::path(
    get,
    path = "/api/v1/sites/{site}/top-pages",
    tag = "analytics",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain"),
        AnalyticsParams
    ),
    responses(
        (status = 200, description = "Top pages by traffic", body = TopListSchema),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site", body = ErrorBody)
    )
)]
pub async fn top_pages(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Page).await
}

/// GET /api/v1/sites/:site/top-referrers
#[utoipa::path(
    get,
    path = "/api/v1/sites/{site}/top-referrers",
    tag = "analytics",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain"),
        AnalyticsParams
    ),
    responses(
        (status = 200, description = "Top referrers", body = TopListSchema),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site", body = ErrorBody)
    )
)]
pub async fn top_referrers(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Referrer).await
}

/// GET /api/v1/sites/:site/top-os
#[utoipa::path(
    get,
    path = "/api/v1/sites/{site}/top-os",
    tag = "analytics",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain"),
        AnalyticsParams
    ),
    responses(
        (status = 200, description = "Top operating systems", body = TopListSchema),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site", body = ErrorBody)
    )
)]
pub async fn top_os(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Os).await
}

/// GET /api/v1/sites/:site/top-regions
#[utoipa::path(
    get,
    path = "/api/v1/sites/{site}/top-regions",
    tag = "analytics",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain"),
        AnalyticsParams
    ),
    responses(
        (status = 200, description = "Top regions", body = TopListSchema),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site", body = ErrorBody)
    )
)]
pub async fn top_regions(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Region).await
}

/// GET /api/v1/sites/:site/top-countries
#[utoipa::path(
    get,
    path = "/api/v1/sites/{site}/top-countries",
    tag = "analytics",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain"),
        AnalyticsParams
    ),
    responses(
        (status = 200, description = "Top countries", body = TopListSchema),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site", body = ErrorBody)
    )
)]
pub async fn top_countries(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Country).await
}

/// GET /api/v1/sites/:site/top-browsers
#[utoipa::path(
    get,
    path = "/api/v1/sites/{site}/top-browsers",
    tag = "analytics",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain"),
        AnalyticsParams
    ),
    responses(
        (status = 200, description = "Top browsers", body = TopListSchema),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site", body = ErrorBody)
    )
)]
pub async fn top_browsers(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Browser).await
}

/// GET /api/v1/sites/:site/top-devices
#[utoipa::path(
    get,
    path = "/api/v1/sites/{site}/top-devices",
    tag = "analytics",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain"),
        AnalyticsParams
    ),
    responses(
        (status = 200, description = "Top device types", body = TopListSchema),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site", body = ErrorBody)
    )
)]
pub async fn top_devices(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Device).await
}

/// GET /api/v1/sites/:site/events
#[utoipa::path(
    get,
    path = "/api/v1/sites/{site}/events",
    tag = "analytics",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain"),
        EventsParams
    ),
    responses(
        (status = 200, description = "Custom event breakdown"),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site", body = ErrorBody)
    )
)]
pub async fn events(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<EventsParams>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let mut event_name = params.name.filter(|s| !s.is_empty());
    let mut filters = Vec::new();
    for f in parse_filters(&params.filter) {
        // Click-to-filter on the Events page emits event_name:eq:…; treat it
        // as the dedicated name filter so the breakdown collapses to one row.
        if event_name.is_none()
            && f.field == stomatopod_core::query::pageviews::FilterField::EventName
            && f.op == stomatopod_core::query::pageviews::FilterOp::Eq
        {
            event_name = Some(f.value);
        } else {
            filters.push(f);
        }
    }
    let q = EventQuery {
        site_id,
        range: resolve_range(
            params.range.as_deref(),
            params.from.as_deref(),
            params.to.as_deref(),
        ),
        event_name,
        filters,
        limit: params.limit.unwrap_or(20),
    };
    match state.backend.query_custom_events(&q).await {
        Ok(result) => Json(serde_json::to_value(result).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/v1/sites/:site/funnels  — list funnels
#[utoipa::path(
    get,
    path = "/api/v1/sites/{site}/funnels",
    tag = "funnels",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain")
    ),
    responses(
        (status = 200, description = "Funnels defined for the site"),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site", body = ErrorBody)
    )
)]
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

/// Body for `POST /api/v1/sites/:site/funnels`. Steps are a real JSON array
/// (unlike the dashboard form, which posts them as a string field).
#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateFunnelBody {
    pub name: String,
    /// Ordered steps (≥ 2). Schema uses [`FunnelStepSchema`]; runtime uses core `FunnelStep`.
    #[schema(value_type = Vec<FunnelStepSchema>)]
    pub steps: Vec<FunnelStep>,
}

/// POST /api/v1/sites/:site/funnels  — create a funnel. Read API keys are
/// permitted (same authorization as queries); the key must be in-org and, if
/// site-bound, match the target site.
#[utoipa::path(
    post,
    path = "/api/v1/sites/{site}/funnels",
    tag = "funnels",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain")
    ),
    request_body = CreateFunnelBody,
    responses(
        (status = 201, description = "Funnel created", body = FunnelSchema),
        (status = 400, description = "Invalid body", body = ErrorBody),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site", body = ErrorBody)
    )
)]
pub async fn create_funnel(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    Json(body): Json<CreateFunnelBody>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if body.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "name is required"})),
        )
            .into_response();
    }
    if body.steps.len() < 2 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "a funnel needs at least 2 steps"})),
        )
            .into_response();
    }
    let definition = match serde_json::to_string(&body.steps) {
        Ok(d) => d,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let funnel = Funnel {
        id: Ulid::new(),
        site_id,
        name: body.name,
        definition,
        created_at: chrono::Utc::now(),
    };
    match state.meta.create_funnel(&funnel).await {
        Ok(_) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(&funnel).unwrap()),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// DELETE /api/v1/sites/:site/funnels/:funnel_id  — remove a funnel definition.
/// Same auth as create: dashboard session or a read key in scope for the site.
#[utoipa::path(
    delete,
    path = "/api/v1/sites/{site}/funnels/{funnel_id}",
    tag = "funnels",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain"),
        ("funnel_id" = String, Path, description = "Funnel ULID")
    ),
    responses(
        (status = 204, description = "Funnel deleted"),
        (status = 400, description = "Invalid funnel id", body = ErrorBody),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site or funnel", body = ErrorBody)
    )
)]
pub async fn delete_funnel(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path((site, funnel_id)): Path<(String, String)>,
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
    match state.meta.get_funnel(funnel_ulid).await {
        Ok(Some(f)) if f.site_id == site_id => {}
        Ok(_) => return not_found(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
    match state.meta.delete_funnel(funnel_ulid).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/v1/sites/:site/funnels/:funnel_id  — run a funnel query
#[utoipa::path(
    get,
    path = "/api/v1/sites/{site}/funnels/{funnel_id}",
    tag = "funnels",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain"),
        ("funnel_id" = String, Path, description = "Funnel ULID"),
        ("range" = Option<String>, Query, description = "Preset range: 7d, 30d, 90d, 12m")
    ),
    responses(
        (status = 200, description = "Funnel conversion result"),
        (status = 400, description = "Invalid funnel id", body = ErrorBody),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site or funnel", body = ErrorBody)
    )
)]
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

// ---- CSV helpers ----

/// Quote a CSV field if it contains a comma, quote, or newline (RFC 4180).
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Build a `text/csv` attachment response with the given filename + body.
fn csv_response(filename: &str, body: String) -> Response {
    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "text/csv".to_string()),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        body,
    )
        .into_response()
}

fn field_filename(field: TopListField) -> &'static str {
    match field {
        TopListField::Page => "top-pages",
        TopListField::Referrer => "top-referrers",
        TopListField::Country => "top-countries",
        TopListField::Browser => "top-browsers",
        TopListField::Device => "top-devices",
        TopListField::Os => "top-os",
        TopListField::Region => "top-regions",
        TopListField::UtmSource => "utm-source",
        TopListField::UtmMedium => "utm-medium",
        TopListField::UtmCampaign => "utm-campaign",
        TopListField::UtmTerm => "utm-term",
        TopListField::UtmContent => "utm-content",
    }
}

// ---- Entry / exit pages ----

/// GET /api/v1/sites/:site/top-entry-pages
#[utoipa::path(
    get,
    path = "/api/v1/sites/{site}/top-entry-pages",
    tag = "analytics",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain"),
        AnalyticsParams
    ),
    responses(
        (status = 200, description = "Top entry (landing) pages"),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site", body = ErrorBody)
    )
)]
pub async fn top_entry_pages(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let range = params.range();
    match state
        .backend
        .query_entry_pages(
            site_id,
            &range,
            params.limit_or_default(),
            &params.filters(),
        )
        .await
    {
        Ok(result) => {
            if params.wants_csv() {
                let mut csv = String::from("url,sessions,pct,bounce_rate\n");
                for r in &result.rows {
                    csv.push_str(&format!(
                        "{},{},{:.2},{:.2}\n",
                        csv_field(&r.url),
                        r.sessions,
                        r.pct,
                        r.bounce_rate
                    ));
                }
                csv_response("top-entry-pages.csv", csv)
            } else {
                Json(serde_json::to_value(result).unwrap()).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/v1/sites/:site/top-exit-pages
#[utoipa::path(
    get,
    path = "/api/v1/sites/{site}/top-exit-pages",
    tag = "analytics",
    security(("read_key" = [])),
    params(
        ("site" = String, Path, description = "Site ULID or domain"),
        AnalyticsParams
    ),
    responses(
        (status = 200, description = "Top exit pages"),
        (status = 403, description = "Out of scope", body = ErrorBody),
        (status = 404, description = "Unknown site", body = ErrorBody)
    )
)]
pub async fn top_exit_pages(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let range = params.range();
    match state
        .backend
        .query_exit_pages(
            site_id,
            &range,
            params.limit_or_default(),
            &params.filters(),
        )
        .await
    {
        Ok(result) => {
            if params.wants_csv() {
                let mut csv = String::from("url,exits,pct,exit_rate\n");
                for r in &result.rows {
                    csv.push_str(&format!(
                        "{},{},{:.2},{:.2}\n",
                        csv_field(&r.url),
                        r.exits,
                        r.pct,
                        r.exit_rate
                    ));
                }
                csv_response("top-exit-pages.csv", csv)
            } else {
                Json(serde_json::to_value(result).unwrap()).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---- Analytics alerts ----

#[derive(Deserialize)]
pub struct CreateAlertBody {
    #[serde(rename = "type")]
    pub alert_type: String,
    pub threshold: f64,
    #[serde(default)]
    pub window_minutes: u32,
    /// Ignored. Alerts fan out to every notification channel on the site.
    /// Kept optional for older clients that still send a channel id.
    #[serde(default)]
    #[allow(dead_code)]
    pub channel_id: Option<String>,
}

#[derive(Deserialize)]
pub struct PatchAlertBody {
    pub enabled: bool,
}

/// GET /api/v1/sites/:site/analytics-alerts
pub async fn list_analytics_alerts(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match state.meta.list_analytics_alerts(site_id).await {
        Ok(alerts) => Json(serde_json::json!({ "alerts": alerts })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// POST /api/v1/sites/:site/analytics-alerts
///
/// Dashboard-only: read API keys must not wire alert rules (they can still
/// list alerts and query analytics).
pub async fn create_analytics_alert(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    Json(body): Json<CreateAlertBody>,
) -> impl IntoResponse {
    if !principal.is_dashboard() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "requires a dashboard session, not an API key"
            })),
        )
            .into_response();
    }
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let kind = match AnalyticsAlertKind::from_str(&body.alert_type) {
        Some(k) => k,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "unknown alert type"})),
            )
                .into_response()
        }
    };
    // Alerts notify every destination on the site. Require at least one so
    // create fails early with a clear error rather than silent no-ops.
    let channels = state
        .meta
        .list_alert_channels(site_id)
        .await
        .unwrap_or_default();
    if channels.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "add a notification destination under Settings before creating an alert"
            })),
        )
            .into_response();
    }
    let alert = AnalyticsAlert {
        id: Ulid::new(),
        site_id,
        kind,
        config: AnalyticsAlertConfig {
            threshold: body.threshold,
            window_minutes: if body.window_minutes == 0 {
                60
            } else {
                body.window_minutes
            },
        },
        enabled: true,
        created_at: chrono::Utc::now(),
    };
    match state.meta.create_analytics_alert(&alert).await {
        Ok(_) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(&alert).unwrap()),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// PATCH /api/v1/sites/:site/analytics-alerts/:id  — enable/disable
pub async fn patch_analytics_alert(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path((site, alert_id)): Path<(String, String)>,
    Json(body): Json<PatchAlertBody>,
) -> impl IntoResponse {
    if !principal.is_dashboard() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "requires a dashboard session, not an API key"
            })),
        )
            .into_response();
    }
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let id = match Ulid::from_string(&alert_id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid alert id"})),
            )
                .into_response()
        }
    };
    match state.meta.get_analytics_alert(id).await {
        Ok(Some(a)) if a.site_id == site_id => {}
        Ok(_) => return not_found(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
    match state
        .meta
        .set_analytics_alert_enabled(id, body.enabled)
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// DELETE /api/v1/sites/:site/analytics-alerts/:id
pub async fn delete_analytics_alert(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path((site, alert_id)): Path<(String, String)>,
) -> impl IntoResponse {
    if !principal.is_dashboard() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "requires a dashboard session, not an API key"
            })),
        )
            .into_response();
    }
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let id = match Ulid::from_string(&alert_id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid alert id"})),
            )
                .into_response()
        }
    };
    match state.meta.get_analytics_alert(id).await {
        Ok(Some(a)) if a.site_id == site_id => {}
        Ok(_) => return not_found(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
    match state.meta.delete_analytics_alert(id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---- Raw data export ----

/// Cap on rows returned per export request (spec: 100k).
const EXPORT_MAX_ROWS: u32 = 100_000;

/// GET /api/v1/sites/:site/export/events
pub async fn export_events(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let range = params.range();
    let limit = params.limit.unwrap_or(EXPORT_MAX_ROWS).min(EXPORT_MAX_ROWS);
    let rows = match state
        .backend
        .query_events_list(site_id, &range, limit)
        .await
    {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if params.wants_csv() {
        let mut csv = String::from(
            "id,name,kind,timestamp,url,referrer,country_code,browser,os,device_type,properties\n",
        );
        for r in &rows {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{}\n",
                csv_field(&r.id),
                csv_field(&r.name),
                csv_field(&r.kind),
                r.timestamp.to_rfc3339(),
                csv_field(&r.url),
                csv_field(r.referrer.as_deref().unwrap_or("")),
                csv_field(r.country_code.as_deref().unwrap_or("")),
                csv_field(&r.browser),
                csv_field(&r.os),
                csv_field(&r.device_type),
                csv_field(r.properties.as_deref().unwrap_or("")),
            ));
        }
        csv_response("events.csv", csv)
    } else {
        Json(serde_json::json!({ "rows": rows })).into_response()
    }
}

/// GET /api/v1/sites/:site/export/sessions
pub async fn export_sessions(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let range = params.range();
    let limit = params.limit.unwrap_or(EXPORT_MAX_ROWS).min(EXPORT_MAX_ROWS);
    let rows = match state.backend.query_sessions(site_id, &range, limit).await {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if params.wants_csv() {
        let mut csv = String::from(
            "session_id,started_at,ended_at,duration_secs,pageviews,entry_url,exit_url,\
referrer,country_code,browser,os,device_type,utm_source,utm_medium,utm_campaign,is_bounce\n",
        );
        for r in &rows {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                csv_field(&r.session_id),
                r.started_at.to_rfc3339(),
                r.ended_at.to_rfc3339(),
                r.duration_secs,
                r.pageviews,
                csv_field(&r.entry_url),
                csv_field(&r.exit_url),
                csv_field(r.referrer.as_deref().unwrap_or("")),
                csv_field(r.country_code.as_deref().unwrap_or("")),
                csv_field(&r.browser),
                csv_field(&r.os),
                csv_field(&r.device_type),
                csv_field(r.utm_source.as_deref().unwrap_or("")),
                csv_field(r.utm_medium.as_deref().unwrap_or("")),
                csv_field(r.utm_campaign.as_deref().unwrap_or("")),
                r.is_bounce,
            ));
        }
        csv_response("sessions.csv", csv)
    } else {
        Json(serde_json::json!({ "rows": rows })).into_response()
    }
}

// ---- Tier-3: campaign report, paths ----

/// GET /api/v1/sites/:site/campaigns — UTM breakdowns (source/medium/
/// campaign/term/content) in a single response.
pub async fn campaigns(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let range = params.range();
    let filters = params.filters();
    let limit = params.limit_or_default();
    let mut out = serde_json::Map::new();
    for field in TopListField::UTM {
        match state
            .backend
            .query_top_list(site_id, field, &range, limit, &filters)
            .await
        {
            Ok(tl) => {
                out.insert(field.token().to_string(), serde_json::to_value(tl).unwrap());
            }
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    }
    Json(serde_json::Value::Object(out)).into_response()
}

/// Query params for the single-dimension UTM breakdown used by `spq query utm`.
/// Fields are inlined (not flattened from `AnalyticsParams`) so `axum_extra`
/// Query collection of repeated `filter=` keys works reliably.
#[derive(Deserialize, Default)]
pub struct UtmParams {
    pub range: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<u32>,
    #[serde(default)]
    pub filter: Vec<String>,
    /// One of: `source`, `medium`, `campaign`, `term`, `content`
    /// (or full `utm_*` tokens).
    pub dimension: Option<String>,
    /// Restrict to a single utm_source value.
    pub utm_source: Option<String>,
    /// Restrict to a single utm_medium value.
    pub utm_medium: Option<String>,
}

fn utm_field_from_dimension(dim: &str) -> Option<TopListField> {
    match dim {
        "source" | "utm_source" => Some(TopListField::UtmSource),
        "medium" | "utm_medium" => Some(TopListField::UtmMedium),
        "campaign" | "utm_campaign" => Some(TopListField::UtmCampaign),
        "term" | "utm_term" => Some(TopListField::UtmTerm),
        "content" | "utm_content" => Some(TopListField::UtmContent),
        _ => None,
    }
}

/// GET /api/v1/sites/:site/utm — single UTM dimension top-list.
///
/// Powers `spq query utm`. Prefer `/campaigns` when you want every UTM
/// dimension in one response.
pub async fn utm(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<UtmParams>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let Some(dim) = params.dimension.as_deref().filter(|s| !s.is_empty()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "dimension is required (source|medium|campaign|term|content)"
            })),
        )
            .into_response();
    };
    let Some(field) = utm_field_from_dimension(dim) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "unknown dimension; use source|medium|campaign|term|content"
            })),
        )
            .into_response();
    };
    let mut filters = parse_filters(&params.filter);
    if let Some(src) = params.utm_source.as_deref().filter(|s| !s.is_empty()) {
        filters.push(Filter {
            field: stomatopod_core::query::pageviews::FilterField::UtmSource,
            op: stomatopod_core::query::pageviews::FilterOp::Eq,
            value: src.to_string(),
        });
    }
    if let Some(med) = params.utm_medium.as_deref().filter(|s| !s.is_empty()) {
        filters.push(Filter {
            field: stomatopod_core::query::pageviews::FilterField::UtmMedium,
            op: stomatopod_core::query::pageviews::FilterOp::Eq,
            value: med.to_string(),
        });
    }
    // Cap again after appending the convenience filters.
    filters.truncate(10);
    let range = resolve_range(
        params.range.as_deref(),
        params.from.as_deref(),
        params.to.as_deref(),
    );
    let limit = params.limit.unwrap_or(20);
    match state
        .backend
        .query_top_list(site_id, field, &range, limit, &filters)
        .await
    {
        Ok(tl) => Json(serde_json::to_value(tl).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
