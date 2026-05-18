use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub mode: Mode,
    pub listen: ListenConfig,
    pub storage: StorageConfig,
    pub geo: GeoConfig,
    pub auth: AuthConfig,
    pub limits: LimitsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::default(),
            listen: ListenConfig::default(),
            storage: StorageConfig::default(),
            geo: GeoConfig::default(),
            auth: AuthConfig::default(),
            limits: LimitsConfig::default(),
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
}

impl Default for EmbeddedConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("./data"),
            wal_fsync_interval_ms: 200,
            parquet_flush_rows: 50_000,
            parquet_flush_interval_s: 30,
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
