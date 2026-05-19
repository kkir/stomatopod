use std::sync::Arc;

use dashmap::DashMap;
use minijinja::Environment;
use stomatopod_core::{
    config::Config,
    traits::{AgentStore, MetaStore, StorageBackend},
};
use tokio::sync::mpsc;
use ulid::Ulid;

use stomatopod_ingest::geo::GeoLookup;

pub struct AppState {
    pub backend: Arc<dyn StorageBackend>,
    pub agent_store: Arc<dyn AgentStore>,
    pub meta: Arc<dyn MetaStore>,
    pub templates: Environment<'static>,
    pub config: Arc<Config>,
    pub tracker_hash: String,
    /// Channel to the batch accumulator for event ingestion.
    pub ingest_tx: mpsc::Sender<stomatopod_core::domain::event::Event>,
    /// Channel to the span batch accumulator.
    pub span_ingest_tx: mpsc::Sender<stomatopod_core::domain::agent_span::AgentSpan>,
    /// Site key cache: public_key → site_id.
    pub site_cache: Arc<DashMap<String, Ulid>>,
    /// Sentinel token cache: token_hash → site_id.
    pub sentinel_token_cache: Arc<DashMap<String, Ulid>>,
    /// PII redaction keys applied to span `properties`.
    pub redact_keys: Arc<Vec<String>>,
    /// GeoIP lookup service.
    pub geo: Arc<GeoLookup>,
}
