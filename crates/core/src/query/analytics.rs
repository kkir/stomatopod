//! Tier-2 analytics query types: entry/exit pages, sparklines, and the
//! raw session/event export rows.
//!
//! These mirror the shape of the JSON the API returns so handlers can
//! serialize them directly. The actual SQL lives in each storage backend.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---- Entry / exit pages ----

/// Top entry pages: where sessions begin. `bounce_rate` is the share of
/// sessions starting on this page that consisted of a single pageview.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EntryPages {
    pub rows: Vec<EntryPageRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryPageRow {
    pub url: String,
    pub sessions: u64,
    pub pct: f64,
    pub bounce_rate: f64,
}

/// Top exit pages: where sessions end. `exit_rate` is exits divided by
/// total pageviews on that page.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExitPages {
    pub rows: Vec<ExitPageRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitPageRow {
    pub url: String,
    pub exits: u64,
    pub pct: f64,
    pub exit_rate: f64,
}

// ---- Export rows ----

/// A single derived session row, used by the sessions export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub session_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration_secs: i64,
    pub pageviews: u64,
    pub entry_url: String,
    pub exit_url: String,
    pub referrer: Option<String>,
    pub country_code: Option<String>,
    pub browser: String,
    pub os: String,
    pub device_type: String,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub is_bounce: bool,
}

// ---- Tier-3: sparklines on top-N ----

/// A set of per-value mini timeseries, one row per top dimension value.
/// `points` is aligned to a shared, ordered list of day buckets so the
/// frontend can render every sparkline against the same x-axis.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TopSparklines {
    /// The day buckets (ISO `YYYY-MM-DD`) every row's `points` aligns to.
    pub days: Vec<String>,
    pub rows: Vec<SparkRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparkRow {
    pub value: String,
    pub total: u64,
    pub points: Vec<u64>,
}

impl TopSparklines {
    /// Assemble sparklines from raw `(value, day, count)` rows. `days` is the
    /// ordered bucket axis; the top `limit` values by total are kept, each
    /// projected onto the full axis (missing days → 0). Shared so all three
    /// backends agree on shape.
    pub fn from_counts(rows: Vec<(String, String, u64)>, days: Vec<String>, limit: usize) -> Self {
        let mut by_value: HashMap<String, (u64, HashMap<String, u64>)> = HashMap::new();
        for (value, day, count) in rows {
            let e = by_value.entry(value).or_default();
            e.0 += count;
            *e.1.entry(day).or_default() += count;
        }
        let mut rows: Vec<SparkRow> = by_value
            .into_iter()
            .map(|(value, (total, per_day))| {
                let points = days
                    .iter()
                    .map(|d| per_day.get(d).copied().unwrap_or(0))
                    .collect();
                SparkRow {
                    value,
                    total,
                    points,
                }
            })
            .collect();
        rows.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.value.cmp(&b.value)));
        rows.truncate(limit);
        TopSparklines { days, rows }
    }
}

/// A single raw event row, used by the events export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEventRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub timestamp: DateTime<Utc>,
    pub url: String,
    pub referrer: Option<String>,
    pub country_code: Option<String>,
    pub browser: String,
    pub os: String,
    pub device_type: String,
    pub properties: Option<String>,
}

#[cfg(test)]
mod tier3_tests {
    use super::*;

    #[test]
    fn sparklines_pick_top_values_and_align_to_axis() {
        // /a beats /b by total; axis is the sorted union of days.
        let rows = vec![
            ("/a".into(), "2026-01-01".into(), 3),
            ("/a".into(), "2026-01-03".into(), 5),
            ("/b".into(), "2026-01-02".into(), 1),
        ];
        let days = vec![
            "2026-01-01".to_string(),
            "2026-01-02".to_string(),
            "2026-01-03".to_string(),
        ];
        let s = TopSparklines::from_counts(rows, days, 1);
        assert_eq!(s.rows.len(), 1);
        assert_eq!(s.rows[0].value, "/a");
        assert_eq!(s.rows[0].total, 8);
        // Missing middle day fills with 0.
        assert_eq!(s.rows[0].points, vec![3, 0, 5]);
    }
}
