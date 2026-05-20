use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{bail, Result};
use parking_lot::Mutex;
use stomatopod_core::domain::agent_span::AgentSpan;
use tracing::{info, warn};

use super::buffer::SpanBuffer;

const MAGIC: &[u8; 4] = b"SWL!";
const MAX_WAL_SIZE: u64 = 64 * 1024 * 1024; // 64MB

/// Span-side WAL. Separate from the event WAL so spans (10-100x the
/// pageview rate) don't serialize through the analytics mutex, and so
/// span retention can diverge from event retention.
pub struct SpanWal {
    dir: PathBuf,
    inner: Mutex<WalInner>,
}

struct WalInner {
    writer: BufWriter<File>,
    path: PathBuf,
    bytes_written: u64,
}

impl SpanWal {
    pub fn open(dir: &Path) -> Result<Arc<Self>> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("swal-{}.bin", ulid::Ulid::new()));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(MAGIC)?;
        writer.flush()?;

        Ok(Arc::new(Self {
            dir: dir.to_path_buf(),
            inner: Mutex::new(WalInner {
                writer,
                path,
                bytes_written: 4,
            }),
        }))
    }

    pub fn append(&self, spans: &[AgentSpan]) -> Result<()> {
        let payload = bincode::serialize(spans)?;
        let compressed = zstd::encode_all(payload.as_slice(), 1)?;
        let checksum = crc32fast::hash(&compressed);
        let len = compressed.len() as u32;

        let mut inner = self.inner.lock();
        inner.writer.write_all(&len.to_le_bytes())?;
        inner.writer.write_all(&compressed)?;
        inner.writer.write_all(&checksum.to_le_bytes())?;
        inner.writer.flush()?;
        inner.bytes_written += 4 + compressed.len() as u64 + 4;

        if inner.bytes_written >= MAX_WAL_SIZE {
            self.rotate_locked(&mut inner)?;
        }
        Ok(())
    }

    pub fn replay(&self, buffer: &SpanBuffer) -> Result<()> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&self.dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("bin"))
            .collect();
        entries.sort();

        let mut total = 0usize;
        for path in &entries {
            // Skip the currently-active file so we don't truncate-replay it.
            if *path == self.inner.lock().path {
                continue;
            }
            match Self::replay_file(path, buffer) {
                Ok(n) => {
                    total += n;
                    info!("Replayed {} spans from {}", n, path.display());
                    std::fs::remove_file(path)?;
                }
                Err(e) => warn!("Span WAL replay error for {}: {e}", path.display()),
            }
        }
        info!("Span WAL replay complete: {} total spans", total);
        Ok(())
    }

    fn replay_file(path: &Path, buffer: &SpanBuffer) -> Result<usize> {
        let mut file = File::open(path)?;
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)?;
        if &magic != MAGIC {
            bail!("bad span WAL magic in {}", path.display());
        }

        let mut total = 0;
        loop {
            let mut len_buf = [0u8; 4];
            if file.read_exact(&mut len_buf).is_err() {
                break;
            }
            let len = u32::from_le_bytes(len_buf) as usize;

            let mut compressed = vec![0u8; len];
            if file.read_exact(&mut compressed).is_err() {
                warn!("truncated span WAL record in {}", path.display());
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
            let spans: Vec<AgentSpan> = bincode::deserialize(&payload)?;
            total += spans.len();
            buffer.push_batch(spans);
        }
        Ok(total)
    }

    fn rotate_locked(&self, inner: &mut WalInner) -> Result<()> {
        inner.writer.flush()?;
        let new_path = self.dir.join(format!("swal-{}.bin", ulid::Ulid::new()));
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
}
