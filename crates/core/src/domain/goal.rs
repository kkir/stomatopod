use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::query::pageviews::Filter;

/// A conversion goal: a named custom event (with optional property filters)
/// that represents a meaningful conversion. Goals are single-step funnels —
/// completions are derived by querying events, never stored separately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: Ulid,
    pub site_id: Ulid,
    pub name: String,
    pub event_name: String,
    /// JSON-serialized `Vec<Filter>`; empty/None means no property filters.
    pub filters: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Goal {
    /// Decode the stored `filters` JSON into the typed filter list. A
    /// missing or malformed value yields an empty list rather than erroring.
    pub fn parsed_filters(&self) -> Vec<Filter> {
        self.filters
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }
}
