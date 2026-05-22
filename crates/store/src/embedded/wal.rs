use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::Result;
use parking_lot::Mutex;
use stomatopod_core::domain::event::Event;
use tracing::info;

use super::{
    buffer::EventBuffer,
    wal_common::{append_batch, open_segment, replay_all, WalInner},
};

const MAGIC: &[u8; 4] = b"WAL!";
const PREFIX: &str = "wal";

pub struct Wal {
    dir: PathBuf,
    #[allow(dead_code)]
    fsync_interval: Duration,
    inner: Mutex<WalInner>,
}

impl Wal {
    pub fn open(dir: &Path, fsync_interval_ms: u64) -> Result<Arc<Self>> {
        let (dir, inner) = open_segment(dir, PREFIX, MAGIC)?;
        Ok(Arc::new(Self {
            dir,
            fsync_interval: Duration::from_millis(fsync_interval_ms),
            inner,
        }))
    }

    /// Append a batch of events to the WAL. Called by the flush worker,
    /// not the hot ingest path (which writes to the in-memory buffer first).
    pub fn append(&self, events: &[Event]) -> Result<()> {
        append_batch(&self.inner, &self.dir, PREFIX, MAGIC, events)
    }

    /// Sync the underlying file to disk.
    pub fn fsync(&self) -> Result<()> {
        let inner = self.inner.lock();
        inner.writer.get_ref().sync_all()?;
        Ok(())
    }

    /// Replay all WAL files on startup, loading events into the buffer.
    pub fn replay(&self, buffer: &EventBuffer) -> Result<()> {
        let active = self.inner.lock().path.clone();
        let total = replay_all(&self.dir, MAGIC, buffer, &active, "events")?;
        info!("WAL replay complete: {total} total events");
        Ok(())
    }

    /// Delete a specific WAL file after successful Parquet flush.
    pub fn delete_file(&self, path: &Path) -> Result<()> {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
