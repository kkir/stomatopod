use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl TimeRange {
    pub fn last_n_days(n: i64) -> Self {
        let end = Utc::now();
        let start = end - chrono::Duration::days(n);
        Self { start, end }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    Hour,
    #[default]
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageviewsQuery {
    pub site_id: Ulid,
    pub range: TimeRange,
    pub granularity: Granularity,
    pub filters: Vec<Filter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Filter {
    pub field: FilterField,
    pub op: FilterOp,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterField {
    Url,
    Referrer,
    Country,
    Browser,
    Os,
    DeviceType,
    UtmSource,
    UtmMedium,
    UtmCampaign,
    EventName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Eq,
    NotEq,
    Contains,
    StartsWith,
}

// ---- Result types ----

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PageviewsResult {
    pub buckets: Vec<TimeBucket>,
    pub total_pageviews: u64,
    pub total_sessions: u64,
    pub bounce_rate: f64,
    pub avg_duration_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeBucket {
    pub ts: DateTime<Utc>,
    pub pageviews: u64,
    pub sessions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TopList {
    pub rows: Vec<TopRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopRow {
    pub value: String,
    pub pageviews: u64,
    pub sessions: u64,
    pub pct: f64,
}
