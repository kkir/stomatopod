use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// One unit of agent execution observed by the Sentinel sidecar.
///
/// A Request span wraps an outbound LLM call; ToolCall and Reasoning
/// spans are emitted as children with `parent_span_id` set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpan {
    pub id: Ulid,
    pub site_id: Ulid,
    /// Stable identifier for the agent process. Supplied by the sidecar
    /// (e.g. hostname + PID, or a user-supplied tag).
    pub agent_id: String,
    /// Logical conversation identifier. Distinct from `Event.session_id`
    /// (which is a 16-byte hash); agent sessions are vendor-supplied strings.
    pub agent_session_id: String,
    pub parent_span_id: Option<Ulid>,
    pub kind: SpanKind,
    pub model: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,

    // ---- Token usage (deterministic, parsed from API headers/body) ----
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
    /// Cost in USD, computed locally from a model price table. Never
    /// trusts LLM self-report.
    pub cost_usd: f64,

    // ---- Tool-call metadata (only set when kind == ToolCall) ----
    pub tool_name: Option<String>,
    /// BLAKE3 hash of the canonicalized tool arguments. Used by the
    /// sidecar's repetition heuristic; raw args are redacted before send.
    pub tool_input_hash: Option<String>,

    pub stop_reason: Option<String>,
    /// Free-form metadata for forward-compat (e.g. OTel attributes).
    pub properties: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanKind {
    Request,
    ToolCall,
    Reasoning,
}

impl SpanKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpanKind::Request => "request",
            SpanKind::ToolCall => "tool_call",
            SpanKind::Reasoning => "reasoning",
        }
    }
}

impl std::fmt::Display for SpanKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
