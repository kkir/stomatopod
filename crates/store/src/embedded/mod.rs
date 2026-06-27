pub mod arrow_schema;
pub mod buffer;
pub mod meta;
pub mod reader;
pub mod spans;
pub mod util;
pub mod wal;
pub mod wal_common;
pub mod writer;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::info;
use ulid::Ulid;

use stomatopod_core::{
    config::EmbeddedConfig,
    domain::{agent_span::AgentSpan, event::Event},
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
    },
    traits::{AgentStore, MetaStore, StorageBackend},
};

use self::{
    buffer::EventBuffer,
    meta::SqliteMeta,
    reader::EmbeddedReader,
    spans::{buffer::SpanBuffer, reader::SpanReader, wal::SpanWal, writer::SpanParquetWriter},
    wal::Wal,
    writer::ParquetWriter,
};

pub struct EmbeddedBackend {
    pub meta: Arc<SqliteMeta>,
    pub reader: Arc<EmbeddedReader>,
    pub span_reader: Arc<SpanReader>,
    tx: mpsc::Sender<Vec<Event>>,
    span_tx: mpsc::Sender<Vec<AgentSpan>>,
}

impl EmbeddedBackend {
    pub async fn open(cfg: &EmbeddedConfig) -> anyhow::Result<Self> {
        let data_dir = cfg.data_dir.clone();
        tokio::fs::create_dir_all(&data_dir).await?;
        tokio::fs::create_dir_all(data_dir.join("parquet")).await?;
        tokio::fs::create_dir_all(data_dir.join("wal")).await?;
        tokio::fs::create_dir_all(data_dir.join("parquet_spans").join("v1")).await?;
        tokio::fs::create_dir_all(data_dir.join("wal_spans")).await?;

        let meta = Arc::new(SqliteMeta::open(&data_dir.join("meta.db")).await?);
        let wal = Wal::open(&data_dir.join("wal"), cfg.wal_fsync_interval_ms)?;
        let buffer = Arc::new(EventBuffer::new(cfg.parquet_flush_rows * 4));
        let reader = Arc::new(EmbeddedReader::new(data_dir.join("parquet")).await?);

        let span_wal = SpanWal::open(&data_dir.join("wal_spans"))?;
        let span_buffer = Arc::new(SpanBuffer::new(cfg.parquet_flush_rows * 4));
        let span_reader =
            Arc::new(SpanReader::new(data_dir.join("parquet_spans").join("v1")).await?);

        // Channel for batched writes from the ingest handler
        let (tx, rx) = mpsc::channel::<Vec<Event>>(256);
        let (span_tx, span_rx) = mpsc::channel::<Vec<AgentSpan>>(256);

        // Replay WAL into buffer on startup
        wal.replay(&buffer)?;
        span_wal.replay(&span_buffer)?;
        info!("WAL replay complete, starting Parquet flush workers");

        // Start background Parquet flush task
        let flush_writer = ParquetWriter::new(
            data_dir.join("parquet"),
            cfg.parquet_flush_rows,
            cfg.parquet_flush_interval_s,
        );
        tokio::spawn(flush_writer.run(rx, buffer.clone(), wal.clone(), reader.clone()));

        let span_writer = SpanParquetWriter::new(
            data_dir.join("parquet_spans").join("v1"),
            cfg.parquet_flush_rows,
            cfg.parquet_flush_interval_s,
        );
        tokio::spawn(span_writer.run(
            span_rx,
            span_buffer.clone(),
            span_wal.clone(),
            span_reader.clone(),
        ));

        Ok(Self {
            meta,
            reader,
            span_reader,
            tx,
            span_tx,
        })
    }
}

#[async_trait]
impl StorageBackend for EmbeddedBackend {
    async fn ingest_events(&self, events: Vec<Event>) -> Result<(), StoreError> {
        self.tx
            .send(events)
            .await
            .map_err(|_| StoreError::Unavailable("ingest channel closed".into()))
    }

    async fn query_pageviews(&self, q: &PageviewsQuery) -> Result<PageviewsResult, StoreError> {
        self.reader.query_pageviews(q).await
    }

