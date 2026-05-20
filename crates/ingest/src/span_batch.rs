use std::{sync::Arc, time::Duration};

use tokio::sync::mpsc;
use tracing::error;

use stomatopod_core::{domain::agent_span::AgentSpan, traits::AgentStore};

/// Mirror of `run_batcher` for agent spans. Accumulates spans from the
/// ingest handler and flushes to the `AgentStore` either when the batch
/// fills or on a timer. The channel carries whole request batches so
/// the handler can enqueue atomically.
pub async fn run_span_batcher(
    mut rx: mpsc::Receiver<Vec<AgentSpan>>,
    store: Arc<dyn AgentStore>,
    batch_size: usize,
    flush_interval_ms: u64,
) {
    let mut buf: Vec<AgentSpan> = Vec::with_capacity(batch_size);
    let mut interval = tokio::time::interval(Duration::from_millis(flush_interval_ms));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(spans) = rx.recv() => {
                buf.extend(spans);
                if buf.len() >= batch_size {
                    flush(&mut buf, &store).await;
                }
            }
            _ = interval.tick() => {
                if !buf.is_empty() {
                    flush(&mut buf, &store).await;
                }
            }
            else => break,
        }
    }

    if !buf.is_empty() {
        flush(&mut buf, &store).await;
    }
}

async fn flush(buf: &mut Vec<AgentSpan>, store: &Arc<dyn AgentStore>) {
    let batch = std::mem::take(buf);
    if let Err(e) = store.ingest_spans(batch).await {
        error!("Span batch flush error: {e}");
    }
}
