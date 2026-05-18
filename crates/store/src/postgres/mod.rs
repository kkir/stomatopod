//! Postgres-backed storage implementation for SaaS deployments.
//! Uses sqlx with connection pooling and monthly-partitioned event tables.

use async_trait::async_trait;
use ulid::Ulid;

use stomatopod_core::{
    config::PostgresConfig,
    domain::{
        event::Event,
        org::{Funnel, Organization, User},
        site::Site,
    },
    error::StoreError,
    query::{
        events::EventQuery,
        funnel::{FunnelQuery, FunnelResult},
        pageviews::{PageviewsQuery, PageviewsResult, TimeRange, TopList},
    },
    traits::{MetaStore, StorageBackend},
};

pub struct PostgresBackend {
    _url: String,
}

impl PostgresBackend {
    pub async fn connect(cfg: &PostgresConfig) -> anyhow::Result<Self> {
        Ok(Self { _url: cfg.url.clone() })
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

    async fn query_top_pages(&self, _site_id: Ulid, _range: &TimeRange, _limit: u32) -> Result<TopList, StoreError> {
        todo!("postgres query_top_pages")
    }

    async fn query_top_referrers(&self, _site_id: Ulid, _range: &TimeRange, _limit: u32) -> Result<TopList, StoreError> {
        todo!("postgres query_top_referrers")
    }

    async fn query_top_countries(&self, _site_id: Ulid, _range: &TimeRange, _limit: u32) -> Result<TopList, StoreError> {
        todo!("postgres query_top_countries")
    }

    async fn query_top_browsers(&self, _site_id: Ulid, _range: &TimeRange, _limit: u32) -> Result<TopList, StoreError> {
        todo!("postgres query_top_browsers")
    }

    async fn query_top_devices(&self, _site_id: Ulid, _range: &TimeRange, _limit: u32) -> Result<TopList, StoreError> {
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
    async fn create_site(&self, _site: &Site) -> Result<(), StoreError> { todo!() }
    async fn get_site(&self, _id: Ulid) -> Result<Option<Site>, StoreError> { todo!() }
    async fn get_site_by_key(&self, _public_key: &str) -> Result<Option<Site>, StoreError> { todo!() }
    async fn get_site_by_domain(&self, _domain: &str) -> Result<Option<Site>, StoreError> { todo!() }
    async fn list_sites(&self, _org_id: Ulid) -> Result<Vec<Site>, StoreError> { todo!() }
    async fn delete_site(&self, _id: Ulid) -> Result<(), StoreError> { todo!() }
    async fn create_org(&self, _org: &Organization) -> Result<(), StoreError> { todo!() }
    async fn get_org(&self, _id: Ulid) -> Result<Option<Organization>, StoreError> { todo!() }
    async fn list_orgs(&self) -> Result<Vec<Organization>, StoreError> { todo!() }
    async fn create_user(&self, _user: &User) -> Result<(), StoreError> { todo!() }
    async fn get_user_by_email(&self, _email: &str) -> Result<Option<User>, StoreError> { todo!() }
    async fn create_funnel(&self, _funnel: &Funnel) -> Result<(), StoreError> { todo!() }
    async fn get_funnel(&self, _id: Ulid) -> Result<Option<Funnel>, StoreError> { todo!() }
    async fn list_funnels(&self, _site_id: Ulid) -> Result<Vec<Funnel>, StoreError> { todo!() }
    async fn delete_funnel(&self, _id: Ulid) -> Result<(), StoreError> { todo!() }
}
