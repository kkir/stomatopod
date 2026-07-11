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
#[derive(Deserialize, Default)]
pub struct AnalyticsParams {
    pub range: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub compare: Option<String>,
    #[serde(default)]
    pub granularity: Granularity,
    pub limit: Option<u32>,
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

#[derive(Deserialize)]
pub struct EventsParams {
    #[serde(default = "default_range")]
    pub range: String,
    pub from: Option<String>,
    pub to: Option<String>,
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
    /// IANA timezone (stored on the site; digests currently run in UTC).
    timezone: String,
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
            timezone: s.timezone,
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
    // CSV export has no row cap; the JSON view keeps the dashboard's default.
    let limit = if params.wants_csv() {
        params.limit.unwrap_or(100_000)
    } else {
        params.limit_or_default()
    };
    match state
        .backend
        .query_top_list(site_id, field, &range, limit, &params.filters())
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
                Json(serde_json::to_value(result).unwrap()).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/v1/sites/:site/top-pages
pub async fn top_pages(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Page).await
}

/// GET /api/v1/sites/:site/top-referrers
pub async fn top_referrers(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Referrer).await
}

/// GET /api/v1/sites/:site/top-os
pub async fn top_os(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Os).await
}

/// GET /api/v1/sites/:site/top-regions
pub async fn top_regions(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Region).await
}

/// GET /api/v1/sites/:site/top-countries
pub async fn top_countries(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Country).await
}

/// GET /api/v1/sites/:site/top-browsers
pub async fn top_browsers(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Browser).await
}

/// GET /api/v1/sites/:site/top-devices
pub async fn top_devices(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    FormQuery(params): FormQuery<AnalyticsParams>,
) -> impl IntoResponse {
    top_list_response(&state, &principal, &site, &params, TopListField::Device).await
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
        range: resolve_range(
            Some(&params.range),
            params.from.as_deref(),
            params.to.as_deref(),
        ),
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

/// Body for `POST /api/v1/sites/:site/funnels`. Steps are a real JSON array
/// (unlike the dashboard form, which posts them as a string field).
#[derive(Deserialize)]
pub struct CreateFunnelBody {
    pub name: String,
    pub steps: Vec<FunnelStep>,
}

/// POST /api/v1/sites/:site/funnels  — create a funnel. Read API keys are
/// permitted (same authorization as queries); the key must be in-org and, if
/// site-bound, match the target site.
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
    pub channel_id: String,
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
    let channel_id = match Ulid::from_string(&body.channel_id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid channel id"})),
            )
                .into_response()
        }
    };
    // The alert channel must belong to the same site.
    let channels = state
        .meta
        .list_alert_channels(site_id)
        .await
        .unwrap_or_default();
    if !channels.iter().any(|c| c.id == channel_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "channel does not belong to this site"})),
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
        channel_id,
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

// ---------------------------------------------------------------------------
