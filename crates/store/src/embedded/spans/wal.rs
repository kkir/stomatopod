use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use parking_lot::Mutex;
use stomatopod_core::domain::agent_span::AgentSpan;
use tracing::info;

use super::buffer::SpanBuffer;
use crate::embedded::wal_common::{append_batch, open_segment, replay_all, WalInner};

/// Span WAL magic. Distinct from the events WAL's `WAL!` so a misplaced
/// segment file is rejected at replay rather than silently misinterpreted.
const MAGIC: &[u8; 4] = b"SWL!";
const PREFIX: &str = "swal";

/// Span-side WAL. Separate from the event WAL so spans (10-100x the
/// pageview rate) don't serialize through the analytics mutex, and so
/// span retention can diverge from event retention.
pub struct SpanWal {
    dir: PathBuf,
    inner: Mutex<WalInner>,
}

impl SpanWal {
    pub fn open(dir: &Path) -> Result<Arc<Self>> {
        let (dir, inner) = open_segment(dir, PREFIX, MAGIC)?;
        Ok(Arc::new(Self { dir, inner }))
    }

    pub fn append(&self, spans: &[AgentSpan]) -> Result<()> {
        append_batch(&self.inner, &self.dir, PREFIX, MAGIC, spans)
    }

    pub fn replay(&self, buffer: &SpanBuffer) -> Result<()> {
        let active = self.inner.lock().path.clone();
        let total = replay_all(&self.dir, MAGIC, buffer, &active, "spans")?;
        info!("Span WAL replay complete: {total} total spans");
        Ok(())
    }
}
