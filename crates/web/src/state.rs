use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use dashmap::DashMap;
use minijinja::Environment;
use stomatopod_core::{
    config::Config,
    domain::control::ControlEnvelope,
    traits::{AgentStore, MetaStore, StorageBackend},
};
use tokio::sync::{broadcast, mpsc};
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
    /// Per-site broadcast channels for sentinel control commands.
    /// `broadcast` (not mpsc) so multiple sidecars per site can subscribe
    /// and slow consumers lag rather than deadlock the publisher.
    pub control_channels: DashMap<Ulid, broadcast::Sender<ControlEnvelope>>,
    /// Monotonic sequence used to dedupe control commands on the
    /// sidecar after reconnects.
    pub control_seq: AtomicU64,
}

impl AppState {
    /// Get or create the broadcast channel for a site. Channel capacity
    /// is 64; if a sidecar lags by more than 64 messages it'll see a
    /// Lagged error and skip — preferable to blocking publish.
    pub fn control_channel(&self, site_id: Ulid) -> broadcast::Sender<ControlEnvelope> {
        if let Some(tx) = self.control_channels.get(&site_id) {
            return tx.clone();
        }
        let (tx, _) = broadcast::channel(64);
        self.control_channels.insert(site_id, tx.clone());
        tx
    }

    pub fn next_control_seq(&self) -> u64 {
        self.control_seq.fetch_add(1, Ordering::SeqCst)
    }
}
