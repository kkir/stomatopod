use chrono::{DateTime, NaiveDate, Utc};
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

    /// Build a range from two ISO `YYYY-MM-DD` dates (inclusive). `start` is
    /// the beginning of `from`, `end` the last microsecond of `to`. If the
    /// pair is reversed they are swapped so the range is always non-empty.
    pub fn from_dates(from: NaiveDate, to: NaiveDate) -> Self {
        let (from, to) = if from <= to { (from, to) } else { (to, from) };
        let start = from.and_hms_opt(0, 0, 0).unwrap().and_utc();
        let end = to.and_hms_micro_opt(23, 59, 59, 999_999).unwrap().and_utc();
        Self { start, end }
    }

    /// Parse `from`/`to` strings (`YYYY-MM-DD`). Returns `None` unless both
    /// parse — a half-specified custom range falls back to the preset.
    pub fn parse_dates(from: &str, to: &str) -> Option<Self> {
        let from = NaiveDate::parse_from_str(from, "%Y-%m-%d").ok()?;
        let to = NaiveDate::parse_from_str(to, "%Y-%m-%d").ok()?;
        Some(Self::from_dates(from, to))
    }

    /// The equal-length window immediately preceding this one, used for
    /// period-over-period comparison. A 7-day range maps to the 7 days
    /// before it.
    pub fn previous(&self) -> Self {
        let span = self.end - self.start;
        Self {
            start: self.start - span,
            end: self.start,
        }
    }
}

/// Canonical form of a dashboard range label. Unknown inputs fall back to
/// the default range (matching `TimeRange::from_label`), so handlers can
/// safely thread the result back into templates and link URLs without
/// propagating a bogus user-supplied value.
pub fn canonical_label(label: &str) -> &'static str {
    match label {
        "7d" => "7d",
        "30d" => "30d",
        "90d" => "90d",
        "12m" => "12m",
        _ => "30d",
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

impl Granularity {
    /// Pick a sensible default bucket size for a range. Short windows get
    /// hourly buckets; long ones widen to week/month so the chart stays
    /// readable. Used when the caller doesn't pin a granularity explicitly
    /// (e.g. a custom date range).
    pub fn auto_for_range(range: &TimeRange) -> Self {
        let days = (range.end - range.start).num_days();
        match days {
            d if d <= 1 => Granularity::Hour,
            d if d <= 92 => Granularity::Day,
            d if d <= 400 => Granularity::Week,
            _ => Granularity::Month,
        }
    }
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

impl Filter {
    /// Parse the wire form `field:op:value` (e.g. `country:eq:US`). The value
    /// may itself contain colons (URLs), so only the first two are split on.
    /// Returns `None` for unknown fields/ops or an empty value.
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.splitn(3, ':');
        let field = FilterField::from_token(parts.next()?)?;
        let op = FilterOp::from_token(parts.next()?)?;
        let value = parts.next()?.to_string();
        if value.is_empty() {
            return None;
        }
        Some(Self { field, op, value })
    }

    /// Re-render this filter as its `field:op:value` wire form. Round-trips
    /// with [`Filter::parse`].
    pub fn to_token(&self) -> String {
        format!("{}:{}:{}", self.field.token(), self.op.token(), self.value)
    }

    /// The value as it should be bound into SQL for this operator —
    /// `Contains`/`StartsWith` become `LIKE` patterns, others pass through.
    pub fn sql_value(&self) -> String {
        match self.op {
            FilterOp::Contains => format!("%{}%", escape_like(&self.value)),
            FilterOp::StartsWith => format!("{}%", escape_like(&self.value)),
            _ => self.value.clone(),
        }
    }
}

/// Escape `LIKE` metacharacters so a user-supplied value matches literally.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterField {
    Url,
    Referrer,
    Country,
    Region,
    Browser,
    Os,
    DeviceType,
    UtmSource,
    UtmMedium,
    UtmCampaign,
    UtmTerm,
    UtmContent,
    EventName,
}

