//! Shared write-ahead-log mechanics: segment rotation, bincode+zstd record
//! framing, CRC verification, and replay. The events WAL (`Wal`) is the
//! only on-disk stream today; the common helpers keep a per-stream magic
//! byte so the on-disk format stays identifiable.

use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{bail, Result};
use parking_lot::Mutex;
use serde::{de::DeserializeOwned, Serialize};
use tracing::warn;

use crate::embedded::buffer::Buffer;

pub(crate) const MAX_WAL_SIZE: u64 = 64 * 1024 * 1024;

pub(crate) struct WalInner {
    pub writer: BufWriter<File>,
    pub path: PathBuf,
    pub bytes_written: u64,
}

/// Open the first segment of a WAL directory, writing the 4-byte magic
/// header. The caller wraps the returned `Mutex<WalInner>` in its own
/// type-specific struct so on-disk formats stay distinct.
pub(crate) fn open_segment(
    dir: &Path,
    prefix: &str,
    magic: &[u8; 4],
) -> Result<(PathBuf, Mutex<WalInner>)> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{prefix}-{}.bin", ulid::Ulid::new()));
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(magic)?;
    writer.flush()?;
    let path_clone = path.clone();
    Ok((
        dir.to_path_buf(),
        Mutex::new(WalInner {
            writer,
            path: path_clone,
            bytes_written: 4,
        }),
    ))
}

/// Append a batch of records as a length-prefixed, zstd-compressed,
/// CRC-trailered block. Rotates the segment when it exceeds `MAX_WAL_SIZE`.
/// Returns the sealed (rotated-away) segment path when a rotation occurs.
pub(crate) fn append_batch<T: Serialize>(
    inner: &Mutex<WalInner>,
    dir: &Path,
    prefix: &str,
    magic: &[u8; 4],
    records: &[T],
) -> Result<Option<PathBuf>> {
    let payload = bincode::serialize(records)?;
    let compressed = zstd::encode_all(payload.as_slice(), 1)?;
    let checksum = crc32fast::hash(&compressed);
    let len = compressed.len() as u32;

    let mut guard = inner.lock();
    guard.writer.write_all(&len.to_le_bytes())?;
    guard.writer.write_all(&compressed)?;
    guard.writer.write_all(&checksum.to_le_bytes())?;
    guard.writer.flush()?;
    guard.bytes_written += 4 + compressed.len() as u64 + 4;

    let sealed = if guard.bytes_written >= MAX_WAL_SIZE {
        Some(rotate_locked(&mut guard, dir, prefix, magic)?)
    } else {
        None
    };
    Ok(sealed)
}

/// Rotate the active segment to a fresh empty file. Returns the previous
/// (now sealed) segment path so the caller can track or delete it.
pub(crate) fn rotate_locked(
    inner: &mut WalInner,
    dir: &Path,
    prefix: &str,
    magic: &[u8; 4],
) -> Result<PathBuf> {
    inner.writer.flush()?;
    let sealed = inner.path.clone();
    let new_path = dir.join(format!("{prefix}-{}.bin", ulid::Ulid::new()));
    let new_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&new_path)?;
    let mut new_writer = BufWriter::new(new_file);
    new_writer.write_all(magic)?;
    new_writer.flush()?;
    inner.writer = new_writer;
    inner.path = new_path;
    inner.bytes_written = 4;
    Ok(sealed)
}

/// Replay every segment in `dir` into `buffer`, deleting each segment after
/// successful read. `active_path` is the currently-open segment and is
/// skipped so the live writer isn't truncated out from under itself.
pub(crate) fn replay_all<T>(
    dir: &Path,
    magic: &[u8; 4],
    buffer: &Buffer<T>,
    active_path: &Path,
    label: &str,
) -> Result<usize>
where
    T: DeserializeOwned,
{
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("bin"))
        .collect();
    entries.sort();

    let mut total = 0usize;
    for path in &entries {
        if path == active_path {
            continue;
        }
        match replay_file::<T>(path, magic, buffer) {
            Ok(n) => {
                total += n;
                tracing::info!("Replayed {n} {label} from {}", path.display());
                std::fs::remove_file(path)?;
            }
            Err(e) => warn!("{label} WAL replay error for {}: {e}", path.display()),
        }
    }
    Ok(total)
}

fn replay_file<T>(path: &Path, magic: &[u8; 4], buffer: &Buffer<T>) -> Result<usize>
where
    T: DeserializeOwned,
{
    let mut file = File::open(path)?;
    let mut header = [0u8; 4];
    file.read_exact(&mut header)?;
    if &header != magic {
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
        let records: Vec<T> = bincode::deserialize(&payload)?;
        total += records.len();
        // Replay may exceed the live capacity hard cap; force-push is intentional.
        buffer.push_batch(records);
    }
    Ok(total)
}
