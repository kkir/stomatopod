use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Metadata row for a Sentinel-monitored agent. Created lazily on the
/// first span ingest for a given (site_id, agent_id) pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: Ulid,
    pub site_id: Ulid,
    /// Stable identifier reported by the sidecar.
    pub agent_id: String,
    pub name: String,
    pub policy_id: Option<Ulid>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

/// Bearer token used by a sidecar to authenticate to /api/v1/spans and
/// /api/v1/sentinel/stream. Stored as a BLAKE3 hash; verification is on
/// the hot path so argon2 is overkill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentinelToken {
    pub id: Ulid,
    pub site_id: Ulid,
    pub name: String,
    /// Hex-encoded BLAKE3 hash of the token.
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertChannel {
    pub id: Ulid,
    pub site_id: Ulid,
    pub kind: AlertChannelKind,
    /// Destination URL. For webhooks/Slack this is the POST target.
    pub url: String,
    /// Optional shared secret used to sign outgoing payloads.
    pub secret: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_error_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertChannelKind {
    Webhook,
    Slack,
}

impl AlertChannelKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertChannelKind::Webhook => "webhook",
            AlertChannelKind::Slack => "slack",
        }
    }
}
