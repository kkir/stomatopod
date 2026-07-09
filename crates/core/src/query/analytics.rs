//! Tier-2 analytics query types: entry/exit pages, goal conversion stats,
//! and the raw session/event export rows.
//!
//! These mirror the shape of the JSON the API returns so handlers can
//! serialize them directly. The actual SQL lives in each storage backend.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ulid::Ulid;

use super::pageviews::{Filter, Granularity, TimeRange};

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

// ---- Goals ----

/// Parameters for a goal-conversion query. `event_name` + `filters` match
/// the goal's target event; `range`/`granularity` shape the timeseries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalQuery {
    pub site_id: Ulid,
    pub event_name: String,
    pub filters: Vec<Filter>,
    pub range: TimeRange,
    pub granularity: Granularity,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GoalStats {
    pub completions: u64,
    pub unique_completions: u64,
    pub conversion_rate: f64,
    pub timeseries: Vec<GoalBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalBucket {
    pub date: String,
    pub completions: u64,
    pub conversion_rate: f64,
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

// ---- Tier-3: user paths / flow ----

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PathReport {
    pub rows: Vec<PathRow>,
    pub total_sessions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathRow {
    /// Ordered page URLs that make up this navigation sequence.
    pub steps: Vec<String>,
    pub sessions: u64,
    pub pct: f64,
}

impl PathReport {
    /// Build the top page-navigation sequences from raw `(session, seq, url)`
    /// rows (the first `depth` pageviews per session). Sequences are counted
    /// across sessions; the top `limit` by frequency are returned.
    pub fn from_steps(rows: Vec<(String, u32, String)>, limit: usize) -> Self {
        let mut by_session: HashMap<String, Vec<(u32, String)>> = HashMap::new();
        for (sid, seq, url) in rows {
            by_session.entry(sid).or_default().push((seq, url));
        }
        let total_sessions = by_session.len() as u64;
        let mut counts: HashMap<Vec<String>, u64> = HashMap::new();
        for mut steps in by_session.into_values() {
            steps.sort_by_key(|(seq, _)| *seq);
            let path: Vec<String> = steps.into_iter().map(|(_, url)| url).collect();
            if path.is_empty() {
                continue;
            }
            *counts.entry(path).or_default() += 1;
        }
        let mut rows: Vec<PathRow> = counts
            .into_iter()
            .map(|(steps, sessions)| {
                let pct = if total_sessions > 0 {
                    sessions as f64 / total_sessions as f64 * 100.0
                } else {
                    0.0
                };
                PathRow {
                    steps,
                    sessions,
                    pct,
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            b.sessions
                .cmp(&a.sessions)
                .then_with(|| a.steps.cmp(&b.steps))
        });
        rows.truncate(limit);
        PathReport {
            rows,
            total_sessions,
        }
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

    #[test]
    fn paths_count_sequences_ordered_by_seq() {
        // Two sessions share A->B; one is A->C.
        let rows = vec![
            ("s1".into(), 1, "/a".into()),
            ("s1".into(), 2, "/b".into()),
            ("s2".into(), 2, "/b".into()),
            ("s2".into(), 1, "/a".into()),
            ("s3".into(), 1, "/a".into()),
            ("s3".into(), 2, "/c".into()),
        ];
        let report = PathReport::from_steps(rows, 10);
        assert_eq!(report.total_sessions, 3);
        assert_eq!(
            report.rows[0].steps,
            vec!["/a".to_string(), "/b".to_string()]
        );
        assert_eq!(report.rows[0].sessions, 2);
    }
}
