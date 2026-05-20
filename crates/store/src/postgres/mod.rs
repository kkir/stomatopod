//! Postgres-backed storage implementation for SaaS deployments.
//! Uses sqlx with connection pooling and monthly-partitioned event tables.

use async_trait::async_trait;
use ulid::Ulid;

use stomatopod_core::{
    config::PostgresConfig,
    domain::{
        agent::{Agent, AlertChannel, SentinelToken},
        agent_span::AgentSpan,
        event::Event,
        incident::{Incident, IncidentStatus},
        org::{Funnel, Organization, User},
        policy::Policy,
        site::Site,
    },
    error::StoreError,
    query::{
        events::EventQuery,
        funnel::{FunnelQuery, FunnelResult},
        pageviews::{PageviewsQuery, PageviewsResult, TimeRange, TopList},
        spans::{AgentSummary, SpanQuery, SpanRow},
    },
    traits::{AgentStore, MetaStore, StorageBackend},
};

pub struct PostgresBackend {
    _url: String,
}

impl PostgresBackend {
    pub async fn connect(cfg: &PostgresConfig) -> anyhow::Result<Self> {
        Ok(Self {
            _url: cfg.url.clone(),
        })
    }
}

#[async_trait]
impl StorageBackend for PostgresBackend {
    async fn ingest_events(&self, _events: Vec<Event>) -> Result<(), StoreError> {
        // TODO: batch INSERT INTO events (...) VALUES ...
        todo!("postgres ingest")
    }

    async fn query_pageviews(&self, _q: &PageviewsQuery) -> Result<PageviewsResult, StoreError> {
        todo!("postgres query_pageviews")
    }

    async fn query_top_pages(
        &self,
        _site_id: Ulid,
        _range: &TimeRange,
        _limit: u32,
    ) -> Result<TopList, StoreError> {
        todo!("postgres query_top_pages")
    }

    async fn query_top_referrers(
        &self,
        _site_id: Ulid,
        _range: &TimeRange,
        _limit: u32,
    ) -> Result<TopList, StoreError> {
        todo!("postgres query_top_referrers")
    }

    async fn query_top_countries(
        &self,
        _site_id: Ulid,
        _range: &TimeRange,
        _limit: u32,
    ) -> Result<TopList, StoreError> {
        todo!("postgres query_top_countries")
    }

    async fn query_top_browsers(
        &self,
        _site_id: Ulid,
        _range: &TimeRange,
        _limit: u32,
    ) -> Result<TopList, StoreError> {
        todo!("postgres query_top_browsers")
    }

    async fn query_top_devices(
        &self,
        _site_id: Ulid,
        _range: &TimeRange,
        _limit: u32,
    ) -> Result<TopList, StoreError> {
        todo!("postgres query_top_devices")
    }

    async fn query_custom_events(&self, _q: &EventQuery) -> Result<TopList, StoreError> {
        todo!("postgres query_custom_events")
    }

    async fn query_funnel(&self, _q: &FunnelQuery) -> Result<FunnelResult, StoreError> {
        todo!("postgres query_funnel")
    }
}

#[async_trait]
impl MetaStore for PostgresBackend {
    async fn create_site(&self, _site: &Site) -> Result<(), StoreError> {
        todo!()
    }
    async fn get_site(&self, _id: Ulid) -> Result<Option<Site>, StoreError> {
        todo!()
    }
    async fn get_site_by_key(&self, _public_key: &str) -> Result<Option<Site>, StoreError> {
        todo!()
    }
    async fn get_site_by_domain(&self, _domain: &str) -> Result<Option<Site>, StoreError> {
        todo!()
    }
    async fn list_sites(&self, _org_id: Ulid) -> Result<Vec<Site>, StoreError> {
        todo!()
    }
    async fn delete_site(&self, _id: Ulid) -> Result<(), StoreError> {
        todo!()
    }
    async fn create_org(&self, _org: &Organization) -> Result<(), StoreError> {
        todo!()
    }
    async fn get_org(&self, _id: Ulid) -> Result<Option<Organization>, StoreError> {
        todo!()
    }
    async fn list_orgs(&self) -> Result<Vec<Organization>, StoreError> {
        todo!()
    }
    async fn create_user(&self, _user: &User) -> Result<(), StoreError> {
        todo!()
    }
    async fn get_user_by_email(&self, _email: &str) -> Result<Option<User>, StoreError> {
        todo!()
    }
    async fn create_funnel(&self, _funnel: &Funnel) -> Result<(), StoreError> {
        todo!()
    }
    async fn get_funnel(&self, _id: Ulid) -> Result<Option<Funnel>, StoreError> {
        todo!()
    }
    async fn list_funnels(&self, _site_id: Ulid) -> Result<Vec<Funnel>, StoreError> {
        todo!()
    }
    async fn delete_funnel(&self, _id: Ulid) -> Result<(), StoreError> {
        todo!()
    }

