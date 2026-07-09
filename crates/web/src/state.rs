use std::sync::Arc;

use dashmap::DashMap;
use stomatopod_core::{
    config::Config,
    domain::api_key::ApiKeyScope,
    traits::{MetaStore, StorageBackend},
};
use tokio::sync::mpsc;
use ulid::Ulid;

use stomatopod_ingest::geo::GeoLookup;

/// A resolved API key held in the in-memory cache. `key_id` lets cache
/// hits still update `last_used_at`; `scope`/`site_id` let the same map
/// back both the ingest and read-auth paths.
#[derive(Debug, Clone, Copy)]
pub struct ApiKeyCacheEntry {
    pub org_id: Ulid,
    pub site_id: Option<Ulid>,
    pub key_id: Ulid,
    pub scope: ApiKeyScope,
}

pub struct AppState {
    pub backend: Arc<dyn StorageBackend>,
    pub meta: Arc<dyn MetaStore>,
    pub config: Arc<Config>,
    pub tracker_hash: String,
    /// Channel to the batch accumulator for event ingestion.
    pub ingest_tx: mpsc::Sender<stomatopod_core::domain::event::Event>,
    /// Site key cache: public_key → site_id.
    pub site_cache: Arc<DashMap<String, Ulid>>,
    /// API key cache: key_hash → resolved key. Serves both the ingest and
    /// read-auth paths. Evicted on revocation so deletes take effect at once.
    pub api_key_cache: Arc<DashMap<String, ApiKeyCacheEntry>>,
    /// GeoIP lookup service.
    pub geo: Arc<GeoLookup>,
    /// Email transport for digest delivery (test-send + scheduler).
    pub digest_sender: Arc<dyn crate::digest::DigestSender>,
}
