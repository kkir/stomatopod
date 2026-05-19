use std::{path::PathBuf, sync::Arc, time::Duration};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Payload shape mirrors `SpanIngestPayload` on the server. Defined
/// here as well to keep the sidecar free of the `stomatopod-ingest`
/// dependency.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SpanRow {
    pub agent_id: String,
    pub agent_session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    pub kind: String,
    pub model: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: chrono::DateTime<chrono::Utc>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
    pub cost_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_input_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct SpanIngestPayload<'a> {
    spans: &'a [SpanRow],
}

/// Ships spans to stomatopod with batching and a disk spool. On server
/// 5xx or transport error, batches are persisted to `spool_dir/` and
/// retried on the next flush — so an outage during an incident doesn't
/// lose the evidence.
pub struct SpanShipper {
    server_url: String,
    token: String,
    http: reqwest::Client,
    tx: mpsc::Sender<SpanRow>,
    spool_dir: PathBuf,
}

impl SpanShipper {
    pub fn new(server_url: String, token: String, spool_dir: PathBuf) -> Arc<Self> {
        let (tx, rx) = mpsc::channel::<SpanRow>(4096);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client");
        let shipper = Arc::new(Self {
            server_url,
            token,
            http,
            tx,
            spool_dir: spool_dir.clone(),
        });
        let _ = std::fs::create_dir_all(&spool_dir);
        tokio::spawn(run(shipper.clone(), rx));
        shipper
    }

    pub fn send(&self, span: SpanRow) {
        if let Err(e) = self.tx.try_send(span) {
            warn!("span shipper channel full or closed: {e}");
        }
    }
}

async fn run(s: Arc<SpanShipper>, mut rx: mpsc::Receiver<SpanRow>) {
    let mut buf: Vec<SpanRow> = Vec::with_capacity(64);
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(span) = rx.recv() => {
                buf.push(span);
                if buf.len() >= 64 {
                    flush(&s, &mut buf).await;
                }
            }
            _ = interval.tick() => {
                flush(&s, &mut buf).await;
                replay_spool(&s).await;
            }
            else => break,
        }
    }
    if !buf.is_empty() {
        flush(&s, &mut buf).await;
    }
}

async fn flush(s: &Arc<SpanShipper>, buf: &mut Vec<SpanRow>) {
    if buf.is_empty() {
        return;
    }
    let batch = std::mem::take(buf);
    if let Err(e) = post(s, &batch).await {
        warn!("span POST failed, spooling {} spans: {e}", batch.len());
        if let Err(e) = spool_to_disk(&s.spool_dir, &batch) {
            warn!("disk spool failed: {e}");
        }
    }
}

async fn post(s: &Arc<SpanShipper>, batch: &[SpanRow]) -> anyhow::Result<()> {
    let url = format!("{}/api/v1/spans", s.server_url.trim_end_matches('/'));
    let resp = s
        .http
        .post(&url)
        .bearer_auth(&s.token)
        .json(&SpanIngestPayload { spans: batch })
        .send()
        .await?;
    if resp.status().is_success() {
        debug!("posted {} spans", batch.len());
        Ok(())
    } else if resp.status().is_server_error() {
        anyhow::bail!("server returned {}", resp.status());
    } else {
        // 4xx are likely permanent — log and drop rather than spool.
        warn!("span POST got 4xx ({}); dropping batch", resp.status());
        Ok(())
    }
}

fn spool_to_disk(dir: &std::path::Path, batch: &[SpanRow]) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("spool-{}.json", ulid::Ulid::new()));
    let mut f = std::fs::File::create(path)?;
    let bytes = serde_json::to_vec(batch)?;
    f.write_all(&bytes)?;
    Ok(())
}

async fn replay_spool(s: &Arc<SpanShipper>) {
    let entries = match std::fs::read_dir(&s.spool_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let batch: Vec<SpanRow> = match serde_json::from_slice(&bytes) {
            Ok(b) => b,
            Err(e) => {
                warn!("dropping malformed spool file {}: {e}", path.display());
                let _ = std::fs::remove_file(&path);
                continue;
            }
        };
        if post(s, &batch).await.is_ok() {
            let _ = std::fs::remove_file(&path);
            debug!(
                "replayed {} spooled spans from {}",
                batch.len(),
                path.display()
            );
        } else {
            // Server still down — leave it for next tick.
            break;
        }
    }
}

/// Tracks the current per-process `(agent_id, agent_session_id)` so
/// the proxy doesn't have to thread them through every layer.
#[derive(Default)]
pub struct SessionRegistry {
    inner: Mutex<Option<(String, String)>>,
}

impl SessionRegistry {
    pub fn set(&self, agent_id: String, session_id: String) {
        *self.inner.lock() = Some((agent_id, session_id));
    }
    pub fn get(&self) -> Option<(String, String)> {
        self.inner.lock().clone()
    }
}
