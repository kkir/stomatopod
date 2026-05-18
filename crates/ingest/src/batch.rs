use std::{sync::Arc, time::Duration};

use tokio::sync::mpsc;
use tracing::error;

use stomatopod_core::{domain::event::Event, traits::StorageBackend};

/// Accumulates events from the ingest handler and flushes them to the
/// storage backend in batches, either when full or on a timer.
pub async fn run_batcher(
    mut rx: mpsc::Receiver<Event>,
    backend: Arc<dyn StorageBackend>,
    batch_size: usize,
    flush_interval_ms: u64,
) {
    let mut buf: Vec<Event> = Vec::with_capacity(batch_size);
    let mut interval = tokio::time::interval(Duration::from_millis(flush_interval_ms));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(ev) = rx.recv() => {
                buf.push(ev);
                if buf.len() >= batch_size {
                    flush(&mut buf, &backend).await;
                }
            }
            _ = interval.tick() => {
                if !buf.is_empty() {
                    flush(&mut buf, &backend).await;
                }
            }
            else => break,
        }
    }

    // Final flush on shutdown
    if !buf.is_empty() {
        flush(&mut buf, &backend).await;
    }
}

async fn flush(buf: &mut Vec<Event>, backend: &Arc<dyn StorageBackend>) {
    let batch = std::mem::take(buf);
    if let Err(e) = backend.ingest_events(batch).await {
        error!("Batch flush error: {e}");
    }
}
