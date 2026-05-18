use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{bail, Result};
use parking_lot::Mutex;
use stomatopod_core::domain::event::Event;
use tracing::{info, warn};

use super::buffer::EventBuffer;

const MAGIC: &[u8; 4] = b"WAL!";
const MAX_WAL_SIZE: u64 = 64 * 1024 * 1024; // 64MB

pub struct Wal {
    dir: PathBuf,
    #[allow(dead_code)]
    fsync_interval: Duration,
    inner: Mutex<WalInner>,
}

struct WalInner {
    writer: BufWriter<File>,
    path: PathBuf,
    bytes_written: u64,
}

impl Wal {
    pub fn open(dir: &Path, fsync_interval_ms: u64) -> Result<Arc<Self>> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("wal-{}.bin", ulid::Ulid::new()));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(MAGIC)?;
        writer.flush()?;

        Ok(Arc::new(Self {
            dir: dir.to_path_buf(),
            fsync_interval: Duration::from_millis(fsync_interval_ms),
            inner: Mutex::new(WalInner {
                writer,
                path,
                bytes_written: 4,
            }),
        }))
    }

    /// Append a batch of events to the WAL. This is called by the flush worker,
    /// not the hot ingest path (which writes to the in-memory buffer first).
    pub fn append(&self, events: &[Event]) -> Result<()> {
        let payload = bincode::serialize(events)?;
        let compressed = zstd::encode_all(payload.as_slice(), 1)?;
        let checksum = crc32fast::hash(&compressed);
        let len = compressed.len() as u32;

        let mut inner = self.inner.lock();
        inner.writer.write_all(&len.to_le_bytes())?;
        inner.writer.write_all(&compressed)?;
        inner.writer.write_all(&checksum.to_le_bytes())?;
        inner.writer.flush()?;
        inner.bytes_written += 4 + compressed.len() as u64 + 4;

        // Rotate if oversized
        if inner.bytes_written >= MAX_WAL_SIZE {
            self.rotate_locked(&mut inner)?;
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
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&self.dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("bin"))
            .collect();
        entries.sort();

        let mut total = 0usize;
        for path in &entries {
            match Self::replay_file(path, buffer) {
                Ok(n) => {
                    total += n;
                    info!("Replayed {} events from {}", n, path.display());
                    std::fs::remove_file(path)?;
                }
                Err(e) => {
                    warn!("WAL replay error for {}: {e}", path.display());
                }
            }
        }
        info!("WAL replay complete: {} total events", total);
        Ok(())
    }

    fn replay_file(path: &Path, buffer: &EventBuffer) -> Result<usize> {
        let mut file = File::open(path)?;
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)?;
        if &magic != MAGIC {
            bail!("bad WAL magic in {}", path.display());
        }

        let mut total = 0;
        loop {
            let mut len_buf = [0u8; 4];
            if file.read_exact(&mut len_buf).is_err() {
                break; // clean EOF
            }
            let len = u32::from_le_bytes(len_buf) as usize;

            let mut compressed = vec![0u8; len];
            if file.read_exact(&mut compressed).is_err() {
                warn!("truncated WAL record in {}", path.display());
                break;
            }

            let mut crc_buf = [0u8; 4];
            if file.read_exact(&mut crc_buf).is_err() {
                warn!("missing CRC in {}", path.display());
                break;
            }
            let expected_crc = u32::from_le_bytes(crc_buf);
            let actual_crc = crc32fast::hash(&compressed);
            if actual_crc != expected_crc {
                warn!("CRC mismatch in {}", path.display());
                break;
            }

            let payload = zstd::decode_all(compressed.as_slice())?;
            let events: Vec<Event> = bincode::deserialize(&payload)?;
            total += events.len();
            buffer.push_batch(events);
        }
        Ok(total)
    }

    fn rotate_locked(&self, inner: &mut WalInner) -> Result<()> {
        inner.writer.flush()?;
        let new_path = self.dir.join(format!("wal-{}.bin", ulid::Ulid::new()));
        let new_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&new_path)?;
        let mut new_writer = BufWriter::new(new_file);
        new_writer.write_all(MAGIC)?;
        new_writer.flush()?;
        inner.writer = new_writer;
        inner.path = new_path;
        inner.bytes_written = 4;
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
