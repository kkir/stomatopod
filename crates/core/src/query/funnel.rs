use serde::{Deserialize, Serialize};
use ulid::Ulid;

use super::pageviews::{Filter, TimeRange};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunnelQuery {
    pub site_id: Ulid,
    pub range: TimeRange,
    pub steps: Vec<FunnelStep>,
    /// Maximum seconds allowed between consecutive steps (default: 86400).
    pub window_secs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunnelStep {
    pub name: String,
    pub event_name: String,
    pub filters: Vec<Filter>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FunnelResult {
    pub steps: Vec<FunnelStepResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunnelStepResult {
    pub name: String,
    pub sessions: u64,
    /// Conversion rate vs. the previous step (1.0 for the first step).
    pub conversion_rate: f64,
    pub drop_off_rate: f64,
}
