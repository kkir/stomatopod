use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanQuery {
    pub site_id: Ulid,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub since: DateTime<Utc>,
    pub until: DateTime<Utc>,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanRow {
    pub id: String,
    pub agent_id: String,
    pub agent_session_id: String,
    pub kind: String,
    pub model: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
    pub tool_name: Option<String>,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSummary {
    pub agent_id: String,
    pub last_seen_at: DateTime<Utc>,
    pub total_spans: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cost_usd: f64,
}
