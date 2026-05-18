use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// The atomic unit of analytics data. Pageviews and custom events share
/// this struct; `kind` discriminates them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Monotonically sortable unique ID (ULID = timestamp + randomness).
    pub id: Ulid,
    /// Which site produced this event — present on every row for multi-tenancy.
    pub site_id: Ulid,
    /// Event name: "pageview" or a custom name like "purchase".
    pub name: String,
    pub kind: EventKind,
    /// Wall-clock time reported by the client.
    pub timestamp: DateTime<Utc>,
    /// Server-side receipt time, used for dedup lag detection.
    pub received_at: DateTime<Utc>,

    // ---- Page context ----
    pub url: String,
    pub referrer: Option<String>,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub utm_term: Option<String>,
    pub utm_content: Option<String>,

    // ---- Device ----
    pub browser: String,
    pub browser_version: String,
    pub os: String,
    pub os_version: String,
    pub device_type: DeviceType,
    pub screen_width: Option<u16>,
    pub screen_height: Option<u16>,
    pub language: Option<String>,

    // ---- Network / Geo ----
    /// IPv4: last octet zeroed. IPv6: last 80 bits zeroed.
    pub ip_anonymized: String,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,

    // ---- Session ----
    /// Cookieless: BLAKE3(site_id || ip_anon || ua || utc_day)[..16]
    pub session_id: [u8; 16],

    // ---- Custom event payload ----
    /// JSON object for custom events; None for pageviews.
    pub properties: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Pageview,
    Custom,
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventKind::Pageview => write!(f, "pageview"),
            EventKind::Custom => write!(f, "custom"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    Desktop,
    Mobile,
    Tablet,
    #[default]
    Unknown,
}

impl std::fmt::Display for DeviceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceType::Desktop => write!(f, "desktop"),
            DeviceType::Mobile => write!(f, "mobile"),
            DeviceType::Tablet => write!(f, "tablet"),
            DeviceType::Unknown => write!(f, "unknown"),
        }
    }
}
