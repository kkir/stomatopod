use async_trait::async_trait;
use ulid::Ulid;

use crate::{
    domain::{
        agent::{Agent, AlertChannel, SentinelToken},
        agent_span::AgentSpan,
        analytics_alert::{AnalyticsAlert, AnalyticsAlertFire},
        annotation::Annotation,
        api_key::ApiKey,
        goal::Goal,
        incident::{Incident, IncidentStatus},
        org::{Funnel, Organization, User},
        policy::Policy,
        site::Site,
    },
    error::StoreError,
    query::{
        analytics::{
            EntryPages, ExitPages, GoalQuery, GoalStats, PathReport, RawEventRow, RealtimeSnapshot,
            RetentionGrid, SessionRow, TopSparklines,
        },
        events::EventQuery,
        funnel::{FunnelQuery, FunnelResult},
        pageviews::{Filter, PageviewsQuery, PageviewsResult, TimeRange, TopList, TopListField},
        spans::{AgentSummary, SpanQuery, SpanRow},
        tier4::EventPropRow,
    },
};

use crate::domain::event::Event;

/// Primary analytics write + query interface.
///
/// All web/ingest code holds `Arc<dyn StorageBackend>` — concrete backend
/// types are never imported outside `crates/store` and `bin/stomatopod`.
#[async_trait]
pub trait StorageBackend: Send + Sync + 'static {
    // ---- Write path ----

    /// Enqueue a batch of events. Returns only after durability is guaranteed
    /// (WAL flush for embedded; network ack for postgres/clickhouse).
    async fn ingest_events(&self, events: Vec<Event>) -> Result<(), StoreError>;

    // ---- Analytics queries ----

    async fn query_pageviews(&self, q: &PageviewsQuery) -> Result<PageviewsResult, StoreError>;

    /// Group pageviews by a single dimension (URL, referrer, country, browser,
    /// device type, OS, or region) and return the top `limit` values ordered
    /// by traffic. `filters` further narrows the rows (ANDed together).
    async fn query_top_list(
        &self,
        site_id: Ulid,
        field: TopListField,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<TopList, StoreError>;

    async fn query_custom_events(&self, q: &EventQuery) -> Result<TopList, StoreError>;

    async fn query_funnel(&self, q: &FunnelQuery) -> Result<FunnelResult, StoreError>;

    // ---- Tier-2 analytics queries ----

    /// Top entry pages (where sessions begin) over `range`, with bounce rate.
    async fn query_entry_pages(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<EntryPages, StoreError>;

    /// Top exit pages (where sessions end) over `range`, with exit rate.
    async fn query_exit_pages(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<ExitPages, StoreError>;

    /// Live snapshot of the last `window_minutes` of activity.
    async fn query_realtime(
        &self,
        site_id: Ulid,
        window_minutes: u32,
    ) -> Result<RealtimeSnapshot, StoreError>;

    /// Goal completions + conversion-rate timeseries.
    async fn query_goal(&self, q: &GoalQuery) -> Result<GoalStats, StoreError>;

    /// Derived session rows for export (newest first).
    async fn query_sessions(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<Vec<SessionRow>, StoreError>;

    /// Raw event rows for export (newest first).
    async fn query_events_list(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<Vec<RawEventRow>, StoreError>;

    // ---- Tier-3 analytics queries ----

    /// Per-value daily mini timeseries for the top `limit` values of a
    /// dimension over `range`. Powers the sparklines next to top-N rows.
    async fn query_top_sparklines(
        &self,
        site_id: Ulid,
        field: TopListField,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<TopSparklines, StoreError>;

    /// Weekly retention cohort grid over `range`.
    async fn query_retention(
        &self,
        site_id: Ulid,
        range: &TimeRange,
    ) -> Result<RetentionGrid, StoreError>;

    /// Top page-navigation sequences (first `depth` pageviews per session).
    async fn query_paths(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        depth: u32,
        limit: u32,
    ) -> Result<PathReport, StoreError>;

    // ---- Tier-4 analytics: raw custom-event rows ----

    /// Fetch raw custom-event rows (name, url, session, timestamp, key
    /// dimensions, and the JSON property bag) for in-process Tier-4
    /// aggregation (Core Web Vitals, scroll, revenue, A/B, heatmaps, search).
    ///
    /// `names` restricts to those event names; an empty slice returns every
    /// custom event. `limit` caps the row count. The default returns an empty
    /// vector so backends can opt in incrementally.
    async fn query_event_props(
        &self,
        _site_id: Ulid,
        _names: &[String],
        _range: &TimeRange,
        _limit: u32,
    ) -> Result<Vec<EventPropRow>, StoreError> {
        Ok(Vec::new())
    }
}

/// Metadata CRUD: sites, orgs, users, funnels.
///
/// Embedded: backed by SQLite. SaaS: backed by a Postgres connection pool.
#[async_trait]
pub trait MetaStore: Send + Sync + 'static {
    // ---- Sites ----
    async fn create_site(&self, site: &Site) -> Result<(), StoreError>;
    async fn update_site(&self, site: &Site) -> Result<(), StoreError>;
    async fn get_site(&self, id: Ulid) -> Result<Option<Site>, StoreError>;
    async fn get_site_by_key(&self, public_key: &str) -> Result<Option<Site>, StoreError>;
    async fn get_site_by_domain(&self, domain: &str) -> Result<Option<Site>, StoreError>;
    async fn list_sites(&self, org_id: Ulid) -> Result<Vec<Site>, StoreError>;
    async fn delete_site(&self, id: Ulid) -> Result<(), StoreError>;

    // ---- Organizations ----
    async fn create_org(&self, org: &Organization) -> Result<(), StoreError>;
    async fn get_org(&self, id: Ulid) -> Result<Option<Organization>, StoreError>;
    async fn list_orgs(&self) -> Result<Vec<Organization>, StoreError>;

    // ---- Users ----
    async fn create_user(&self, user: &User) -> Result<(), StoreError>;
    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, StoreError>;

    // ---- Funnels ----
    async fn create_funnel(&self, funnel: &Funnel) -> Result<(), StoreError>;
    async fn get_funnel(&self, id: Ulid) -> Result<Option<Funnel>, StoreError>;
    async fn list_funnels(&self, site_id: Ulid) -> Result<Vec<Funnel>, StoreError>;
    async fn delete_funnel(&self, id: Ulid) -> Result<(), StoreError>;

    // ---- Goals ----
    async fn create_goal(&self, goal: &Goal) -> Result<(), StoreError>;
    async fn get_goal(&self, id: Ulid) -> Result<Option<Goal>, StoreError>;
    async fn list_goals(&self, site_id: Ulid) -> Result<Vec<Goal>, StoreError>;
    async fn delete_goal(&self, id: Ulid) -> Result<(), StoreError>;

    // ---- Annotations ----
    async fn create_annotation(&self, annotation: &Annotation) -> Result<(), StoreError>;
    async fn get_annotation(&self, id: Ulid) -> Result<Option<Annotation>, StoreError>;
    /// Annotations whose date falls within `[start, end]` (inclusive),
    /// newest-dated first.
    async fn list_annotations(
        &self,
        site_id: Ulid,
        start: chrono::NaiveDate,
        end: chrono::NaiveDate,
    ) -> Result<Vec<Annotation>, StoreError>;
    async fn delete_annotation(&self, id: Ulid) -> Result<(), StoreError>;

    // ---- Analytics alerts ----
    async fn create_analytics_alert(&self, alert: &AnalyticsAlert) -> Result<(), StoreError>;
    async fn get_analytics_alert(&self, id: Ulid) -> Result<Option<AnalyticsAlert>, StoreError>;
    async fn list_analytics_alerts(&self, site_id: Ulid)
        -> Result<Vec<AnalyticsAlert>, StoreError>;
    /// All enabled alerts across every site — used by the evaluator loop.
    async fn list_enabled_analytics_alerts(&self) -> Result<Vec<AnalyticsAlert>, StoreError>;
    async fn set_analytics_alert_enabled(&self, id: Ulid, enabled: bool) -> Result<(), StoreError>;
    async fn delete_analytics_alert(&self, id: Ulid) -> Result<(), StoreError>;
    async fn record_analytics_alert_fire(
        &self,
        fire: &AnalyticsAlertFire,
    ) -> Result<(), StoreError>;
    /// Most recent fire time for an alert, for cooldown enforcement.
    async fn last_analytics_alert_fire(
        &self,
        alert_id: Ulid,
    ) -> Result<Option<AnalyticsAlertFire>, StoreError>;

    // ---- Agents ----
    async fn upsert_agent(&self, agent: &Agent) -> Result<(), StoreError>;
    async fn list_agents(&self, site_id: Ulid) -> Result<Vec<Agent>, StoreError>;
    async fn get_agent(&self, site_id: Ulid, agent_id: &str) -> Result<Option<Agent>, StoreError>;

    // ---- Sentinel tokens ----
    async fn create_sentinel_token(&self, token: &SentinelToken) -> Result<(), StoreError>;
    async fn list_sentinel_tokens(&self, site_id: Ulid) -> Result<Vec<SentinelToken>, StoreError>;
    async fn get_sentinel_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<SentinelToken>, StoreError>;
    async fn touch_sentinel_token(&self, id: Ulid) -> Result<(), StoreError>;
    async fn delete_sentinel_token(&self, id: Ulid) -> Result<(), StoreError>;

    // ---- API keys ----
    async fn create_api_key(&self, key: &ApiKey) -> Result<(), StoreError>;
    async fn list_api_keys(&self, org_id: Ulid) -> Result<Vec<ApiKey>, StoreError>;
    async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, StoreError>;
    async fn touch_api_key(&self, id: Ulid) -> Result<(), StoreError>;
    async fn delete_api_key(&self, id: Ulid) -> Result<(), StoreError>;

    // ---- Alert channels ----
    async fn create_alert_channel(&self, channel: &AlertChannel) -> Result<(), StoreError>;
    async fn list_alert_channels(&self, site_id: Ulid) -> Result<Vec<AlertChannel>, StoreError>;
    async fn delete_alert_channel(&self, id: Ulid) -> Result<(), StoreError>;

    // ---- Policies ----
    async fn upsert_policy(&self, policy: &Policy) -> Result<(), StoreError>;
    async fn get_policy(&self, site_id: Ulid) -> Result<Option<Policy>, StoreError>;

    // ---- Incidents ----
    async fn record_incident(&self, incident: &Incident) -> Result<(), StoreError>;
    async fn list_incidents(&self, site_id: Ulid, limit: u32) -> Result<Vec<Incident>, StoreError>;
    async fn update_incident_status(
        &self,
        id: Ulid,
        status: IncidentStatus,
    ) -> Result<(), StoreError>;
}

/// Span ingest + query for the AI firewall product surface.
///
/// Deliberately separate from `StorageBackend` so other backends
/// (postgres, clickhouse) can opt in without implementing stubs, and so
/// analytics-only deployments aren't forced to carry agent-observability
/// machinery.
#[async_trait]
pub trait AgentStore: Send + Sync + 'static {
    /// Enqueue a batch of spans. Returns only after durability is
    /// guaranteed (WAL flush for embedded).
    async fn ingest_spans(&self, spans: Vec<AgentSpan>) -> Result<(), StoreError>;

    /// Recent spans for a single agent (or all agents in a site if
    /// `agent_id` is None), ordered by `started_at` descending.
    async fn query_spans(&self, q: &SpanQuery) -> Result<Vec<SpanRow>, StoreError>;

    /// Per-agent summary cards for the dashboard.
    async fn summarize_agents(
        &self,
        site_id: Ulid,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<AgentSummary>, StoreError>;

    /// Running session cost — used by the server-side cost-threshold
    /// detector and by the dashboard's cost meter.
    async fn session_cost_usd(
        &self,
        site_id: Ulid,
        agent_session_id: &str,
    ) -> Result<f64, StoreError>;
}
