use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub id: Ulid,
    pub org_id: Ulid,
    /// Primary domain without scheme, e.g. "example.com".
    pub domain: String,
    pub name: String,
    /// IANA timezone, e.g. "America/New_York". Used for session boundary.
    pub timezone: String,
    /// Write-only key embedded in the tracking snippet.
    pub public_key: String,
    pub created_at: DateTime<Utc>,
    pub is_active: bool,
}
