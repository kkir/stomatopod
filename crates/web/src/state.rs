use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use dashmap::DashMap;
use stomatopod_core::{
    config::Config,
    domain::{api_key::ApiKeyScope, control::ControlEnvelope},
    traits::{AgentStore, MetaStore, StorageBackend},
};
use tokio::sync::{broadcast, mpsc};
use ulid::Ulid;

use stomatopod_ingest::geo::GeoLookup;

use crate::alerts::AlertDispatcher;

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
    pub agent_store: Arc<dyn AgentStore>,
    pub meta: Arc<dyn MetaStore>,
    pub config: Arc<Config>,
    pub tracker_hash: String,
    /// Channel to the batch accumulator for event ingestion.
    pub ingest_tx: mpsc::Sender<stomatopod_core::domain::event::Event>,
    /// Channel to the span batch accumulator. Whole request batches
    /// land atomically so a mid-batch back-pressure rejection can't
    /// leave the channel in a partially-accepted state.
    pub span_ingest_tx: mpsc::Sender<Vec<stomatopod_core::domain::agent_span::AgentSpan>>,
    /// Site key cache: public_key → site_id.
    pub site_cache: Arc<DashMap<String, Ulid>>,
    /// Sentinel token cache: token_hash → (site_id, token_id). The
    /// token id is stored so cache hits can still update `last_used_at`.
    pub sentinel_token_cache: Arc<DashMap<String, (Ulid, Ulid)>>,
    /// API key cache: key_hash → resolved key. Serves both the ingest and
    /// read-auth paths. Evicted on revocation so deletes take effect at once.
    pub api_key_cache: Arc<DashMap<String, ApiKeyCacheEntry>>,
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
    /// Outbound alert dispatcher (webhook + Slack).
    pub alerts: AlertDispatcher,
    /// Email transport for digest delivery (test-send + scheduler).
    pub digest_sender: Arc<dyn crate::digest::DigestSender>,
}

impl AppState {
    /// Get or create the broadcast channel for a site. Channel capacity
    /// is 64; if a sidecar lags by more than 64 messages it'll see a
    /// Lagged error and skip — preferable to blocking publish.
    ///
    /// Uses DashMap's entry API so concurrent callers can't each create
    /// their own sender and have publishes go to an unobserved channel.
    pub fn control_channel(&self, site_id: Ulid) -> broadcast::Sender<ControlEnvelope> {
        self.control_channels
            .entry(site_id)
            .or_insert_with(|| broadcast::channel(64).0)
            .clone()
    }

    pub fn next_control_seq(&self) -> u64 {
        self.control_seq.fetch_add(1, Ordering::SeqCst)
    }
}
