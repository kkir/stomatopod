use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Config {
    pub mode: Mode,
    pub listen: ListenConfig,
    pub storage: StorageConfig,
    pub geo: GeoConfig,
    pub auth: AuthConfig,
    pub limits: LimitsConfig,
    pub sentinel: SentinelConfig,
    pub email: EmailConfig,
    /// Public base URL used to build share-link and digest URLs in emails
    /// and the share UI, e.g. `https://analytics.example.com`. No trailing
    /// slash. Defaults to `http://localhost:8080`.
    pub base_url: String,
}

/// Email/transactional-send settings for digests. When `provider` is
/// `none` (the default) digests are rendered and logged but not sent — the
/// scheduler still runs, which keeps self-hosted deployments side-effect
/// free until an operator wires up a provider.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct EmailConfig {
    /// `none`, `smtp`, `postmark`, or `resend`.
    pub provider: String,
    pub api_key: Option<String>,
    /// From address, e.g. `analytics@yourdomain.com`.
    pub from: String,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            provider: "none".into(),
            api_key: None,
            from: "analytics@localhost".into(),
        }
    }
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

/// AI firewall server-side settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct SentinelConfig {
    /// Object keys whose values are stripped from span `properties`
    /// before storage. Case-insensitive.
    pub redact_keys: Vec<String>,
}

impl Default for SentinelConfig {
    fn default() -> Self {
        Self {
            redact_keys: vec![
                "authorization".into(),
                "api_key".into(),
                "apikey".into(),
                "password".into(),
                "secret".into(),
                "token".into(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Single org, auto-created on first boot. No plan limits enforced.
    #[default]
    SelfHosted,
    /// Multi-org. Requires explicit org creation; plan limits enforced.
    Saas,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ListenConfig {
    pub host: String,
    pub port: u16,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
}

impl Default for ListenConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 8080,
            tls_cert: None,
            tls_key: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum StorageConfig {
    Embedded(EmbeddedConfig),
    Postgres(PostgresConfig),
    Clickhouse(ClickhouseConfig),
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self::Embedded(EmbeddedConfig::default())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct EmbeddedConfig {
    pub data_dir: PathBuf,
    pub wal_fsync_interval_ms: u64,
    pub parquet_flush_rows: usize,
    pub parquet_flush_interval_s: u64,
    /// When false (the default), the server refuses to start if it detects it
    /// is running inside a container with `data_dir` on ephemeral container
    /// storage rather than a mounted volume — otherwise all analytics data is
    /// silently lost on the next redeploy. Set to true (or
    /// `STOMATOPOD_STORAGE__ALLOW_EPHEMERAL=true`) only for throwaway demos and
    /// ephemeral test containers.
    pub allow_ephemeral: bool,
}

impl Default for EmbeddedConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("./data"),
            wal_fsync_interval_ms: 200,
            parquet_flush_rows: 50_000,
            parquet_flush_interval_s: 30,
            allow_ephemeral: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PostgresConfig {
    pub url: String,
    #[serde(default = "default_pg_max_connections")]
    pub max_connections: u32,
}

fn default_pg_max_connections() -> u32 {
    20
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClickhouseConfig {
    pub url: String,
    #[serde(default = "default_ch_database")]
    pub database: String,
    pub username: String,
    pub password: String,
}

fn default_ch_database() -> String {
    "stomatopod".into()
}

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
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            secret_key: String::new(),
            session_ttl_s: 86400 * 30,
        }
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
            ingest_channel_size: 65_536,
            ingest_batch_size: 1_000,
            ingest_flush_interval_ms: 100,
            max_events_per_request: 10,
        }
    }
}
