use std::cmp::Reverse;

use stomatopod_core::query::pageviews::{TopList, TopRow};

/// Compute percentage share for each row and sort by pageviews descending.
pub fn compute_percentages(mut rows: Vec<TopRow>) -> TopList {
    let total: u64 = rows.iter().map(|r| r.pageviews).sum();
    if total > 0 {
        for row in &mut rows {
            row.pct = (row.pageviews as f64 / total as f64) * 100.0;
        }
    }
    rows.sort_unstable_by_key(|r| Reverse(r.pageviews));
    TopList { rows }
}
