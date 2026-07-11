//! Dense timeseries helpers: fill missing buckets so charts show zero-traffic
//! gaps instead of skipping those days.

use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use stomatopod_core::query::pageviews::{Granularity, TimeRange};

use crate::ui::query::DashQuery;
use crate::ui::types::TimeBucket;

/// Resolve the dashboard range from query params (preset or custom dates).
pub fn range_from_query(q: &DashQuery) -> TimeRange {
    if let (Some(from), Some(to)) = (q.from.as_deref(), q.to.as_deref()) {
        if let Some(r) = TimeRange::parse_dates(from, to) {
            return r;
        }
    }
    TimeRange::from_label(q.range.as_deref().unwrap_or("30d"))
}

/// Bucket size for densifying. Prefer the same auto rule the API uses so
/// filled zeros land on the same grid as real buckets. When sparse points
/// clearly use a finer step (e.g. hourly inside a multi-day window), keep that.
pub fn infer_granularity(buckets: &[TimeBucket], range: &TimeRange) -> Granularity {
    let auto = Granularity::auto_for_range(range);
    if buckets.len() < 2 {
        return auto;
    }
    let mut deltas: Vec<i64> = buckets
        .windows(2)
        .map(|w| (w[1].ts - w[0].ts).num_seconds().abs())
        .filter(|&s| s > 0)
        .collect();
    if deltas.is_empty() {
        return auto;
    }
    deltas.sort_unstable();
    let med = deltas[deltas.len() / 2];
    // Only refine when observed spacing is clearly finer than auto (e.g.
    // explicit hourly data). Never coarsen past auto or multi-day gaps
    // would collapse the axis to weekly/monthly.
    let observed = match med {
        s if s <= 90 * 60 => Granularity::Hour,
        s if s <= 36 * 3600 => Granularity::Day,
        s if s <= 10 * 24 * 3600 => Granularity::Week,
        _ => Granularity::Month,
    };
    match (auto, observed) {
        (Granularity::Day, Granularity::Hour) => Granularity::Hour,
        (Granularity::Week, Granularity::Hour | Granularity::Day) => observed,
        (Granularity::Month, g) if g != Granularity::Month => g,
        _ => auto,
    }
}

fn trunc_utc(ts: DateTime<Utc>, g: Granularity) -> DateTime<Utc> {
    match g {
        Granularity::Hour => ts
            .date_naive()
            .and_hms_opt(ts.hour(), 0, 0)
            .unwrap()
            .and_utc(),
        Granularity::Day => ts.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc(),
        Granularity::Week => {
            // Align with common date_trunc('week') (Monday start).
            let d = ts.date_naive();
            let days_from_mon = d.weekday().num_days_from_monday() as i64;
            (d - Duration::days(days_from_mon))
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
        }
        Granularity::Month => NaiveDate_ymd(ts.year(), ts.month(), 1)
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc(),
    }
}

#[allow(non_snake_case)]
fn NaiveDate_ymd(year: i32, month: u32, day: u32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(year, month, day).expect("valid ymd")
}

fn advance(ts: DateTime<Utc>, g: Granularity) -> DateTime<Utc> {
    match g {
        Granularity::Hour => ts + Duration::hours(1),
        Granularity::Day => ts + Duration::days(1),
        Granularity::Week => ts + Duration::weeks(1),
        Granularity::Month => {
            let d = ts.date_naive();
            let (y, m) = if d.month() == 12 {
                (d.year() + 1, 1)
            } else {
                (d.year(), d.month() + 1)
            };
            NaiveDate_ymd(y, m, 1).and_hms_opt(0, 0, 0).unwrap().and_utc()
        }
    }
}

/// Expand sparse query buckets to a continuous axis over `range`, inserting
/// zero pageviews/sessions for missing slots so charts show empty days.
///
/// Caps at 400 points so pathological ranges stay renderable.
pub fn fill_time_buckets(buckets: &[TimeBucket], range: &TimeRange) -> Vec<TimeBucket> {
    const MAX_POINTS: usize = 400;

    if buckets.is_empty() {
        // Still build a flat zero series across the range so callers can
        // distinguish "no data rows" from "all zeros over a full axis".
        let g = Granularity::auto_for_range(range);
        return densify(&[], range, g, MAX_POINTS);
    }

    let g = infer_granularity(buckets, range);
    densify(buckets, range, g, MAX_POINTS)
}

fn densify(
    buckets: &[TimeBucket],
    range: &TimeRange,
    g: Granularity,
    max_points: usize,
) -> Vec<TimeBucket> {
    use std::collections::HashMap;

    let by_ts: HashMap<i64, &TimeBucket> = buckets.iter().map(|b| (b.ts.timestamp(), b)).collect();

    // Prefer the query window; if sparse data falls slightly outside (clock
    // skew / open end), expand so no real points are dropped.
    let mut start = trunc_utc(range.start, g);
    let mut end = trunc_utc(range.end, g);
    if let Some(first) = buckets.first() {
        let t = trunc_utc(first.ts, g);
        if t < start {
            start = t;
        }
    }
    if let Some(last) = buckets.last() {
        let t = trunc_utc(last.ts, g);
        if t > end {
            end = t;
        }
    }
    if end < start {
        end = start;
    }

    let mut out = Vec::new();
    let mut t = start;
    while t <= end && out.len() < max_points {
        if let Some(b) = by_ts.get(&t.timestamp()) {
            out.push(TimeBucket {
                ts: t,
                pageviews: b.pageviews,
                sessions: b.sessions,
            });
        } else {
            out.push(TimeBucket {
                ts: t,
                pageviews: 0,
                sessions: 0,
            });
        }
        let next = advance(t, g);
        if next <= t {
            break;
        }
        t = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn bucket(y: i32, m: u32, d: u32, pv: u64) -> TimeBucket {
        TimeBucket {
            ts: Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap(),
            pageviews: pv,
            sessions: pv.saturating_div(2).max(if pv > 0 { 1 } else { 0 }),
        }
    }

    #[test]
    fn fills_middle_day_with_zero() {
        let range = TimeRange {
            start: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 1, 3, 23, 59, 59).unwrap(),
        };
        let sparse = vec![bucket(2026, 1, 1, 10), bucket(2026, 1, 3, 5)];
        let filled = fill_time_buckets(&sparse, &range);
        assert_eq!(filled.len(), 3);
        assert_eq!(filled[0].pageviews, 10);
        assert_eq!(filled[1].pageviews, 0);
        assert_eq!(filled[1].sessions, 0);
        assert_eq!(filled[2].pageviews, 5);
    }

    #[test]
    fn fills_leading_and_trailing_zeros() {
        let range = TimeRange {
            start: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 1, 5, 12, 0, 0).unwrap(),
        };
        let sparse = vec![bucket(2026, 1, 3, 7)];
        let filled = fill_time_buckets(&sparse, &range);
        assert_eq!(filled.len(), 5);
        assert_eq!(filled[0].pageviews, 0);
        assert_eq!(filled[2].pageviews, 7);
        assert_eq!(filled[4].pageviews, 0);
    }
}
