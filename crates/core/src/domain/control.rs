use serde::{Deserialize, Serialize};

/// Server-issued command delivered to a Sentinel sidecar over SSE.
///
/// Each command carries a monotonic `seq` so the sidecar can dedupe
/// after a reconnect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlEnvelope {
    pub seq: u64,
    pub agent_id: String,
    #[serde(flatten)]
    pub command: ControlCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ControlCommand {
    /// Immediately reject the next outbound request. The sidecar returns
    /// a synthesized terminal response so the agent loop exits cleanly.
    Kill { reason: String },
    /// Inject a system message into the next outbound request.
    Hint { message: String },
    /// Lift a previous Kill.
    Resume,
}

impl ControlCommand {
    pub fn kind_str(&self) -> &'static str {
        match self {
            ControlCommand::Kill { .. } => "kill",
            ControlCommand::Hint { .. } => "hint",
            ControlCommand::Resume => "resume",
        }
    }
}
