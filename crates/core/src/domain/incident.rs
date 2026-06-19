use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incident {
    pub id: Ulid,
    pub site_id: Ulid,
    pub agent_id: String,
    pub trigger: IncidentTrigger,
    pub status: IncidentStatus,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IncidentTrigger {
    /// Same tool arguments observed N consecutive times.
    Repetition { count: u32, args_hash: String },
    /// Output token rate exceeded the configured threshold.
    TokenVelocity { tokens_per_sec: f64 },
    /// Session-level cost cap exceeded.
    CostThreshold { usd: f64 },
    /// An analytics alert condition fired (traffic spike/drop, goal
    /// threshold, referrer spike). `alert_type` is the alert kind token,
    /// `value` the observed metric, `threshold` the configured trigger.
    AnalyticsAlert {
        alert_type: String,
        value: f64,
        threshold: f64,
    },
    /// Operator clicked the Kill button.
    Manual,
}

impl IncidentTrigger {
    pub fn kind_str(&self) -> &'static str {
        match self {
            IncidentTrigger::Repetition { .. } => "repetition",
            IncidentTrigger::TokenVelocity { .. } => "token_velocity",
            IncidentTrigger::CostThreshold { .. } => "cost_threshold",
            IncidentTrigger::AnalyticsAlert { .. } => "analytics_alert",
            IncidentTrigger::Manual => "manual",
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