    // ---- AI firewall metadata (Phase 8+) ----
    async fn upsert_agent(&self, _agent: &Agent) -> Result<(), StoreError> {
        todo!("postgres upsert_agent")
    }
    async fn list_agents(&self, _site_id: Ulid) -> Result<Vec<Agent>, StoreError> {
        todo!("postgres list_agents")
    }
    async fn get_agent(
        &self,
        _site_id: Ulid,
        _agent_id: &str,
    ) -> Result<Option<Agent>, StoreError> {
        todo!("postgres get_agent")
    }
    async fn create_sentinel_token(&self, _token: &SentinelToken) -> Result<(), StoreError> {
        todo!("postgres create_sentinel_token")
    }
    async fn list_sentinel_tokens(&self, _site_id: Ulid) -> Result<Vec<SentinelToken>, StoreError> {
        todo!("postgres list_sentinel_tokens")
    }
    async fn get_sentinel_token_by_hash(
        &self,
        _token_hash: &str,
    ) -> Result<Option<SentinelToken>, StoreError> {
        todo!("postgres get_sentinel_token_by_hash")
    }
    async fn touch_sentinel_token(&self, _id: Ulid) -> Result<(), StoreError> {
        todo!("postgres touch_sentinel_token")
    }
    async fn delete_sentinel_token(&self, _id: Ulid) -> Result<(), StoreError> {
        todo!("postgres delete_sentinel_token")
    }
    async fn create_alert_channel(&self, _channel: &AlertChannel) -> Result<(), StoreError> {
        todo!("postgres create_alert_channel")
    }
    async fn list_alert_channels(&self, _site_id: Ulid) -> Result<Vec<AlertChannel>, StoreError> {
        todo!("postgres list_alert_channels")
    }
    async fn delete_alert_channel(&self, _id: Ulid) -> Result<(), StoreError> {
        todo!("postgres delete_alert_channel")
    }
    async fn upsert_policy(&self, _policy: &Policy) -> Result<(), StoreError> {
        todo!("postgres upsert_policy")
    }
    async fn get_policy(&self, _site_id: Ulid) -> Result<Option<Policy>, StoreError> {
        todo!("postgres get_policy")
    }
    async fn record_incident(&self, _incident: &Incident) -> Result<(), StoreError> {
        todo!("postgres record_incident")
    }
    async fn list_incidents(
        &self,
        _site_id: Ulid,
        _limit: u32,
    ) -> Result<Vec<Incident>, StoreError> {
        todo!("postgres list_incidents")
    }
    async fn update_incident_status(
        &self,
        _id: Ulid,
        _status: IncidentStatus,
    ) -> Result<(), StoreError> {
        todo!("postgres update_incident_status")
    }
}

// AgentStore is deferred to Phase 8+. The trait is implemented here as
// `todo!()` stubs so the SaaS backend already satisfies the `Arc<dyn
// AgentStore>` bound in `bin/stomatopod/src/main.rs` when it is eventually
// wired up — main currently bails on `StorageConfig::Postgres` before this
// impl is ever called.
#[async_trait]
impl AgentStore for PostgresBackend {
    async fn ingest_spans(&self, _spans: Vec<AgentSpan>) -> Result<(), StoreError> {
        todo!("postgres ingest_spans (Phase 8+)")
    }

    async fn query_spans(&self, _q: &SpanQuery) -> Result<Vec<SpanRow>, StoreError> {
        todo!("postgres query_spans (Phase 8+)")
    }

    async fn summarize_agents(
        &self,
        _site_id: Ulid,
        _since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<AgentSummary>, StoreError> {
        todo!("postgres summarize_agents (Phase 8+)")
    }

    async fn session_cost_usd(
        &self,
        _site_id: Ulid,
        _agent_session_id: &str,
    ) -> Result<f64, StoreError> {
        todo!("postgres session_cost_usd (Phase 8+)")
    }
}