    async fn query_top_list(
        &self,
        site_id: Ulid,
        field: TopListField,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<TopList, StoreError> {
        self.reader
            .query_top_list(site_id, field, range, limit, filters)
            .await
    }

    async fn query_custom_events(&self, q: &EventQuery) -> Result<TopList, StoreError> {
        self.reader.query_custom_events(q).await
    }

    async fn query_funnel(&self, q: &FunnelQuery) -> Result<FunnelResult, StoreError> {
        self.reader.query_funnel(q).await
    }

    async fn query_entry_pages(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<EntryPages, StoreError> {
        self.reader
            .query_entry_pages(site_id, range, limit, filters)
            .await
    }

    async fn query_exit_pages(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<ExitPages, StoreError> {
        self.reader
            .query_exit_pages(site_id, range, limit, filters)
            .await
    }

    async fn query_realtime(
        &self,
        site_id: Ulid,
        window_minutes: u32,
    ) -> Result<RealtimeSnapshot, StoreError> {
        self.reader.query_realtime(site_id, window_minutes).await
    }

    async fn query_goal(&self, q: &GoalQuery) -> Result<GoalStats, StoreError> {
        self.reader.query_goal(q).await
    }

    async fn query_sessions(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<Vec<SessionRow>, StoreError> {
        self.reader.query_sessions(site_id, range, limit).await
    }

    async fn query_events_list(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<Vec<RawEventRow>, StoreError> {
        self.reader.query_events_list(site_id, range, limit).await
    }

    async fn query_top_sparklines(
        &self,
        site_id: Ulid,
        field: TopListField,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<TopSparklines, StoreError> {
        self.reader
            .query_top_sparklines(site_id, field, range, limit, filters)
            .await
    }

    async fn query_retention(
        &self,
        site_id: Ulid,
        range: &TimeRange,
    ) -> Result<RetentionGrid, StoreError> {
        self.reader.query_retention(site_id, range).await
    }

    async fn query_paths(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        depth: u32,
        limit: u32,
    ) -> Result<PathReport, StoreError> {
        self.reader.query_paths(site_id, range, depth, limit).await
    }

    async fn query_event_props(
        &self,
        site_id: Ulid,
        names: &[String],
        range: &TimeRange,
        limit: u32,
    ) -> Result<Vec<stomatopod_core::query::tier4::EventPropRow>, StoreError> {
        self.reader
            .query_event_props(site_id, names, range, limit)
            .await
    }
}

// Forward MetaStore calls to the SQLite meta store
#[async_trait]
impl MetaStore for EmbeddedBackend {
    async fn create_site(
        &self,
        site: &stomatopod_core::domain::site::Site,
    ) -> Result<(), StoreError> {
        self.meta.create_site(site).await
    }

    async fn update_site(
        &self,
        site: &stomatopod_core::domain::site::Site,
    ) -> Result<(), StoreError> {
        self.meta.update_site(site).await
    }

    async fn get_site(
        &self,
        id: Ulid,
    ) -> Result<Option<stomatopod_core::domain::site::Site>, StoreError> {
        self.meta.get_site(id).await
    }

    async fn get_site_by_key(
        &self,
        public_key: &str,
    ) -> Result<Option<stomatopod_core::domain::site::Site>, StoreError> {
        self.meta.get_site_by_key(public_key).await
    }

    async fn get_site_by_domain(
        &self,
        domain: &str,
    ) -> Result<Option<stomatopod_core::domain::site::Site>, StoreError> {
        self.meta.get_site_by_domain(domain).await
    }

    async fn list_sites(
        &self,
        org_id: Ulid,
    ) -> Result<Vec<stomatopod_core::domain::site::Site>, StoreError> {
        self.meta.list_sites(org_id).await
    }

    async fn delete_site(&self, id: Ulid) -> Result<(), StoreError> {
        self.meta.delete_site(id).await
    }

    async fn create_org(
        &self,
        org: &stomatopod_core::domain::org::Organization,
    ) -> Result<(), StoreError> {
        self.meta.create_org(org).await
    }

    async fn get_org(
        &self,
        id: Ulid,
    ) -> Result<Option<stomatopod_core::domain::org::Organization>, StoreError> {
        self.meta.get_org(id).await
    }

    async fn list_orgs(
        &self,
    ) -> Result<Vec<stomatopod_core::domain::org::Organization>, StoreError> {
        self.meta.list_orgs().await
    }

    async fn create_user(
        &self,
        user: &stomatopod_core::domain::org::User,
    ) -> Result<(), StoreError> {
        self.meta.create_user(user).await
    }

    async fn get_user_by_email(
        &self,
        email: &str,
    ) -> Result<Option<stomatopod_core::domain::org::User>, StoreError> {
        self.meta.get_user_by_email(email).await
    }

    async fn get_user(
        &self,
        id: Ulid,
    ) -> Result<Option<stomatopod_core::domain::org::User>, StoreError> {
        self.meta.get_user(id).await
    }

    async fn create_funnel(
        &self,
        funnel: &stomatopod_core::domain::org::Funnel,
    ) -> Result<(), StoreError> {
        self.meta.create_funnel(funnel).await
    }

    async fn get_funnel(
        &self,
        id: Ulid,
    ) -> Result<Option<stomatopod_core::domain::org::Funnel>, StoreError> {
        self.meta.get_funnel(id).await
    }

    async fn list_funnels(
        &self,
        site_id: Ulid,
    ) -> Result<Vec<stomatopod_core::domain::org::Funnel>, StoreError> {
        self.meta.list_funnels(site_id).await
    }

    async fn delete_funnel(&self, id: Ulid) -> Result<(), StoreError> {
        self.meta.delete_funnel(id).await
    }

    async fn create_goal(
        &self,
        goal: &stomatopod_core::domain::goal::Goal,
    ) -> Result<(), StoreError> {
        self.meta.create_goal(goal).await
    }

    async fn get_goal(
        &self,
        id: Ulid,
    ) -> Result<Option<stomatopod_core::domain::goal::Goal>, StoreError> {
        self.meta.get_goal(id).await
    }

    async fn list_goals(
        &self,
        site_id: Ulid,
    ) -> Result<Vec<stomatopod_core::domain::goal::Goal>, StoreError> {
        self.meta.list_goals(site_id).await
    }

    async fn delete_goal(&self, id: Ulid) -> Result<(), StoreError> {
        self.meta.delete_goal(id).await
    }

    async fn create_annotation(
        &self,
        annotation: &stomatopod_core::domain::annotation::Annotation,
    ) -> Result<(), StoreError> {
        self.meta.create_annotation(annotation).await
    }

    async fn get_annotation(
        &self,
        id: Ulid,
    ) -> Result<Option<stomatopod_core::domain::annotation::Annotation>, StoreError> {
        self.meta.get_annotation(id).await
    }

    async fn list_annotations(
        &self,
        site_id: Ulid,
        start: chrono::NaiveDate,
        end: chrono::NaiveDate,
    ) -> Result<Vec<stomatopod_core::domain::annotation::Annotation>, StoreError> {
        self.meta.list_annotations(site_id, start, end).await
    }

    async fn delete_annotation(&self, id: Ulid) -> Result<(), StoreError> {
        self.meta.delete_annotation(id).await
    }

    async fn create_analytics_alert(
        &self,
        alert: &stomatopod_core::domain::analytics_alert::AnalyticsAlert,
    ) -> Result<(), StoreError> {
        self.meta.create_analytics_alert(alert).await
    }

    async fn get_analytics_alert(
        &self,
        id: Ulid,
    ) -> Result<Option<stomatopod_core::domain::analytics_alert::AnalyticsAlert>, StoreError> {
        self.meta.get_analytics_alert(id).await
    }

    async fn list_analytics_alerts(
        &self,
        site_id: Ulid,
    ) -> Result<Vec<stomatopod_core::domain::analytics_alert::AnalyticsAlert>, StoreError> {
        self.meta.list_analytics_alerts(site_id).await
    }

    async fn list_enabled_analytics_alerts(
        &self,
    ) -> Result<Vec<stomatopod_core::domain::analytics_alert::AnalyticsAlert>, StoreError> {
        self.meta.list_enabled_analytics_alerts().await
    }

    async fn set_analytics_alert_enabled(&self, id: Ulid, enabled: bool) -> Result<(), StoreError> {
        self.meta.set_analytics_alert_enabled(id, enabled).await
    }

    async fn delete_analytics_alert(&self, id: Ulid) -> Result<(), StoreError> {
        self.meta.delete_analytics_alert(id).await
    }

    async fn record_analytics_alert_fire(
        &self,
        fire: &stomatopod_core::domain::analytics_alert::AnalyticsAlertFire,
    ) -> Result<(), StoreError> {
        self.meta.record_analytics_alert_fire(fire).await
    }

    async fn last_analytics_alert_fire(
        &self,
        alert_id: Ulid,
    ) -> Result<Option<stomatopod_core::domain::analytics_alert::AnalyticsAlertFire>, StoreError>
    {
        self.meta.last_analytics_alert_fire(alert_id).await
    }

    async fn create_share_link(
        &self,
        link: &stomatopod_core::domain::share_link::ShareLink,
    ) -> Result<(), StoreError> {
        self.meta.create_share_link(link).await
    }

    async fn list_share_links(
        &self,
        site_id: Ulid,
    ) -> Result<Vec<stomatopod_core::domain::share_link::ShareLink>, StoreError> {
        self.meta.list_share_links(site_id).await
    }

    async fn get_share_link(
        &self,
        id: Ulid,
    ) -> Result<Option<stomatopod_core::domain::share_link::ShareLink>, StoreError> {
        self.meta.get_share_link(id).await
    }

    async fn get_share_link_by_token(
        &self,
        token: &str,
    ) -> Result<Option<stomatopod_core::domain::share_link::ShareLink>, StoreError> {
        self.meta.get_share_link_by_token(token).await
    }

    async fn update_share_link(
        &self,
        id: Ulid,
        label: Option<String>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), StoreError> {
        self.meta.update_share_link(id, label, expires_at).await
    }

    async fn delete_share_link(&self, id: Ulid) -> Result<(), StoreError> {
        self.meta.delete_share_link(id).await
    }

    async fn upsert_digest_subscription(
        &self,
        sub: &stomatopod_core::domain::digest::DigestSubscription,
    ) -> Result<(), StoreError> {
        self.meta.upsert_digest_subscription(sub).await
    }

    async fn get_digest_subscription(
        &self,
        user_id: Ulid,
        site_id: Ulid,
    ) -> Result<Option<stomatopod_core::domain::digest::DigestSubscription>, StoreError> {
        self.meta.get_digest_subscription(user_id, site_id).await
    }

    async fn delete_digest_subscription(
        &self,
        user_id: Ulid,
        site_id: Ulid,
    ) -> Result<(), StoreError> {
        self.meta.delete_digest_subscription(user_id, site_id).await
    }

    async fn list_enabled_digest_subscriptions(
        &self,
    ) -> Result<Vec<stomatopod_core::domain::digest::DigestSubscription>, StoreError> {
        self.meta.list_enabled_digest_subscriptions().await
    }

    async fn record_digest_bounce(&self, id: Ulid, disable_at: u32) -> Result<(), StoreError> {
        self.meta.record_digest_bounce(id, disable_at).await
    }

    async fn upsert_agent(
        &self,
        agent: &stomatopod_core::domain::agent::Agent,
    ) -> Result<(), StoreError> {
        self.meta.upsert_agent(agent).await
    }

    async fn list_agents(
        &self,
        site_id: Ulid,
    ) -> Result<Vec<stomatopod_core::domain::agent::Agent>, StoreError> {
        self.meta.list_agents(site_id).await
    }

    async fn get_agent(
        &self,
        site_id: Ulid,
        agent_id: &str,
    ) -> Result<Option<stomatopod_core::domain::agent::Agent>, StoreError> {
        self.meta.get_agent(site_id, agent_id).await
    }

    async fn create_sentinel_token(
        &self,
        token: &stomatopod_core::domain::agent::SentinelToken,
    ) -> Result<(), StoreError> {
        self.meta.create_sentinel_token(token).await
    }

    async fn list_sentinel_tokens(
        &self,
        site_id: Ulid,
    ) -> Result<Vec<stomatopod_core::domain::agent::SentinelToken>, StoreError> {
        self.meta.list_sentinel_tokens(site_id).await
    }

    async fn get_sentinel_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<stomatopod_core::domain::agent::SentinelToken>, StoreError> {
        self.meta.get_sentinel_token_by_hash(token_hash).await
    }

    async fn touch_sentinel_token(&self, id: Ulid) -> Result<(), StoreError> {
        self.meta.touch_sentinel_token(id).await
    }

    async fn delete_sentinel_token(&self, id: Ulid) -> Result<(), StoreError> {
        self.meta.delete_sentinel_token(id).await
    }

    async fn create_api_key(
        &self,
        key: &stomatopod_core::domain::api_key::ApiKey,
    ) -> Result<(), StoreError> {
        self.meta.create_api_key(key).await
    }

    async fn list_api_keys(
        &self,
        org_id: Ulid,
    ) -> Result<Vec<stomatopod_core::domain::api_key::ApiKey>, StoreError> {
        self.meta.list_api_keys(org_id).await
    }

    async fn get_api_key_by_hash(
        &self,
        key_hash: &str,
    ) -> Result<Option<stomatopod_core::domain::api_key::ApiKey>, StoreError> {
        self.meta.get_api_key_by_hash(key_hash).await
    }

    async fn touch_api_key(&self, id: Ulid) -> Result<(), StoreError> {
        self.meta.touch_api_key(id).await
    }

    async fn delete_api_key(&self, id: Ulid) -> Result<(), StoreError> {
        self.meta.delete_api_key(id).await
    }

    async fn create_alert_channel(
        &self,
        channel: &stomatopod_core::domain::agent::AlertChannel,
    ) -> Result<(), StoreError> {
        self.meta.create_alert_channel(channel).await
    }

    async fn list_alert_channels(
        &self,
        site_id: Ulid,
    ) -> Result<Vec<stomatopod_core::domain::agent::AlertChannel>, StoreError> {
        self.meta.list_alert_channels(site_id).await
    }

    async fn delete_alert_channel(&self, id: Ulid) -> Result<(), StoreError> {
        self.meta.delete_alert_channel(id).await
    }

    async fn upsert_policy(
        &self,
        policy: &stomatopod_core::domain::policy::Policy,
    ) -> Result<(), StoreError> {
        self.meta.upsert_policy(policy).await
    }

    async fn get_policy(
        &self,
        site_id: Ulid,
    ) -> Result<Option<stomatopod_core::domain::policy::Policy>, StoreError> {
        self.meta.get_policy(site_id).await
    }

    async fn record_incident(
        &self,
        incident: &stomatopod_core::domain::incident::Incident,
    ) -> Result<(), StoreError> {
        self.meta.record_incident(incident).await
    }

    async fn list_incidents(
        &self,
        site_id: Ulid,
        limit: u32,
    ) -> Result<Vec<stomatopod_core::domain::incident::Incident>, StoreError> {
        self.meta.list_incidents(site_id, limit).await
    }

    async fn update_incident_status(
        &self,
        id: Ulid,
        status: stomatopod_core::domain::incident::IncidentStatus,
    ) -> Result<(), StoreError> {
        self.meta.update_incident_status(id, status).await
    }
}

#[async_trait]
impl AgentStore for EmbeddedBackend {
    async fn ingest_spans(&self, spans: Vec<AgentSpan>) -> Result<(), StoreError> {
        self.span_tx
            .send(spans)
            .await
            .map_err(|_| StoreError::Unavailable("span ingest channel closed".into()))
    }

    async fn query_spans(&self, q: &SpanQuery) -> Result<Vec<SpanRow>, StoreError> {
        self.span_reader.query_spans(q).await
    }

    async fn summarize_agents(
        &self,
        site_id: Ulid,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<AgentSummary>, StoreError> {
        self.span_reader.summarize_agents(site_id, since).await
    }

    async fn session_cost_usd(
        &self,
        site_id: Ulid,
        agent_session_id: &str,
    ) -> Result<f64, StoreError> {
        self.span_reader
            .session_cost_usd(site_id, agent_session_id)
            .await
    }
}
