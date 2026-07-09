//! Response DTOs for the JSON API.
//!
//! Each struct mirrors the exact `serde_json::json!`/`Serialize` shape a
//! handler in `crates/web/src/routes/*.rs` returns (copied from the handler,
//! not guessed). Fields the UI never reads are left off entirely; serde
//! ignores unknown JSON fields by default, so trimming a DTO down to what a
//! page actually uses is safe.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

// ---- Sites: GET/POST /api/v1/sites (analytics::list_sites, sites::create_site_api) ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SiteSummary {
    pub id: String,
    pub domain: String,
    pub name: String,
    /// The site's public tracker key, embedded in the browser snippet's
    /// `data-site`. Present in the list response; defaulted for older shapes.
    #[serde(default)]
    pub public_key: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct SitesList {
    #[serde(default)]
    pub sites: Vec<SiteSummary>,
}

#[derive(Debug, Serialize)]
pub struct CreateSiteBody {
    pub domain: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatedSite {
    pub id: String,
}

// ---- Pageviews + timeseries: GET /api/v1/sites/:site/pageviews (analytics::pageviews) ----
// `comparison` is only present when the request carries `compare=1`; it is
// the same shape one level down (the prior equal-length window's totals).

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TimeBucket {
    pub ts: DateTime<Utc>,
    pub pageviews: u64,
    #[allow(dead_code)]
    pub sessions: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct PageviewsResult {
    #[serde(default)]
    pub buckets: Vec<TimeBucket>,
    pub total_pageviews: u64,
    pub total_sessions: u64,
    pub bounce_rate: f64,
    #[serde(default)]
    pub comparison: Option<Box<PageviewsResult>>,
}

// ---- Top-N breakdowns: GET /api/v1/sites/:site/top-{pages,referrers,countries,
// regions,browsers,os,devices} (analytics::top_list_response) and
// GET /api/v1/sites/:site/events (analytics::events, same TopList shape). ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TopRow {
    pub value: String,
    pub pageviews: u64,
    pub sessions: u64,
    pub pct: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct TopList {
    #[serde(default)]
    pub rows: Vec<TopRow>,
}

// ---- Entry/exit pages: GET /api/v1/sites/:site/top-entry-pages,
// GET /api/v1/sites/:site/top-exit-pages (analytics::top_entry_pages / top_exit_pages) ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EntryPageRow {
    pub url: String,
    pub sessions: u64,
    pub pct: f64,
    pub bounce_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct EntryPages {
    #[serde(default)]
    pub rows: Vec<EntryPageRow>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ExitPageRow {
    pub url: String,
    pub exits: u64,
    pub pct: f64,
    pub exit_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct ExitPages {
    #[serde(default)]
    pub rows: Vec<ExitPageRow>,
}

// ---- Annotations: GET/POST/DELETE /api/v1/sites/:site/annotations[/:id]
// (analytics::list_annotations / create_annotation / delete_annotation) ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AnnotationDto {
    pub id: String,
    pub date: NaiveDate,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct AnnotationsList {
    #[serde(default)]
    pub annotations: Vec<AnnotationDto>,
}

#[derive(Debug, Serialize)]
pub struct CreateAnnotationBody {
    pub date: String,
    pub text: String,
}

// ---- Real-time: GET /api/v1/sites/:site/realtime (analytics::realtime,
// serializes stomatopod_core::query::analytics::RealtimeSnapshot) ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RealtimeTopPage {
    pub url: String,
    pub active_sessions: u64,
    pub pct: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RealtimeEvent {
    pub name: String,
    pub url: String,
    pub seconds_ago: i64,
    #[serde(default)]
    pub properties: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct RealtimeSnapshot {
    pub active_sessions: u64,
    pub pageviews_per_minute: f64,
    #[serde(default)]
    pub top_pages: Vec<RealtimeTopPage>,
    #[serde(default)]
    pub recent_events: Vec<RealtimeEvent>,
}

// ---- Goals: GET/POST /api/v1/sites/:site/goals, GET .../goals/:id/stats
// (analytics::list_goals / create_goal / goal_stats) ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Goal {
    pub id: String,
    pub name: String,
    pub event_name: String,
    /// A serialized filter list, or null when the goal has no filters.
    #[serde(default)]
    pub filters: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct GoalsList {
    #[serde(default)]
    pub goals: Vec<Goal>,
}

#[derive(Debug, Serialize)]
pub struct CreateGoalBody {
    pub name: String,
    pub event_name: String,
}

/// `goal_id` and `name` are injected by the handler on top of the core
/// `GoalStats` shape.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct GoalStats {
    #[serde(default)]
    pub goal_id: String,
    #[serde(default)]
    pub name: String,
    pub completions: u64,
    pub unique_completions: u64,
    pub conversion_rate: f64,
}

// ---- Campaigns: GET /api/v1/sites/:site/campaigns (analytics::campaigns);
// an object keyed by the five UTM `TopListField` tokens. ----

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Campaigns {
    #[serde(default)]
    pub utm_source: TopList,
    #[serde(default)]
    pub utm_medium: TopList,
    #[serde(default)]
    pub utm_campaign: TopList,
    #[serde(default)]
    pub utm_term: TopList,
    #[serde(default)]
    pub utm_content: TopList,
}

// ---- Paths: GET /api/v1/sites/:site/paths (analytics::paths) ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PathRow {
    #[serde(default)]
    pub steps: Vec<String>,
    pub sessions: u64,
    pub pct: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct PathReport {
    #[serde(default)]
    pub rows: Vec<PathRow>,
    pub total_sessions: u64,
}

// ---- Analytics alerts: GET/POST /api/v1/sites/:site/analytics-alerts,
// PATCH/DELETE .../:id (analytics::*, serializes AnalyticsAlert) ----

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct AnalyticsAlertConfig {
    pub threshold: f64,
    #[serde(default)]
    pub window_minutes: u32,
    #[serde(default)]
    pub goal_event_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AnalyticsAlert {
    pub id: String,
    /// One of `traffic_spike`, `traffic_drop`, `goal_threshold`,
    /// `new_referrer_spike`.
    pub kind: String,
    #[serde(default)]
    pub config: AnalyticsAlertConfig,
    pub channel_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct AlertsList {
    #[serde(default)]
    pub alerts: Vec<AnalyticsAlert>,
}

#[derive(Debug, Serialize)]
pub struct CreateAlertBody {
    #[serde(rename = "type")]
    pub alert_type: String,
    pub threshold: f64,
    pub window_minutes: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_event_name: Option<String>,
    pub channel_id: String,
}

#[derive(Debug, Serialize)]
pub struct PatchAlertBody {
    pub enabled: bool,
}

// ---- Alert channels: GET/POST /api/v1/sites/:site/alert-channels,
// DELETE/test .../:id (insights::*_channel_api) ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AlertChannel {
    pub id: String,
    /// One of `webhook`, `slack`, `telegram`.
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub last_error_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct ChannelsList {
    #[serde(default)]
    pub channels: Vec<AlertChannel>,
}

#[derive(Debug, Serialize)]
pub struct CreateChannelBody {
    pub kind: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

/// Result of POST .../alert-channels/:id/test (returned at HTTP 200 even
/// when the delivery itself failed).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChannelTestResult {
    /// `"ok"` or `"fail"`.
    pub result: String,
    #[serde(default)]
    pub error: Option<String>,
}

// ---- API keys: GET/POST /api/v1/keys, GET/POST /api/v1/sites/:site/keys,
// DELETE .../:id (api_keys::*_api) ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ApiKey {
    pub id: String,
    #[serde(default)]
    pub site_id: Option<String>,
    pub name: String,
    /// `"read"` or `"ingest"`.
    pub scope: String,
    pub display_prefix: String,
    #[serde(default)]
    pub last_used_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct KeysList {
    #[serde(default)]
    pub keys: Vec<ApiKey>,
}

/// The create-key response: the same shape as [`ApiKey`] plus the one-time
/// plaintext `secret`, shown once and never stored.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CreatedApiKey {
    pub name: String,
    pub display_prefix: String,
    pub secret: String,
}

/// Body for POST /api/v1/keys (global). `site_id` binds a key to a site.
#[derive(Debug, Serialize)]
pub struct CreateKeyBody {
    pub name: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_id: Option<String>,
}

/// Body for POST /api/v1/sites/:site/keys. `org_wide` makes a read key
/// org-scoped instead of bound to this site.
#[derive(Debug, Serialize)]
pub struct CreateSiteKeyBody {
    pub name: String,
    pub scope: String,
    pub org_wide: bool,
}

// ---- Share links: GET/POST /api/v1/sites/:site/share-links,
// PATCH/DELETE .../:id (share_links::*) ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ShareLink {
    pub id: String,
    pub token: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct ShareLinksList {
    #[serde(default)]
    pub share_links: Vec<ShareLink>,
}

#[derive(Debug, Serialize)]
pub struct CreateShareLinkBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

// ---- Digest subscription: GET/PUT/DELETE
// /api/v1/sites/:site/digest-subscription (digest::*) ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DigestSubscription {
    /// `"weekly"`, `"monthly"`, or `"both"`.
    pub frequency: String,
    pub enabled: bool,
}

/// GET returns `{ "subscription": null }` when none exists.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct DigestSubscriptionResponse {
    #[serde(default)]
    pub subscription: Option<DigestSubscription>,
}

#[derive(Debug, Serialize)]
pub struct PutSubscriptionBody {
    pub frequency: String,
    pub enabled: bool,
}

// ---- Funnels: GET/POST /api/v1/sites/:site/funnels,
// GET .../:funnel_id (analytics::list_funnels / create_funnel / funnel_result) ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Funnel {
    pub id: String,
    pub name: String,
    /// JSON-serialized `Vec<FunnelStep>`; the list page shows only name/count.
    #[serde(default)]
    pub definition: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct FunnelsList {
    #[serde(default)]
    pub funnels: Vec<Funnel>,
}

/// One filter in a funnel step. `field`/`op` are the snake_case tokens the
/// server's `FilterField`/`FilterOp` deserialize from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FilterDraft {
    pub field: String,
    pub op: String,
    pub value: String,
}

/// One step of a funnel definition, POSTed to create a funnel.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FunnelStepDraft {
    pub name: String,
    pub event_name: String,
    pub filters: Vec<FilterDraft>,
}

#[derive(Debug, Serialize)]
pub struct CreateFunnelBody {
    pub name: String,
    pub steps: Vec<FunnelStepDraft>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FunnelStepResult {
    pub name: String,
    pub sessions: u64,
    pub conversion_rate: f64,
    pub drop_off_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct FunnelResult {
    #[serde(default)]
    pub steps: Vec<FunnelStepResult>,
}

// ---- Docs: GET /api/v1/docs (api::docs_api) ----

/// One H2/H3 heading in the docs, for the right-side anchor navigation.
/// Mirrors `api::TocItem`; `slug` matches the `id` stamped on the heading.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DocsTocItem {
    pub level: u8,
    pub text: String,
    pub slug: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct DocsHtml {
    #[serde(default)]
    pub html: String,
    #[serde(default)]
    pub toc: Vec<DocsTocItem>,
}
