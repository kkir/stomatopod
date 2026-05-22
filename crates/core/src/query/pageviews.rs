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

    /// Parse the dashboard range labels (`"7d"`, `"30d"`, `"90d"`, `"12m"`).
    /// Unknown labels fall back to 30 days so the UI never breaks on bad input.
    pub fn from_label(label: &str) -> Self {
        Self::last_n_days(days_for_label(label))
    }
}

/// Map a dashboard range label to a day count. Centralised so the web,
/// partial, and CLI surfaces all agree on what `"7d"` means.
pub fn days_for_label(label: &str) -> i64 {
    match label {
        "7d" => 7,
        "30d" => 30,
        "90d" => 90,
        "12m" => 365,
        _ => 30,
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

/// The dimensions the dashboard slices traffic by. Each variant maps 1:1 to
/// a column in the events table and to a partial template. Centralising the
/// mapping here keeps the trait surface, every backend, and the route layer
/// in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopListField {
    Page,
    Referrer,
    Country,
    Browser,
    Device,
}

impl TopListField {
    /// All five dimensions, in dashboard render order.
    pub const ALL: [TopListField; 5] = [
        TopListField::Page,
        TopListField::Referrer,
        TopListField::Country,
        TopListField::Browser,
        TopListField::Device,
    ];

    /// Events-table column to group by.
    pub fn column(&self) -> &'static str {
        match self {
            TopListField::Page => "url",
            TopListField::Referrer => "referrer",
            TopListField::Country => "country_code",
            TopListField::Browser => "browser",
            TopListField::Device => "device_type",
        }
    }

    /// Filename of the htmx partial that renders this dimension.
    pub fn template_partial(&self) -> &'static str {
        match self {
            TopListField::Page => "partials/top_pages.html",
            TopListField::Referrer => "partials/top_referrers.html",
            TopListField::Country => "partials/top_countries.html",
            TopListField::Browser => "partials/top_browsers.html",
            TopListField::Device => "partials/top_devices.html",
        }
    }

    /// URL path segment used by the route table (e.g. `top_pages`).
    pub fn route_segment(&self) -> &'static str {
        match self {
            TopListField::Page => "top_pages",
            TopListField::Referrer => "top_referrers",
            TopListField::Country => "top_countries",
            TopListField::Browser => "top_browsers",
            TopListField::Device => "top_devices",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopRow {
    pub value: String,
    pub pageviews: u64,
    pub sessions: u64,
    pub pct: f64,
}
