use std::{collections::HashSet, sync::Arc, time::Duration};

use parking_lot::Mutex;
use serde::Deserialize;
use tracing::{info, warn};

/// Shared kill-switch + hint state for the proxy. Three independent mutexes
/// were collapsed into one inner struct so command application is atomic
/// (a `Kill` immediately followed by a `Resume` no longer races against a
/// proxy handler reading the kill reason between the two writes).
pub struct ControlState {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    kill_until_resume: Option<String>,
    hint: Option<String>,
    seen_seqs: HashSet<u64>,
}

impl Default for ControlState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }
}

impl ControlState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn current_kill_reason(&self) -> Option<String> {
        self.inner.lock().kill_until_resume.clone()
    }

    pub fn take_hint(&self) -> Option<String> {
        self.inner.lock().hint.take()
    }

    /// Set the kill reason from the local enforcement layer (cost cap,
    /// velocity, repetition). Server-pushed kills go through `apply`.
    pub fn set_kill_reason(&self, reason: String) {
        self.inner.lock().kill_until_resume = Some(reason);
    }

    pub fn apply(&self, env: ControlEnvelope) {
        let mut s = self.inner.lock();
        // Dedup by seq across reconnects.
        if !s.seen_seqs.insert(env.seq) {
            return;
        }
        if s.seen_seqs.len() > 4096 {
            s.seen_seqs.clear();
        }
        match env.command {
            ControlCommand::Kill { reason } => {
                info!(seq = env.seq, %reason, "control: KILL");
                s.kill_until_resume = Some(reason);
            }
            ControlCommand::Hint { message } => {
                info!(seq = env.seq, "control: HINT");
                s.hint = Some(message);
            }
            ControlCommand::Resume => {
                info!(seq = env.seq, "control: RESUME");
                s.kill_until_resume = None;
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ControlEnvelope {
    pub seq: u64,
    /// Server-side agent identifier for logging/auditing on the receiving
    /// dashboard. Carried on the wire even though the sidecar itself
    /// doesn't dispatch on it.
    #[allow(dead_code)]
    pub agent_id: String,
    #[serde(flatten)]
    pub command: ControlCommand,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ControlCommand {
    Kill { reason: String },
    Hint { message: String },
    Resume,
}

/// Connects to `server_url/api/v1/sentinel/stream` over SSE and feeds
/// every event into `state.apply`. Reconnects with exponential backoff
/// up to 16 s on transport errors.
pub async fn run_control_loop(server_url: String, token: String, state: Arc<ControlState>) {
    let url = format!(
        "{}/api/v1/sentinel/stream",
        server_url.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(0)) // no overall timeout — long-lived stream
        .build()
        .expect("reqwest client");

    let mut delay = Duration::from_secs(1);
    loop {
        match connect(&client, &url, &token, state.clone()).await {
            Ok(_) => {
                // Server closed cleanly.
                delay = Duration::from_secs(1);
            }
            Err(e) => {
                warn!("control stream error: {e}; reconnecting in {:?}", delay);
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(16));
            }
        }
    }
}

async fn connect(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    state: Arc<ControlState>,
) -> anyhow::Result<()> {
    use eventsource_stream::Eventsource;
    use futures_util::StreamExt;

    let resp = client
        .get(url)
        .bearer_auth(token)
        .header("accept", "text/event-stream")
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("server returned {}", resp.status());
    }
    info!("SSE control stream connected");

    let mut stream = resp.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        let event = event?;
        if event.data.is_empty() {
            continue;
        }
        match serde_json::from_str::<ControlEnvelope>(&event.data) {
            Ok(env) => state.apply(env),
            Err(e) => warn!("malformed control event: {e}, data={}", event.data),
        }
    }
    Ok(())
}
