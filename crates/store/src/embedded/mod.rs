pub mod arrow_schema;
pub mod buffer;
pub mod meta;
pub mod reader;
pub mod wal;
pub mod writer;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::info;
use ulid::Ulid;

use stomatopod_core::{
    config::EmbeddedConfig,
    domain::event::Event,
    error::StoreError,
    query::{
        events::EventQuery,
        funnel::{FunnelQuery, FunnelResult},
        pageviews::{PageviewsQuery, PageviewsResult, TimeRange, TopList},
    },
    traits::{MetaStore, StorageBackend},
};

use self::{
    buffer::EventBuffer, meta::SqliteMeta, reader::EmbeddedReader, wal::Wal, writer::ParquetWriter,
};

pub struct EmbeddedBackend {
    pub meta: Arc<SqliteMeta>,
    pub reader: Arc<EmbeddedReader>,
    tx: mpsc::Sender<Vec<Event>>,
}

impl EmbeddedBackend {
    pub async fn open(cfg: &EmbeddedConfig) -> anyhow::Result<Self> {
        let data_dir = cfg.data_dir.clone();
        tokio::fs::create_dir_all(&data_dir).await?;
        tokio::fs::create_dir_all(data_dir.join("parquet")).await?;
        tokio::fs::create_dir_all(data_dir.join("wal")).await?;

        let meta = Arc::new(SqliteMeta::open(&data_dir.join("meta.db")).await?);
        let wal = Wal::open(&data_dir.join("wal"), cfg.wal_fsync_interval_ms)?;
        let buffer = Arc::new(EventBuffer::new(cfg.parquet_flush_rows * 4));
        let reader = Arc::new(EmbeddedReader::new(data_dir.join("parquet")).await?);

        // Channel for batched writes from the ingest handler
        let (tx, rx) = mpsc::channel::<Vec<Event>>(256);

        // Replay WAL into buffer on startup
        wal.replay(&buffer)?;
        info!("WAL replay complete, starting Parquet flush worker");

        // Start background Parquet flush task
        let flush_writer = ParquetWriter::new(
            data_dir.join("parquet"),
            cfg.parquet_flush_rows,
            cfg.parquet_flush_interval_s,
        );
        tokio::spawn(flush_writer.run(rx, buffer.clone(), wal.clone(), reader.clone()));

        Ok(Self { meta, reader, tx })
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

    async fn query_top_pages(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.reader.query_top_pages(site_id, range, limit).await
    }

    async fn query_top_referrers(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.reader.query_top_referrers(site_id, range, limit).await
    }

    async fn query_top_countries(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.reader.query_top_countries(site_id, range, limit).await
    }

    async fn query_top_browsers(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.reader.query_top_browsers(site_id, range, limit).await
    }

    async fn query_top_devices(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.reader.query_top_devices(site_id, range, limit).await
    }

    async fn query_custom_events(&self, q: &EventQuery) -> Result<TopList, StoreError> {
        self.reader.query_custom_events(q).await
    }

    async fn query_funnel(&self, q: &FunnelQuery) -> Result<FunnelResult, StoreError> {
        self.reader.query_funnel(q).await
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
}
