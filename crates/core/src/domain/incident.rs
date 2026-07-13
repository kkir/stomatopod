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
    /// Wire JSON for webhooks still uses the key `agent_id` for compatibility.
    pub source: String,
    pub trigger: IncidentTrigger,
    pub opened_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IncidentTrigger {
    /// An analytics alert condition fired (traffic spike/drop, referrer spike).
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
