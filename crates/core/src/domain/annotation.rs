use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// A dated note pinned to a site's timeseries — "deployed v2", "launched
/// campaign X". Rendered as a vertical marker on the dashboard chart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub id: Ulid,
    pub site_id: Ulid,
    /// The day the note applies to (`YYYY-MM-DD`).
    pub date: NaiveDate,
    pub text: String,
    pub created_at: DateTime<Utc>,
}
