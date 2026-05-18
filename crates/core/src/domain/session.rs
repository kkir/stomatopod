use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use super::event::DeviceType;

/// Derived at query time by grouping events on session_id within a 30-minute
/// inactivity gap. Not stored directly — computed from the events table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: [u8; 16],
    pub site_id: Ulid,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration_secs: u32,
    pub pageview_count: u32,
    pub entry_url: String,
    pub exit_url: String,
    pub referrer: Option<String>,
    pub country_code: Option<String>,
    pub device_type: DeviceType,
    pub browser: String,
    pub os: String,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    /// True when the session has a single pageview with duration < 30s.
    pub is_bounce: bool,
}
