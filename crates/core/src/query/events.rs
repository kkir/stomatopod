use serde::{Deserialize, Serialize};
use ulid::Ulid;

use super::pageviews::{Filter, TimeRange, TopList};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventQuery {
    pub site_id: Ulid,
    pub range: TimeRange,
    /// Filter to a specific event name; None returns all custom events.
    pub event_name: Option<String>,
    pub filters: Vec<Filter>,
    pub limit: u32,
}

pub type EventQueryResult = TopList;
