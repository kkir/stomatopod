use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Outbound alert payload delivered to webhook / Slack / Telegram sinks.
/// Built in-memory by the analytics alert evaluator (not persisted).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incident {
    pub id: Ulid,
    pub site_id: Ulid,
    /// Free-form source label (e.g. `"analytics"`, `"test"`).
    pub agent_id: String,
    pub trigger: IncidentTrigger,
    pub status: IncidentStatus,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IncidentTrigger {
    /// An analytics alert condition fired (traffic spike/drop, goal
    /// threshold, referrer spike). `alert_type` is the alert kind token,
    /// `value` the observed metric, `threshold` the configured trigger.
    AnalyticsAlert {
        alert_type: String,
        value: f64,
        threshold: f64,
    },
}

impl IncidentTrigger {
    pub fn kind_str(&self) -> &'static str {
        match self {
            IncidentTrigger::AnalyticsAlert { .. } => "analytics_alert",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentStatus {
    Open,
    Acknowledged,
    Resolved,
}

impl IncidentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            IncidentStatus::Open => "open",
            IncidentStatus::Acknowledged => "acknowledged",
            IncidentStatus::Resolved => "resolved",
        }
    }
}
