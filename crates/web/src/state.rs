use std::sync::Arc;

use dashmap::DashMap;
use minijinja::Environment;
use stomatopod_core::{
    config::Config,
    traits::{MetaStore, StorageBackend},
};
use tokio::sync::mpsc;
use ulid::Ulid;

use stomatopod_ingest::geo::GeoLookup;

pub struct AppState {
    pub backend: Arc<dyn StorageBackend>,
    pub meta: Arc<dyn MetaStore>,
    pub templates: Environment<'static>,
    pub config: Arc<Config>,
    pub tracker_hash: String,
    /// Channel to the batch accumulator for event ingestion.
    pub ingest_tx: mpsc::Sender<stomatopod_core::domain::event::Event>,
    /// Site key cache: public_key → site_id.
    pub site_cache: Arc<DashMap<String, Ulid>>,
    /// GeoIP lookup service.
    pub geo: Arc<GeoLookup>,
}
