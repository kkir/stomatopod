//! Tier-2 analytics query types: entry/exit pages, the real-time view,
//! goal conversion stats, and the raw session/event export rows.
//!
//! These mirror the shape of the JSON the API returns so handlers can
//! serialize them directly. The actual SQL lives in each storage backend.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use super::pageviews::{Filter, Granularity, TimeRange};

// ---- Entry / exit pages ----

/// Top entry pages: where sessions begin. `bounce_rate` is the share of
/// sessions starting on this page that consisted of a single pageview.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EntryPages {
    pub rows: Vec<EntryPageRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryPageRow {
    pub url: String,
    pub sessions: u64,
    pub pct: f64,
    pub bounce_rate: f64,
}

/// Top exit pages: where sessions end. `exit_rate` is exits divided by
/// total pageviews on that page.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExitPages {
    pub rows: Vec<ExitPageRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitPageRow {
    pub url: String,
    pub exits: u64,
    pub pct: f64,
    pub exit_rate: f64,
}

// ---- Real-time view ----

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RealtimeSnapshot {
    pub active_sessions: u64,
    pub pageviews_per_minute: f64,
    pub top_pages: Vec<RealtimeTopPage>,
    pub recent_events: Vec<RealtimeEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealtimeTopPage {
    pub url: String,
    pub active_sessions: u64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealtimeEvent {
    pub name: String,
    pub url: String,
    pub seconds_ago: i64,
    pub properties: serde_json::Value,
}

// ---- Goals ----

/// Parameters for a goal-conversion query. `event_name` + `filters` match
/// the goal's target event; `range`/`granularity` shape the timeseries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalQuery {
    pub site_id: Ulid,
    pub event_name: String,
    pub filters: Vec<Filter>,
    pub range: TimeRange,
    pub granularity: Granularity,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GoalStats {
    pub completions: u64,
    pub unique_completions: u64,
    pub conversion_rate: f64,
    pub timeseries: Vec<GoalBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalBucket {
    pub date: String,
    pub completions: u64,
    pub conversion_rate: f64,
}

// ---- Export rows ----

/// A single derived session row, used by the sessions export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub session_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration_secs: i64,
    pub pageviews: u64,
    pub entry_url: String,
    pub exit_url: String,
    pub referrer: Option<String>,
    pub country_code: Option<String>,
    pub browser: String,
    pub os: String,
    pub device_type: String,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub is_bounce: bool,
}

/// A single raw event row, used by the events export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEventRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub timestamp: DateTime<Utc>,
    pub url: String,
    pub referrer: Option<String>,
    pub country_code: Option<String>,
    pub browser: String,
    pub os: String,
    pub device_type: String,
    pub properties: Option<String>,
}