impl FilterField {
    /// Events-table column this field filters on.
    pub fn column(&self) -> &'static str {
        match self {
            FilterField::Url => "url",
            FilterField::Referrer => "referrer",
            FilterField::Country => "country_code",
            FilterField::Region => "region",
            FilterField::Browser => "browser",
            FilterField::Os => "os",
            FilterField::DeviceType => "device_type",
            FilterField::UtmSource => "utm_source",
            FilterField::UtmMedium => "utm_medium",
            FilterField::UtmCampaign => "utm_campaign",
            FilterField::UtmTerm => "utm_term",
            FilterField::UtmContent => "utm_content",
            FilterField::EventName => "name",
        }
    }

    /// Canonical wire token (matches the `snake_case` serde form).
    pub fn token(&self) -> &'static str {
        match self {
            FilterField::Url => "url",
            FilterField::Referrer => "referrer",
            FilterField::Country => "country",
            FilterField::Region => "region",
            FilterField::Browser => "browser",
            FilterField::Os => "os",
            FilterField::DeviceType => "device_type",
            FilterField::UtmSource => "utm_source",
            FilterField::UtmMedium => "utm_medium",
            FilterField::UtmCampaign => "utm_campaign",
            FilterField::UtmTerm => "utm_term",
            FilterField::UtmContent => "utm_content",
            FilterField::EventName => "event_name",
        }
    }

    pub fn from_token(s: &str) -> Option<Self> {
        Some(match s {
            "url" => FilterField::Url,
            "referrer" => FilterField::Referrer,
            "country" => FilterField::Country,
            "region" => FilterField::Region,
            "browser" => FilterField::Browser,
            "os" => FilterField::Os,
            "device_type" => FilterField::DeviceType,
            "utm_source" => FilterField::UtmSource,
            "utm_medium" => FilterField::UtmMedium,
            "utm_campaign" => FilterField::UtmCampaign,
            "utm_term" => FilterField::UtmTerm,
            "utm_content" => FilterField::UtmContent,
            "event_name" => FilterField::EventName,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Eq,
    NotEq,
    Contains,
    StartsWith,
}

impl FilterOp {
    /// SQL operator. `Contains`/`StartsWith` use `LIKE` with the pattern
    /// produced by [`Filter::sql_value`].
    pub fn sql_operator(&self) -> &'static str {
        match self {
            FilterOp::Eq => "=",
            FilterOp::NotEq => "<>",
            FilterOp::Contains | FilterOp::StartsWith => "LIKE",
        }
    }

    pub fn token(&self) -> &'static str {
        match self {
            FilterOp::Eq => "eq",
            FilterOp::NotEq => "not_eq",
            FilterOp::Contains => "contains",
            FilterOp::StartsWith => "starts_with",
        }
    }

    pub fn from_token(s: &str) -> Option<Self> {
        Some(match s {
            "eq" => FilterOp::Eq,
            "not_eq" => FilterOp::NotEq,
            "contains" => FilterOp::Contains,
            "starts_with" => FilterOp::StartsWith,
            _ => return None,
        })
    }
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
    Os,
    Region,
}

impl TopListField {
    /// All dimensions, in dashboard render order.
    pub const ALL: [TopListField; 7] = [
        TopListField::Page,
        TopListField::Referrer,
        TopListField::Country,
        TopListField::Browser,
        TopListField::Device,
        TopListField::Os,
        TopListField::Region,
    ];

    /// Events-table column to group by.
    pub fn column(&self) -> &'static str {
        match self {
            TopListField::Page => "url",
            TopListField::Referrer => "referrer",
            TopListField::Country => "country_code",
            TopListField::Browser => "browser",
            TopListField::Device => "device_type",
            TopListField::Os => "os",
            TopListField::Region => "region",
        }
    }

    /// Filename of the htmx partial that renders this dimension.
    pub fn template_partial(&self) -> &'static str {
        match self {
            TopListField::Page => "partials/top_pages.jinja",
            TopListField::Referrer => "partials/top_referrers.jinja",
            TopListField::Country => "partials/top_countries.jinja",
            TopListField::Browser => "partials/top_browsers.jinja",
            TopListField::Device => "partials/top_devices.jinja",
            TopListField::Os => "partials/top_os.jinja",
            TopListField::Region => "partials/top_regions.jinja",
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn filter_parse_round_trips() {
        let f = Filter::parse("country:eq:US").unwrap();
        assert_eq!(f.field, FilterField::Country);
        assert_eq!(f.op, FilterOp::Eq);
        assert_eq!(f.value, "US");
        assert_eq!(f.to_token(), "country:eq:US");
    }

    #[test]
    fn filter_parse_keeps_colons_in_value() {
        let f = Filter::parse("referrer:starts_with:https://x.com/a").unwrap();
        assert_eq!(f.field, FilterField::Referrer);
        assert_eq!(f.value, "https://x.com/a");
        // StartsWith builds a trailing-wildcard LIKE pattern.
        assert_eq!(f.sql_value(), "https://x.com/a%");
    }

    #[test]
    fn filter_parse_rejects_unknown_and_empty() {
        assert!(Filter::parse("bogus:eq:x").is_none());
        assert!(Filter::parse("country:like:x").is_none());
        assert!(Filter::parse("country:eq:").is_none());
    }

    #[test]
    fn contains_pattern_escapes_like_metachars() {
        let f = Filter::parse("url:contains:50%_off").unwrap();
        assert_eq!(f.sql_value(), "%50\\%\\_off%");
    }

    #[test]
    fn previous_range_is_equal_length_and_abuts() {
        let r = TimeRange {
            start: Utc.with_ymd_and_hms(2026, 6, 8, 0, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 6, 15, 0, 0, 0).unwrap(),
        };
        let p = r.previous();
        assert_eq!(p.end, r.start);
        assert_eq!(r.end - r.start, p.end - p.start);
        assert_eq!(p.start, Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap());
    }

    #[test]
    fn from_dates_swaps_reversed_pair() {
        let a = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let b = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let r = TimeRange::from_dates(a, b);
        assert!(r.start < r.end);
        assert_eq!(r.start.date_naive(), b);
    }
}
