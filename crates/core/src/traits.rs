use async_trait::async_trait;
use ulid::Ulid;

use crate::{
    domain::{
        org::{Funnel, Organization, User},
        site::Site,
    },
    error::StoreError,
    query::{
        events::EventQuery,
        funnel::{FunnelQuery, FunnelResult},
        pageviews::{PageviewsQuery, PageviewsResult, TimeRange, TopList},
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

    async fn query_top_pages(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError>;

    async fn query_top_referrers(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError>;

    async fn query_top_countries(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError>;

    async fn query_top_browsers(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError>;

    async fn query_top_devices(
        &self,
        site_id: Ulid,
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
}
