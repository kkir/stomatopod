use std::{collections::HashSet, sync::Arc, time::Duration};

use parking_lot::RwLock;
use serde::Deserialize;
use tracing::{info, warn};

/// State exposed to the proxy handler. `kill_until_resume = true` means
/// the next outbound request should be short-circuited.
#[derive(Default)]
pub struct ControlState {
    pub kill_until_resume: parking_lot::Mutex<Option<String>>,
    pub hint: parking_lot::Mutex<Option<String>>,
    seen_seqs: RwLock<HashSet<u64>>,
}

impl ControlState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn current_kill_reason(&self) -> Option<String> {
        self.kill_until_resume.lock().clone()
    }

    pub fn take_hint(&self) -> Option<String> {
        self.hint.lock().take()
    }

    pub fn apply(&self, env: ControlEnvelope) {
        // Dedup by seq across reconnects.
        {
            let mut seen = self.seen_seqs.write();
            if !seen.insert(env.seq) {
                return;
            }
            // Cap memory.
            if seen.len() > 4096 {
                seen.clear();
            }
        }
        match env.command {
            ControlCommand::Kill { reason } => {
                info!(seq = env.seq, %reason, "control: KILL");
                *self.kill_until_resume.lock() = Some(reason);
            }
            ControlCommand::Hint { message } => {
                info!(seq = env.seq, "control: HINT");
                *self.hint.lock() = Some(message);
            }
            ControlCommand::Resume => {
                info!(seq = env.seq, "control: RESUME");
                *self.kill_until_resume.lock() = None;
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ControlEnvelope {
    pub seq: u64,
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
