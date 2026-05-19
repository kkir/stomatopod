use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Per-site policy applied by the sidecar's local heuristics engine and
/// by the server-side incident detector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub id: Ulid,
    pub site_id: Ulid,
    /// Trip if the same tool_input_hash appears this many times in the
    /// repetition window. None disables the check.
    pub repetition_max: Option<u32>,
    /// Trip if output tokens-per-second exceeds this value over the
    /// velocity window. None disables the check.
    pub velocity_max_tps: Option<f64>,
    /// Trip if a single agent session burns more than this many USD.
    pub cost_cap_usd: Option<f64>,
    /// Template for system-message injection on Hint actions.
    pub hint_template: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Policy {
    /// Conservative defaults suitable for most agent workloads.
    pub fn default_for_site(site_id: Ulid) -> Self {
        Self {
            id: Ulid::new(),
            site_id,
            repetition_max: Some(5),
            velocity_max_tps: Some(2000.0),
            cost_cap_usd: Some(10.0),
            hint_template: None,
            created_at: Utc::now(),
        }
    }
}
