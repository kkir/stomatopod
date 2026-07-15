use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Config {
    pub listen: ListenConfig,
    pub storage: StorageConfig,
    pub geo: GeoConfig,
    pub auth: AuthConfig,
    pub limits: LimitsConfig,
    /// Public base URL used to build dashboard links in digests and cookies,
    /// e.g. `https://analytics.example.com`. No trailing slash. Defaults to
    /// `http://localhost:8080`.
    pub base_url: String,
}

impl Config {
    /// Public base URL with any trailing slash trimmed, falling back to
    /// `http://localhost:8080` when unset.
    pub fn public_base_url(&self) -> &str {
        let trimmed = self.base_url.trim_end_matches('/');
        if trimmed.is_empty() {
            "http://localhost:8080"
        } else {
            trimmed
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ListenConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ListenConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 8080,
        }
    }
}

/// Embedded storage settings (SQLite metadata, WAL, Parquet under `data_dir`).
///
/// Unknown keys such as a legacy `backend = "embedded"` field are ignored so
/// existing `stomatopod.toml` files keep working.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct StorageConfig {
    pub data_dir: PathBuf,
    pub wal_fsync_interval_ms: u64,
    pub parquet_flush_rows: usize,
    pub parquet_flush_interval_s: u64,
    /// When false (the default), the server refuses to start if it detects it
    /// is running inside a container with `data_dir` on ephemeral container
    /// storage rather than a mounted volume - otherwise all analytics data is
    /// silently lost on the next redeploy. Set to true (or
    /// `STOMATOPOD_STORAGE__ALLOW_EPHEMERAL=true`) only for throwaway demos and
    /// ephemeral test containers.
    pub allow_ephemeral: bool,
    /// Drop event data older than this many days. `0` (default) keeps forever.
    /// Deletes Hive `date=` Parquet partitions older than the cutoff.
    pub retention_days: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("./data"),
            wal_fsync_interval_ms: 200,
            parquet_flush_rows: 50_000,
            parquet_flush_interval_s: 30,
            allow_ephemeral: false,
            retention_days: 0,
        }
    }
}

/// Alias kept for call sites that historically took `EmbeddedConfig`.
pub type EmbeddedConfig = StorageConfig;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct GeoConfig {
    pub mmdb_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AuthConfig {
    pub secret_key: String,
    pub session_ttl_s: u64,
    /// When set, forces the session cookie `Secure` flag. When `None`
    /// (default), `Secure` is enabled iff `base_url` starts with `https://`.
    pub cookie_secure: Option<bool>,
    /// When true (default), ingest trusts `CF-Connecting-IP` / `X-Real-IP` /
    /// `X-Forwarded-For` for client IP. Set false when the process is exposed
    /// directly to the internet without a reverse proxy that strips those
    /// headers, so clients cannot spoof geo/session derivation.
    pub trust_forwarded_headers: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            secret_key: String::new(),
            session_ttl_s: 86400 * 30,
            cookie_secure: None,
            trust_forwarded_headers: true,
        }
    }
}

impl AuthConfig {
    /// Effective session-cookie Secure flag given the public base URL.
    pub fn effective_cookie_secure(&self, base_url: &str) -> bool {
        self.cookie_secure
            .unwrap_or_else(|| base_url.starts_with("https://"))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LimitsConfig {
    pub ingest_channel_size: usize,
    pub ingest_batch_size: usize,
    pub ingest_flush_interval_ms: u64,
    pub max_events_per_request: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            ingest_channel_size: 8_192,
            ingest_batch_size: 1_000,
            ingest_flush_interval_ms: 100,
            max_events_per_request: 10,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_reads_data_dir_when_backend_key_present() {
        let cfg: Config = serde_json::from_value(serde_json::json!({
            "storage": { "backend": "embedded", "data_dir": "/data" }
        }))
        .unwrap();
        assert_eq!(cfg.storage.data_dir, PathBuf::from("/data"));
    }

    #[test]
    fn storage_defaults_when_fully_absent() {
        let cfg: Config = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(cfg.storage.data_dir, PathBuf::from("./data"));
    }

    #[test]
    fn storage_reads_partial_keys() {
        let cfg: Config = serde_json::from_value(serde_json::json!({
            "storage": { "data_dir": "/data", "retention_days": 90 }
        }))
        .unwrap();
        assert_eq!(cfg.storage.data_dir, PathBuf::from("/data"));
        assert_eq!(cfg.storage.retention_days, 90);
    }
}
