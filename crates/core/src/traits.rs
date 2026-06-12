use async_trait::async_trait;
use ulid::Ulid;

use crate::{
    domain::{
        agent::{Agent, AlertChannel, SentinelToken},
        agent_span::AgentSpan,
        incident::{Incident, IncidentStatus},
        org::{Funnel, Organization, User},
        policy::Policy,
        site::Site,
    },
    error::StoreError,
    query::{
        events::EventQuery,
        funnel::{FunnelQuery, FunnelResult},
        pageviews::{PageviewsQuery, PageviewsResult, TimeRange, TopList, TopListField},
        spans::{AgentSummary, SpanQuery, SpanRow},
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
    /// or device type) and return the top `limit` values ordered by traffic.
    async fn query_top_list(
        &self,
        site_id: Ulid,
        field: TopListField,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError>;

    async fn query_custom_events(&self, q: &EventQuery) -> Result<TopList, StoreError>;

    async fn query_funnel(&self, q: &FunnelQuery) -> Result<FunnelResult, StoreError>;
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
