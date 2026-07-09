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
    wal_common::{append_batch, open_segment, replay_all, rotate_locked, WalInner},
};

const MAGIC: &[u8; 4] = b"WAL!";
const PREFIX: &str = "wal";

pub struct Wal {
    dir: PathBuf,
    fsync_interval: Duration,
    inner: Mutex<WalInner>,
    /// Paths of segments rotated away from the active writer. Deleted by
    /// [`Self::reclaim_sealed`] after a successful Parquet flush.
    sealed: Mutex<Vec<PathBuf>>,
}

impl Wal {
    pub fn open(dir: &Path, fsync_interval_ms: u64) -> Result<Arc<Self>> {
        let (dir, inner) = open_segment(dir, PREFIX, MAGIC)?;
        Ok(Arc::new(Self {
            dir,
            fsync_interval: Duration::from_millis(fsync_interval_ms),
            inner,
            sealed: Mutex::new(Vec::new()),
        }))
    }

    pub fn fsync_interval(&self) -> Duration {
        self.fsync_interval
    }

    /// Append a batch of events to the WAL. Called by the flush worker,
    /// not the hot ingest path (which writes to the in-memory buffer first).
    pub fn append(&self, events: &[Event]) -> Result<()> {
        if let Some(path) = append_batch(&self.inner, &self.dir, PREFIX, MAGIC, events)? {
            self.sealed.lock().push(path);
        }
        Ok(())
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

    /// Delete sealed (rotated-away) segments and, because the caller only
    /// invokes this when the in-memory buffer is empty, also replace the
    /// active segment with a fresh empty file so unreclaimed WAL data is
    /// only ever for events still buffered.
    pub fn reclaim_sealed(&self) -> Result<()> {
        let sealed: Vec<PathBuf> = std::mem::take(&mut *self.sealed.lock());
        for path in &sealed {
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }

        // Buffer is empty: rotate the active segment and drop the old one.
        let mut guard = self.inner.lock();
        let old_active = rotate_locked(&mut guard, &self.dir, PREFIX, MAGIC)?;
        drop(guard);
        if old_active.exists() {
            std::fs::remove_file(&old_active)?;
        }
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
