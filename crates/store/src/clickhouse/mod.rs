//! ClickHouse-backed analytics storage for high-scale SaaS deployments.
//! Uses the HTTP interface with FORMAT JSONEachRow for portability across
//! ClickHouse versions.

use async_trait::async_trait;
use ulid::Ulid;

use stomatopod_core::{
    config::ClickhouseConfig,
    domain::{agent_span::AgentSpan, event::Event},
    error::StoreError,
    query::{
        events::EventQuery,
        funnel::{FunnelQuery, FunnelResult},
        pageviews::{PageviewsQuery, PageviewsResult, TimeRange, TopList},
        spans::{AgentSummary, SpanQuery, SpanRow},
    },
    traits::{AgentStore, StorageBackend},
};

pub struct ClickhouseBackend {
    client: reqwest::Client,
    url: String,
    database: String,
    username: String,
    password: String,
}

impl ClickhouseBackend {
    pub fn new(cfg: &ClickhouseConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: cfg.url.clone(),
            database: cfg.database.clone(),
            username: cfg.username.clone(),
            password: cfg.password.clone(),
        }
    }

    async fn execute(&self, query: &str) -> Result<String, StoreError> {
        let response = self
            .client
            .post(&self.url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("database", &self.database)])
            .body(query.to_string())
            .send()
            .await
            .map_err(StoreError::db)?;

        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(StoreError::db(format!("ClickHouse error: {text}")));
        }

        response.text().await.map_err(StoreError::db)
    }
}

#[async_trait]
impl StorageBackend for ClickhouseBackend {
    async fn ingest_events(&self, events: Vec<Event>) -> Result<(), StoreError> {
        if events.is_empty() {
            return Ok(());
        }
        let rows: Vec<String> = events
            .iter()
            .map(|e| serde_json::to_string(e).unwrap_or_default())
            .collect();
        let body = format!("INSERT INTO events FORMAT JSONEachRow\n{}", rows.join("\n"));
        self.execute(&body).await?;
        Ok(())
    }

    async fn query_pageviews(&self, _q: &PageviewsQuery) -> Result<PageviewsResult, StoreError> {
        todo!("clickhouse query_pageviews")
    }

    async fn query_top_pages(
        &self,
        _site_id: Ulid,
        _range: &TimeRange,
        _limit: u32,
    ) -> Result<TopList, StoreError> {
        todo!("clickhouse query_top_pages")
    }

    async fn query_top_referrers(
        &self,
        _site_id: Ulid,
        _range: &TimeRange,
        _limit: u32,
    ) -> Result<TopList, StoreError> {
        todo!("clickhouse query_top_referrers")
    }

    async fn query_top_countries(
        &self,
        _site_id: Ulid,
        _range: &TimeRange,
        _limit: u32,
    ) -> Result<TopList, StoreError> {
        todo!("clickhouse query_top_countries")
    }

    async fn query_top_browsers(
        &self,
        _site_id: Ulid,
        _range: &TimeRange,
        _limit: u32,
    ) -> Result<TopList, StoreError> {
        todo!("clickhouse query_top_browsers")
    }

    async fn query_top_devices(
        &self,
        _site_id: Ulid,
        _range: &TimeRange,
        _limit: u32,
    ) -> Result<TopList, StoreError> {
        todo!("clickhouse query_top_devices")
    }

    async fn query_custom_events(&self, _q: &EventQuery) -> Result<TopList, StoreError> {
        todo!("clickhouse query_custom_events")
    }

    async fn query_funnel(&self, _q: &FunnelQuery) -> Result<FunnelResult, StoreError> {
        todo!("clickhouse query_funnel")
    }
}

// AgentStore is deferred to Phase 8+. The trait is implemented here as
// `todo!()` stubs so the SaaS backend already satisfies the `Arc<dyn
// AgentStore>` bound in `bin/stomatopod/src/main.rs` when it is eventually
// wired up — main currently bails on `StorageConfig::Clickhouse` before this
// impl is ever called.
#[async_trait]
impl AgentStore for ClickhouseBackend {
    async fn ingest_spans(&self, _spans: Vec<AgentSpan>) -> Result<(), StoreError> {
        todo!("clickhouse ingest_spans (Phase 8+)")
    }

    async fn query_spans(&self, _q: &SpanQuery) -> Result<Vec<SpanRow>, StoreError> {
        todo!("clickhouse query_spans (Phase 8+)")
    }

    async fn summarize_agents(
        &self,
        _site_id: Ulid,
        _since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<AgentSummary>, StoreError> {
        todo!("clickhouse summarize_agents (Phase 8+)")
    }

    async fn session_cost_usd(
        &self,
        _site_id: Ulid,
        _agent_session_id: &str,
    ) -> Result<f64, StoreError> {
        todo!("clickhouse session_cost_usd (Phase 8+)")
    }
}
