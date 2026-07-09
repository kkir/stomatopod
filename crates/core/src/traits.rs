use async_trait::async_trait;
use ulid::Ulid;

use crate::{
    domain::{
        agent::AlertChannel,
        analytics_alert::{AnalyticsAlert, AnalyticsAlertFire},
        annotation::Annotation,
        api_key::ApiKey,
        digest::DigestSubscription,
        goal::Goal,
        org::{Funnel, Organization, User},
        share_link::ShareLink,
        site::Site,
    },
    error::StoreError,
    query::{
        analytics::{
            EntryPages, ExitPages, GoalQuery, GoalStats, PathReport, RawEventRow, RealtimeSnapshot,
            SessionRow, TopSparklines,
        },
        events::EventQuery,
        funnel::{FunnelQuery, FunnelResult},
        pageviews::{Filter, PageviewsQuery, PageviewsResult, TimeRange, TopList, TopListField},
    },
};

use crate::domain::event::Event;

/// Primary analytics write + query interface.
///
/// All web/ingest code holds `Arc<dyn StorageBackend>` — concrete backend
/// types are never imported outside `crates/store` and the server binary.
#[async_trait]
pub trait StorageBackend: Send + Sync + 'static {
    // ---- Write path ----

    /// Enqueue a batch of events. Returns only after durability is guaranteed
    /// (WAL flush for embedded; network ack for postgres).
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

    /// Top page-navigation sequences (first `depth` pageviews per session).
    async fn query_paths(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        depth: u32,
        limit: u32,
    ) -> Result<PathReport, StoreError>;
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
    async fn get_user(&self, id: Ulid) -> Result<Option<User>, StoreError>;

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

    // ---- Share links ----
    async fn create_share_link(&self, link: &ShareLink) -> Result<(), StoreError>;
    async fn list_share_links(&self, site_id: Ulid) -> Result<Vec<ShareLink>, StoreError>;
    async fn get_share_link(&self, id: Ulid) -> Result<Option<ShareLink>, StoreError>;
    /// Resolve a public token to its link. Powers unauthenticated `/share`
    /// access; a missing row means a revoked or never-issued token.
    async fn get_share_link_by_token(&self, token: &str) -> Result<Option<ShareLink>, StoreError>;
    /// Update the mutable fields (label, expiry) of a link.
    async fn update_share_link(
        &self,
        id: Ulid,
        label: Option<String>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), StoreError>;
    async fn delete_share_link(&self, id: Ulid) -> Result<(), StoreError>;

    // ---- Email digest subscriptions ----
    async fn upsert_digest_subscription(&self, sub: &DigestSubscription) -> Result<(), StoreError>;
    async fn get_digest_subscription(
        &self,
        user_id: Ulid,
        site_id: Ulid,
    ) -> Result<Option<DigestSubscription>, StoreError>;
    async fn delete_digest_subscription(
        &self,
        user_id: Ulid,
        site_id: Ulid,
    ) -> Result<(), StoreError>;
    /// All enabled subscriptions across every site — used by the digest
    /// scheduler loop.
    async fn list_enabled_digest_subscriptions(
        &self,
    ) -> Result<Vec<DigestSubscription>, StoreError>;
    /// Increment the consecutive-bounce counter, disabling the subscription
    /// once it reaches `disable_at`.
    async fn record_digest_bounce(&self, id: Ulid, disable_at: u32) -> Result<(), StoreError>;

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
}
