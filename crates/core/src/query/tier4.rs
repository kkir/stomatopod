//! Tier-4 analytics aggregations: Core Web Vitals, scroll depth/engagement,
//! A/B experiments, revenue, heatmaps, and site search.
//!
//! Every feature here is built from raw custom-event rows ([`EventPropRow`])
//! that storage backends fetch with `query_event_props`. Keeping the
//! aggregation logic as pure functions in `core` means all three backends
//! agree on output shape and the math is unit-tested in one place.
//!
//! By convention these features ride on custom events with reserved,
//! double-underscore names emitted by the browser tracker:
//! `__vital__`, `__scroll__`, `__click__`, `__search__`. Revenue and A/B
//! testing instead read conventional properties (`revenue`, `experiment`/
//! `variant`) off ordinary custom events.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::pageviews::Granularity;

/// Reserved tracker event names that must never appear in the generic
/// custom-events report (they back dedicated Tier-4 surfaces instead).
pub const RESERVED_EVENT_NAMES: &[&str] = &["__vital__", "__scroll__", "__click__", "__search__"];

/// True if `name` is a reserved Tier-4 tracker event.
pub fn is_reserved_event(name: &str) -> bool {
    RESERVED_EVENT_NAMES.contains(&name)
}

/// A raw custom-event row used as the input to every Tier-4 aggregation.
/// Backends populate this from the event store; the property bag carries
/// feature-specific fields (metric/value/rating, depth, x/y, query, revenue…).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPropRow {
    pub name: String,
    pub url: String,
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
    pub referrer: Option<String>,
    pub country_code: Option<String>,
    pub utm_source: Option<String>,
    pub properties: Option<serde_json::Value>,
}

impl EventPropRow {
    fn prop(&self, key: &str) -> Option<&serde_json::Value> {
        self.properties.as_ref().and_then(|p| p.get(key))
    }

    fn prop_f64(&self, key: &str) -> Option<f64> {
        self.prop(key).and_then(json_f64)
    }

    fn prop_str(&self, key: &str) -> Option<String> {
        self.prop(key).and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            _ => None,
        })
    }
}

/// Coerce a JSON value to f64, accepting numbers and numeric strings.
fn json_f64(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Nearest-rank percentile over an unsorted sample. `pct` is in `[0,100]`.
/// Returns 0.0 for an empty sample.
fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (pct / 100.0 * sorted.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[idx]
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

// ---------------------------------------------------------------------------
// Core Web Vitals
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct VitalStat {
    pub p50: f64,
    pub p75: f64,
    pub p95: f64,
    pub good_pct: f64,
    pub samples: u64,
}

impl VitalStat {
    fn from_samples(values: &mut Vec<f64>, good: u64) -> Self {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = values.len() as u64;
        VitalStat {
            p50: round2(percentile(values, 50.0)),
            p75: round2(percentile(values, 75.0)),
            p95: round2(percentile(values, 95.0)),
            good_pct: if n > 0 {
                round1(good as f64 / n as f64 * 100.0)
            } else {
                0.0
            },
            samples: n,
        }
    }
}

/// Aggregated Core Web Vitals report: one [`VitalStat`] per metric.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VitalsReport {
    pub lcp: VitalStat,
    pub cls: VitalStat,
    pub inp: VitalStat,
}

impl VitalsReport {
    /// Build from `__vital__` rows. `url` optionally narrows to one page.
    pub fn from_rows(rows: &[EventPropRow], url: Option<&str>) -> Self {
        let mut buckets: HashMap<String, (Vec<f64>, u64)> = HashMap::new();
        for r in rows {
            if r.name != "__vital__" {
                continue;
            }
            if let Some(u) = url {
                let row_url = r.prop_str("url").unwrap_or_else(|| r.url.clone());
                if row_url != u {
                    continue;
                }
            }
            let Some(metric) = r.prop_str("metric") else {
                continue;
            };
            let Some(value) = r.prop_f64("value") else {
                continue;
            };
            let entry = buckets.entry(metric.to_uppercase()).or_default();
            entry.0.push(value);
            if r.prop_str("rating").as_deref() == Some("good") {
                entry.1 += 1;
            }
        }
        let mut stat = |m: &str| {
            buckets
                .get_mut(m)
                .map(|(v, g)| VitalStat::from_samples(v, *g))
                .unwrap_or_default()
        };
        VitalsReport {
            lcp: stat("LCP"),
            cls: stat("CLS"),
            inp: stat("INP"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VitalPageRow {
    pub url: String,
    pub p75: f64,
    pub good_pct: f64,
    pub samples: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VitalPagesReport {
    pub metric: String,
    pub rows: Vec<VitalPageRow>,
}

impl VitalPagesReport {
    /// Per-page breakdown for a single metric, ranked by worst (highest) p75
    /// — the pages most impactful to fix.
    pub fn from_rows(rows: &[EventPropRow], metric: &str, limit: usize) -> Self {
        let metric_uc = metric.to_uppercase();
        let mut by_url: HashMap<String, (Vec<f64>, u64)> = HashMap::new();
        for r in rows {
            if r.name != "__vital__" {
                continue;
            }
            if r.prop_str("metric").map(|m| m.to_uppercase()).as_deref() != Some(metric_uc.as_str())
            {
                continue;
            }
            let Some(value) = r.prop_f64("value") else {
                continue;
            };
            let url = r.prop_str("url").unwrap_or_else(|| r.url.clone());
            let entry = by_url.entry(url).or_default();
            entry.0.push(value);
            if r.prop_str("rating").as_deref() == Some("good") {
                entry.1 += 1;
            }
        }
        let mut rows: Vec<VitalPageRow> = by_url
            .into_iter()
            .map(|(url, (mut vals, good))| {
                let stat = VitalStat::from_samples(&mut vals, good);
                VitalPageRow {
                    url,
                    p75: stat.p75,
                    good_pct: stat.good_pct,
                    samples: stat.samples,
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            b.p75
                .partial_cmp(&a.p75)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.url.cmp(&b.url))
        });
        rows.truncate(limit);
        VitalPagesReport {
            metric: metric_uc,
            rows,
        }
    }
}

// ---------------------------------------------------------------------------
// Scroll depth / engagement
// ---------------------------------------------------------------------------

const SCROLL_MILESTONES: [u32; 4] = [25, 50, 75, 100];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScrollReport {
    pub url: Option<String>,
    pub sessions_with_scroll_data: u64,
    pub reached_25pct: f64,
    pub reached_50pct: f64,
    pub reached_75pct: f64,
    pub reached_100pct: f64,
}

/// Per-session deepest scroll milestone, keyed by `(session, url)`.
fn session_max_depths(rows: &[EventPropRow], url: Option<&str>) -> HashMap<(String, String), u32> {
    let mut max_depth: HashMap<(String, String), u32> = HashMap::new();
    for r in rows {
        if r.name != "__scroll__" {
            continue;
        }
        let row_url = r.prop_str("url").unwrap_or_else(|| r.url.clone());
        if let Some(u) = url {
            if row_url != u {
                continue;
            }
        }
        let Some(depth) = r.prop_f64("depth") else {
            continue;
        };
        let key = (r.session_id.clone(), row_url);
        let e = max_depth.entry(key).or_insert(0);
        *e = (*e).max(depth as u32);
    }
    max_depth
}

impl ScrollReport {
    /// Engagement for one page (or all pages when `url` is `None`).
    pub fn from_rows(rows: &[EventPropRow], url: Option<&str>) -> Self {
        let depths = session_max_depths(rows, url);
        let total = depths.len() as u64;
        let pct_reaching = |m: u32| -> f64 {
            if total == 0 {
                return 0.0;
            }
            let reached = depths.values().filter(|&&d| d >= m).count() as u64;
            round1(reached as f64 / total as f64 * 100.0)
        };
        ScrollReport {
            url: url.map(|s| s.to_string()),
            sessions_with_scroll_data: total,
            reached_25pct: pct_reaching(25),
            reached_50pct: pct_reaching(50),
            reached_75pct: pct_reaching(75),
            reached_100pct: pct_reaching(100),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScrollPagesReport {
    pub rows: Vec<ScrollReport>,
}

impl ScrollPagesReport {
    /// Per-page scroll engagement ranked by `reached_100pct` descending
    /// (most fully-read content first).
    pub fn from_rows(rows: &[EventPropRow], limit: usize) -> Self {
        let depths = session_max_depths(rows, None);
        // url -> (total sessions, count reaching each milestone)
        let mut by_url: HashMap<String, (u64, [u64; 4])> = HashMap::new();
        for ((_, url), depth) in &depths {
            let e = by_url.entry(url.clone()).or_insert((0, [0; 4]));
            e.0 += 1;
            for (i, m) in SCROLL_MILESTONES.iter().enumerate() {
                if depth >= m {
                    e.1[i] += 1;
                }
            }
        }
        let pct = |reached: u64, total: u64| {
            if total == 0 {
                0.0
            } else {
                round1(reached as f64 / total as f64 * 100.0)
            }
        };
        let mut out: Vec<ScrollReport> = by_url
            .into_iter()
            .map(|(url, (total, hits))| ScrollReport {
                url: Some(url),
                sessions_with_scroll_data: total,
                reached_25pct: pct(hits[0], total),
                reached_50pct: pct(hits[1], total),
                reached_75pct: pct(hits[2], total),
                reached_100pct: pct(hits[3], total),
            })
            .collect();
        out.sort_by(|a, b| {
            b.reached_100pct
                .partial_cmp(&a.reached_100pct)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.url.cmp(&b.url))
        });
        out.truncate(limit);
        ScrollPagesReport { rows: out }
    }
}

// ---------------------------------------------------------------------------
// Site search
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRow {
    pub query: String,
    pub count: u64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchReport {
    pub total_searches: u64,
    pub rows: Vec<SearchRow>,
}

fn search_query(r: &EventPropRow) -> Option<String> {
    if r.name != "__search__" && r.name != "site_search" {
        return None;
    }
    let q = r.prop_str("query")?;
    let q = q.trim();
    if q.is_empty() {
        None
    } else {
        Some(q.to_string())
    }
}

impl SearchReport {
    /// Top internal search terms with share of total search volume.
    pub fn from_rows(rows: &[EventPropRow], limit: usize) -> Self {
        let mut counts: HashMap<String, u64> = HashMap::new();
        let mut total = 0u64;
        for r in rows {
            if let Some(q) = search_query(r) {
                *counts.entry(q).or_default() += 1;
                total += 1;
            }
        }
        let mut rows: Vec<SearchRow> = counts
            .into_iter()
            .map(|(query, count)| SearchRow {
                query,
                count,
                pct: if total > 0 {
                    round1(count as f64 / total as f64 * 100.0)
                } else {
                    0.0
                },
            })
            .collect();
        rows.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.query.cmp(&b.query)));
        rows.truncate(limit);
        SearchReport {
            total_searches: total,
            rows,
        }
    }

    /// Queries that returned zero results (requires `results_count` to be
    /// instrumented on the search event), ranked by frequency.
    pub fn zero_results(rows: &[EventPropRow], limit: usize) -> Self {
        let mut counts: HashMap<String, u64> = HashMap::new();
        let mut total = 0u64;
        for r in rows {
            let Some(q) = search_query(r) else { continue };
            match r.prop_f64("results_count") {
                Some(c) if c == 0.0 => {
                    *counts.entry(q).or_default() += 1;
                    total += 1;
                }
                _ => {}
            }
        }
        let mut rows: Vec<SearchRow> = counts
            .into_iter()
            .map(|(query, count)| SearchRow {
                query,
                count,
                pct: if total > 0 {
                    round1(count as f64 / total as f64 * 100.0)
                } else {
                    0.0
                },
            })
            .collect();
        rows.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.query.cmp(&b.query)));
        rows.truncate(limit);
        SearchReport {
            total_searches: total,
            rows,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchTimeseriesBucket {
    pub date: String,
    pub searches: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchTimeseries {
    pub buckets: Vec<SearchTimeseriesBucket>,
}

impl SearchTimeseries {
    /// Daily search-volume buckets.
    pub fn from_rows(rows: &[EventPropRow]) -> Self {
        let mut by_day: BTreeMap<String, u64> = BTreeMap::new();
        for r in rows {
            if search_query(r).is_some() {
                *by_day
                    .entry(r.timestamp.format("%Y-%m-%d").to_string())
                    .or_default() += 1;
            }
        }
        SearchTimeseries {
            buckets: by_day
                .into_iter()
                .map(|(date, searches)| SearchTimeseriesBucket { date, searches })
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Revenue / e-commerce
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RevenueSummary {
    pub total_revenue: f64,
    pub orders: u64,
    pub aov: f64,
    pub revenue_per_session: f64,
    pub currency: String,
}

/// A single deduplicated revenue-bearing order derived from an event.
struct Order {
    revenue: f64,
    session_id: String,
    url: String,
    referrer: Option<String>,
    country_code: Option<String>,
    utm_source: Option<String>,
    day: String,
}

/// Extract deduplicated orders from rows. Events carrying a numeric `revenue`
/// property are orders; `order_id` (when present) dedupes repeat events for
/// the same order. Events without `order_id` are always counted.
fn extract_orders(rows: &[EventPropRow]) -> Vec<Order> {
    let mut seen_orders: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for r in rows {
        let Some(revenue) = r.prop_f64("revenue") else {
            continue;
        };
        if let Some(order_id) = r.prop_str("order_id") {
            if !seen_orders.insert(order_id) {
                continue; // duplicate order — skip
            }
        }
        out.push(Order {
            revenue,
            session_id: r.session_id.clone(),
            url: r.url.clone(),
            referrer: r.referrer.clone(),
            country_code: r.country_code.clone(),
            utm_source: r.utm_source.clone(),
            day: r.timestamp.format("%Y-%m-%d").to_string(),
        });
    }
    out
}

impl RevenueSummary {
    /// Headline revenue metrics. `total_sessions` is the site-wide session
    /// count for the window (drives revenue-per-session); `currency` is the
    /// site's configured display currency.
    pub fn from_rows(rows: &[EventPropRow], total_sessions: u64, currency: &str) -> Self {
        let orders = extract_orders(rows);
        let total: f64 = orders.iter().map(|o| o.revenue).sum();
        let order_count = orders.len() as u64;
        RevenueSummary {
            total_revenue: round2(total),
            orders: order_count,
            aov: if order_count > 0 {
                round2(total / order_count as f64)
            } else {
                0.0
            },
            revenue_per_session: if total_sessions > 0 {
                round2(total / total_sessions as f64)
            } else {
                0.0
            },
            currency: currency.to_string(),
        }
    }

    /// Whether any revenue events were observed (drives conditional UI).
    pub fn has_revenue(rows: &[EventPropRow]) -> bool {
        rows.iter().any(|r| r.prop_f64("revenue").is_some())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevenueBucket {
    pub date: String,
    pub revenue: f64,
    pub orders: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RevenueTimeseries {
    pub currency: String,
    pub buckets: Vec<RevenueBucket>,
}

impl RevenueTimeseries {
    /// Daily revenue + order-count buckets. `_granularity` is accepted for
    /// API symmetry; aggregation is by calendar day.
    pub fn from_rows(rows: &[EventPropRow], currency: &str, _granularity: Granularity) -> Self {
        let orders = extract_orders(rows);
        let mut by_day: BTreeMap<String, (f64, u64)> = BTreeMap::new();
        for o in &orders {
            let e = by_day.entry(o.day.clone()).or_insert((0.0, 0));
            e.0 += o.revenue;
            e.1 += 1;
        }
        let buckets = by_day
            .into_iter()
            .map(|(date, (revenue, orders))| RevenueBucket {
                date,
                revenue: round2(revenue),
                orders,
            })
            .collect();
        RevenueTimeseries {
            currency: currency.to_string(),
            buckets,
        }
    }
}

/// Dimension to break revenue down by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevenueDimension {
    Referrer,
    Country,
    UtmSource,
    Page,
}

impl RevenueDimension {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "referrer" => Some(Self::Referrer),
            "country" => Some(Self::Country),
            "utm_source" => Some(Self::UtmSource),
            "page" | "url" => Some(Self::Page),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevenueBreakdownRow {
    pub value: String,
    pub revenue: f64,
    pub orders: u64,
    pub revenue_per_session: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RevenueBreakdown {
    pub dimension: String,
    pub currency: String,
    pub rows: Vec<RevenueBreakdownRow>,
}

impl RevenueBreakdown {
    /// Revenue grouped by a single dimension, ordered by revenue descending.
    pub fn from_rows(
        rows: &[EventPropRow],
        dimension: RevenueDimension,
        currency: &str,
        limit: usize,
    ) -> Self {
        let orders = extract_orders(rows);
        // value -> (revenue, order count, distinct sessions)
        let mut groups: HashMap<String, (f64, u64, HashSet<String>)> = HashMap::new();
        for o in &orders {
            let value = match dimension {
                RevenueDimension::Referrer => o.referrer.clone().unwrap_or_else(|| "Direct".into()),
                RevenueDimension::Country => {
                    o.country_code.clone().unwrap_or_else(|| "Unknown".into())
                }
                RevenueDimension::UtmSource => {
                    o.utm_source.clone().unwrap_or_else(|| "(none)".into())
                }
                RevenueDimension::Page => o.url.clone(),
            };
            let e = groups.entry(value).or_insert((0.0, 0, HashSet::new()));
            e.0 += o.revenue;
            e.1 += 1;
            e.2.insert(o.session_id.clone());
        }
        let mut rows: Vec<RevenueBreakdownRow> = groups
            .into_iter()
            .map(|(value, (revenue, orders, sessions))| {
                let sess = sessions.len() as u64;
                RevenueBreakdownRow {
                    value,
                    revenue: round2(revenue),
                    orders,
                    revenue_per_session: if sess > 0 {
                        round2(revenue / sess as f64)
                    } else {
                        0.0
                    },
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            b.revenue
                .partial_cmp(&a.revenue)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.value.cmp(&b.value))
        });
        rows.truncate(limit);
        let dimension = match dimension {
            RevenueDimension::Referrer => "referrer",
            RevenueDimension::Country => "country",
            RevenueDimension::UtmSource => "utm_source",
            RevenueDimension::Page => "page",
        };
        RevenueBreakdown {
            dimension: dimension.to_string(),
            currency: currency.to_string(),
            rows,
        }
    }
}

// ---------------------------------------------------------------------------
// A/B experiments
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentInfo {
    pub name: String,
    pub variants: Vec<String>,
    pub first_seen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExperimentList {
    pub experiments: Vec<ExperimentInfo>,
}

impl ExperimentList {
    /// Auto-detect experiments from any event carrying `experiment` +
    /// `variant` properties.
    pub fn from_rows(rows: &[EventPropRow]) -> Self {
        // name -> (variants set, earliest timestamp)
        let mut found: BTreeMap<String, (BTreeSet<String>, DateTime<Utc>)> = BTreeMap::new();
        for r in rows {
            let (Some(exp), Some(variant)) = (r.prop_str("experiment"), r.prop_str("variant"))
            else {
                continue;
            };
            let entry = found
                .entry(exp)
                .or_insert_with(|| (BTreeSet::new(), r.timestamp));
            entry.0.insert(variant);
            if r.timestamp < entry.1 {
                entry.1 = r.timestamp;
            }
        }
        let experiments = found
            .into_iter()
            .map(|(name, (variants, first))| ExperimentInfo {
                name,
                variants: variants.into_iter().collect(),
                first_seen: first.format("%Y-%m-%d").to_string(),
            })
            .collect();
        ExperimentList { experiments }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantResult {
    pub variant: String,
    pub exposures: u64,
    pub conversions: u64,
    pub conversion_rate: f64,
    pub pct_of_traffic: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExperimentResult {
    pub experiment: String,
    pub goal: Option<String>,
    pub variants: Vec<VariantResult>,
    pub winner: Option<String>,
    pub confidence: Option<f64>,
    /// True when some variant has < 100 exposures (low statistical power).
    pub insufficient_data: bool,
}

impl ExperimentResult {
    /// Variant-level exposure + conversion comparison for one experiment.
    /// `goal` (when given) is the event name whose presence in an exposed
    /// session counts as a conversion; without it, only exposures are shown.
    pub fn from_rows(rows: &[EventPropRow], experiment: &str, goal: Option<&str>) -> Self {
        // variant -> set of exposed sessions
        let mut exposed: HashMap<String, HashSet<String>> = HashMap::new();
        // session -> earliest exposure time per (variant)
        let mut exposure_time: HashMap<String, DateTime<Utc>> = HashMap::new();
        // session -> set of goal-event times
        let mut goal_sessions: HashMap<String, Vec<DateTime<Utc>>> = HashMap::new();
        // session -> any non-exposure event times (for the no-goal fallback)
        let mut activity: HashMap<String, Vec<DateTime<Utc>>> = HashMap::new();

        for r in rows {
            let is_exposure = r.prop_str("experiment").as_deref() == Some(experiment)
                && r.prop_str("variant").is_some();
            if is_exposure {
                let variant = r.prop_str("variant").unwrap();
                exposed
                    .entry(variant)
                    .or_default()
                    .insert(r.session_id.clone());
                exposure_time
                    .entry(r.session_id.clone())
                    .and_modify(|t| {
                        if r.timestamp < *t {
                            *t = r.timestamp
                        }
                    })
                    .or_insert(r.timestamp);
            }
            if let Some(g) = goal {
                if r.name == g {
                    goal_sessions
                        .entry(r.session_id.clone())
                        .or_default()
                        .push(r.timestamp);
                }
            }
            activity
                .entry(r.session_id.clone())
                .or_default()
                .push(r.timestamp);
        }

        let converted = |session: &str| -> bool {
            let Some(&exp_t) = exposure_time.get(session) else {
                return false;
            };
            match goal {
                Some(_) => goal_sessions
                    .get(session)
                    .map(|times| times.iter().any(|&t| t >= exp_t))
                    .unwrap_or(false),
                None => activity
                    .get(session)
                    .map(|times| times.iter().any(|&t| t > exp_t))
                    .unwrap_or(false),
            }
        };

        let total_exposures: u64 = exposed.values().map(|s| s.len() as u64).sum();
        let mut variants: Vec<VariantResult> = exposed
            .iter()
            .map(|(variant, sessions)| {
                let exposures = sessions.len() as u64;
                let conversions = if goal.is_some() {
                    sessions.iter().filter(|s| converted(s)).count() as u64
                } else {
                    0
                };
                VariantResult {
                    variant: variant.clone(),
                    exposures,
                    conversions,
                    conversion_rate: if goal.is_some() && exposures > 0 {
                        round1(conversions as f64 / exposures as f64 * 100.0)
                    } else {
                        0.0
                    },
                    pct_of_traffic: if total_exposures > 0 {
                        round1(exposures as f64 / total_exposures as f64 * 100.0)
                    } else {
                        0.0
                    },
                }
            })
            .collect();
        variants.sort_by(|a, b| a.variant.cmp(&b.variant));

        let insufficient_data = variants.iter().any(|v| v.exposures < 100);

        let (winner, confidence) = if goal.is_some() && variants.len() >= 2 {
            let mut ranked: Vec<&VariantResult> = variants.iter().collect();
            ranked.sort_by(|a, b| {
                b.conversion_rate
                    .partial_cmp(&a.conversion_rate)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let best = ranked[0];
            let second = ranked[1];
            let conf = two_proportion_confidence(
                best.conversions,
                best.exposures,
                second.conversions,
                second.exposures,
            );
            (Some(best.variant.clone()), Some(round1(conf)))
        } else {
            (None, None)
        };

        ExperimentResult {
            experiment: experiment.to_string(),
            goal: goal.map(|s| s.to_string()),
            variants,
            winner,
            confidence,
            insufficient_data,
        }
    }
}

/// Two-proportion z-test, returned as a confidence percentage (two-sided).
/// Compares conversion proportions `a/na` vs `b/nb`.
pub fn two_proportion_confidence(a: u64, na: u64, b: u64, nb: u64) -> f64 {
    if na == 0 || nb == 0 {
        return 0.0;
    }
    let p1 = a as f64 / na as f64;
    let p2 = b as f64 / nb as f64;
    let pooled = (a + b) as f64 / (na + nb) as f64;
    let se = (pooled * (1.0 - pooled) * (1.0 / na as f64 + 1.0 / nb as f64)).sqrt();
    if se == 0.0 {
        return 0.0;
    }
    let z = (p1 - p2).abs() / se;
    // Two-sided confidence = (2 * Phi(z) - 1) * 100.
    let conf = (2.0 * standard_normal_cdf(z) - 1.0) * 100.0;
    conf.clamp(0.0, 100.0)
}

/// Standard normal CDF via the Abramowitz & Stegun erf approximation.
fn standard_normal_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

fn erf(x: f64) -> f64 {
    // Abramowitz & Stegun 7.1.26, max error ~1.5e-7.
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0
        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t
            * (-x * x).exp();
    sign * y
}

// ---------------------------------------------------------------------------
// Heatmaps
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClickPoint {
    pub x: u32,
    pub y: u32,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickElementRow {
    pub element: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClickHeatmap {
    pub url: String,
    pub total_clicks: u64,
    pub points: Vec<ClickPoint>,
    pub elements: Vec<ClickElementRow>,
}

impl ClickHeatmap {
    /// Bucketed click density for one page. `x`/`y` are percentages (0–100);
    /// they are snapped to a grid (`x_step`/`y_step`) to bound payload size.
    /// Also returns the top-clicked element selectors (MVP table fallback).
    pub fn from_rows(rows: &[EventPropRow], url: &str, x_step: u32, y_step: u32) -> Self {
        let x_step = x_step.max(1);
        let y_step = y_step.max(1);
        let mut grid: BTreeMap<(u32, u32), u64> = BTreeMap::new();
        let mut elements: HashMap<String, u64> = HashMap::new();
        let mut total = 0u64;
        for r in rows {
            if r.name != "__click__" {
                continue;
            }
            let row_url = r.prop_str("url").unwrap_or_else(|| r.url.clone());
            if row_url != url {
                continue;
            }
            let (Some(x), Some(y)) = (r.prop_f64("x"), r.prop_f64("y")) else {
                continue;
            };
            let xb = ((x.clamp(0.0, 100.0) as u32) / x_step) * x_step;
            let yb = ((y.clamp(0.0, 100.0) as u32) / y_step) * y_step;
            *grid.entry((xb, yb)).or_default() += 1;
            total += 1;
            if let Some(el) = r.prop_str("element") {
                if !el.is_empty() {
                    *elements.entry(el).or_default() += 1;
                }
            }
        }
        let points = grid
            .into_iter()
            .map(|((x, y), count)| ClickPoint { x, y, count })
            .collect();
        let mut elements: Vec<ClickElementRow> = elements
            .into_iter()
            .map(|(element, count)| ClickElementRow { element, count })
            .collect();
        elements.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.element.cmp(&b.element)));
        elements.truncate(20);
        ClickHeatmap {
            url: url.to_string(),
            total_clicks: total,
            points,
            elements,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollHeatmapBucket {
    pub depth_pct: u32,
    pub reached_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScrollHeatmap {
    pub url: String,
    pub sessions: u64,
    pub scroll_distribution: Vec<ScrollHeatmapBucket>,
}

impl ScrollHeatmap {
    /// Scroll reach distribution for one page, derived from `__scroll__`
    /// milestone events. Depth 0 is everyone (100%).
    pub fn from_rows(rows: &[EventPropRow], url: &str) -> Self {
        let depths = session_max_depths(rows, Some(url));
        let total = depths.len() as u64;
        let reached = |m: u32| -> f64 {
            if total == 0 {
                return 0.0;
            }
            if m == 0 {
                return 100.0;
            }
            let r = depths.values().filter(|&&d| d >= m).count() as u64;
            round1(r as f64 / total as f64 * 100.0)
        };
        let distribution = [0u32, 25, 50, 75, 100]
            .into_iter()
            .map(|m| ScrollHeatmapBucket {
                depth_pct: m,
                reached_pct: reached(m),
            })
            .collect();
        ScrollHeatmap {
            url: url.to_string(),
            sessions: total,
            scroll_distribution: distribution,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, session: &str, props: serde_json::Value) -> EventPropRow {
        EventPropRow {
            name: name.into(),
            url: "/".into(),
            session_id: session.into(),
            timestamp: Utc::now(),
            referrer: None,
            country_code: None,
            utm_source: None,
            properties: Some(props),
        }
    }

    #[test]
    fn percentile_nearest_rank() {
        let v = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&v, 50.0), 3.0);
        assert_eq!(percentile(&v, 95.0), 5.0);
        assert_eq!(percentile(&v, 100.0), 5.0);
    }

    #[test]
    fn vitals_percentiles_and_good_pct() {
        let mut rows = vec![];
        for v in [1000.0, 2000.0, 3000.0, 4000.0] {
            let rating = if v < 2500.0 { "good" } else { "poor" };
            rows.push(row(
                "__vital__",
                "s",
                serde_json::json!({"metric": "LCP", "value": v, "rating": rating, "url": "/p"}),
            ));
        }
        let r = VitalsReport::from_rows(&rows, None);
        assert_eq!(r.lcp.samples, 4);
        assert_eq!(r.lcp.p50, 2000.0);
        assert_eq!(r.lcp.p75, 3000.0);
        assert_eq!(r.lcp.good_pct, 50.0);
        // url filter narrows nothing here (all /p), still 4.
        let r2 = VitalsReport::from_rows(&rows, Some("/other"));
        assert_eq!(r2.lcp.samples, 0);
    }

    #[test]
    fn vital_pages_ranked_by_worst_p75() {
        let mut rows = vec![];
        for v in [1000.0, 1000.0] {
            rows.push(EventPropRow {
                url: "/fast".into(),
                ..row(
                    "__vital__",
                    "s",
                    serde_json::json!({"metric": "LCP", "value": v, "url": "/fast"}),
                )
            });
        }
        for v in [5000.0, 6000.0] {
            rows.push(EventPropRow {
                url: "/slow".into(),
                ..row(
                    "__vital__",
                    "s",
                    serde_json::json!({"metric": "LCP", "value": v, "url": "/slow"}),
                )
            });
        }
        let r = VitalPagesReport::from_rows(&rows, "lcp", 10);
        assert_eq!(r.rows[0].url, "/slow");
        assert!(r.rows[0].p75 > r.rows[1].p75);
    }

    #[test]
    fn scroll_milestones_per_session_max() {
        // s1 reaches 75, s2 reaches 25.
        let rows = vec![
            row("__scroll__", "s1", serde_json::json!({"depth": 25, "url": "/a"})),
            row("__scroll__", "s1", serde_json::json!({"depth": 75, "url": "/a"})),
            row("__scroll__", "s2", serde_json::json!({"depth": 25, "url": "/a"})),
        ];
        let r = ScrollReport::from_rows(&rows, Some("/a"));
        assert_eq!(r.sessions_with_scroll_data, 2);
        assert_eq!(r.reached_25pct, 100.0);
        assert_eq!(r.reached_75pct, 50.0);
        assert_eq!(r.reached_100pct, 0.0);
    }

    #[test]
    fn search_top_terms_and_pct() {
        let rows = vec![
            row("__search__", "s1", serde_json::json!({"query": "pricing"})),
            row("__search__", "s2", serde_json::json!({"query": "pricing"})),
            row("__search__", "s3", serde_json::json!({"query": "docs"})),
        ];
        let r = SearchReport::from_rows(&rows, 10);
        assert_eq!(r.total_searches, 3);
        assert_eq!(r.rows[0].query, "pricing");
        assert_eq!(r.rows[0].count, 2);
        assert!((r.rows[0].pct - 66.7).abs() < 0.2);
    }

    #[test]
    fn search_zero_results_only_counts_zero() {
        let rows = vec![
            row("__search__", "s1", serde_json::json!({"query": "x", "results_count": 0})),
            row("__search__", "s2", serde_json::json!({"query": "y", "results_count": 5})),
        ];
        let r = SearchReport::zero_results(&rows, 10);
        assert_eq!(r.total_searches, 1);
        assert_eq!(r.rows[0].query, "x");
    }

    #[test]
    fn revenue_summary_dedupes_orders() {
        let rows = vec![
            row("purchase", "s1", serde_json::json!({"revenue": 50.0, "order_id": "o1"})),
            // duplicate of o1 — must not double count.
            row("purchase", "s1", serde_json::json!({"revenue": 50.0, "order_id": "o1"})),
            row("purchase", "s2", serde_json::json!({"revenue": 30.0, "order_id": "o2"})),
            // no order_id — counted independently.
            row("purchase", "s3", serde_json::json!({"revenue": 20.0})),
        ];
        let r = RevenueSummary::from_rows(&rows, 100, "USD");
        assert_eq!(r.orders, 3);
        assert_eq!(r.total_revenue, 100.0);
        assert!((r.aov - 33.33).abs() < 0.01);
        assert_eq!(r.revenue_per_session, 1.0);
    }

    #[test]
    fn revenue_breakdown_by_country() {
        let rows = vec![
            EventPropRow {
                country_code: Some("US".into()),
                ..row("purchase", "s1", serde_json::json!({"revenue": 100.0}))
            },
            EventPropRow {
                country_code: Some("GB".into()),
                ..row("purchase", "s2", serde_json::json!({"revenue": 40.0}))
            },
        ];
        let r = RevenueBreakdown::from_rows(&rows, RevenueDimension::Country, "USD", 10);
        assert_eq!(r.rows[0].value, "US");
        assert_eq!(r.rows[0].revenue, 100.0);
    }

    #[test]
    fn experiment_list_detects_variants() {
        let rows = vec![
            row("exp", "s1", serde_json::json!({"experiment": "cta", "variant": "A"})),
            row("exp", "s2", serde_json::json!({"experiment": "cta", "variant": "B"})),
        ];
        let l = ExperimentList::from_rows(&rows);
        assert_eq!(l.experiments.len(), 1);
        assert_eq!(l.experiments[0].name, "cta");
        assert_eq!(l.experiments[0].variants, vec!["A", "B"]);
    }

    #[test]
    fn experiment_result_conversion_and_winner() {
        let mut rows = vec![];
        // Variant A: 4 exposures, 1 conversion.
        for i in 0..4 {
            let s = format!("a{i}");
            rows.push(row(
                "experiment_viewed",
                &s,
                serde_json::json!({"experiment": "cta", "variant": "A"}),
            ));
            if i == 0 {
                rows.push(row("signup", &s, serde_json::json!({})));
            }
        }
        // Variant B: 4 exposures, 3 conversions.
        for i in 0..4 {
            let s = format!("b{i}");
            rows.push(row(
                "experiment_viewed",
                &s,
                serde_json::json!({"experiment": "cta", "variant": "B"}),
            ));
            if i < 3 {
                rows.push(row("signup", &s, serde_json::json!({})));
            }
        }
        let r = ExperimentResult::from_rows(&rows, "cta", Some("signup"));
        let a = r.variants.iter().find(|v| v.variant == "A").unwrap();
        let b = r.variants.iter().find(|v| v.variant == "B").unwrap();
        assert_eq!(a.exposures, 4);
        assert_eq!(a.conversions, 1);
        assert_eq!(b.conversions, 3);
        assert_eq!(r.winner.as_deref(), Some("B"));
        assert!(r.insufficient_data); // < 100 exposures
        assert!(r.confidence.is_some());
    }

    #[test]
    fn experiment_no_goal_shows_exposures_only() {
        let rows = vec![
            row("exp", "s1", serde_json::json!({"experiment": "cta", "variant": "A"})),
            row("exp", "s2", serde_json::json!({"experiment": "cta", "variant": "B"})),
        ];
        let r = ExperimentResult::from_rows(&rows, "cta", None);
        assert!(r.winner.is_none());
        assert!(r.confidence.is_none());
        assert!(r.variants.iter().all(|v| v.conversions == 0));
    }

    #[test]
    fn click_heatmap_buckets_points() {
        let rows = vec![
            row("__click__", "s1", serde_json::json!({"x": 51, "y": 33, "url": "/p", "element": "BUTTON#buy"})),
            row("__click__", "s2", serde_json::json!({"x": 50, "y": 30, "url": "/p", "element": "BUTTON#buy"})),
            row("__click__", "s3", serde_json::json!({"x": 10, "y": 90, "url": "/other"})),
        ];
        let h = ClickHeatmap::from_rows(&rows, "/p", 2, 5);
        assert_eq!(h.total_clicks, 2);
        // (51->50, 33->30) and (50->50, 30->30) collapse to one bucket.
        assert_eq!(h.points.len(), 1);
        assert_eq!(h.points[0].count, 2);
        assert_eq!(h.elements[0].element, "BUTTON#buy");
        assert_eq!(h.elements[0].count, 2);
    }

    #[test]
    fn scroll_heatmap_distribution() {
        let rows = vec![
            row("__scroll__", "s1", serde_json::json!({"depth": 50, "url": "/p"})),
            row("__scroll__", "s2", serde_json::json!({"depth": 100, "url": "/p"})),
        ];
        let h = ScrollHeatmap::from_rows(&rows, "/p");
        assert_eq!(h.sessions, 2);
        assert_eq!(h.scroll_distribution[0].reached_pct, 100.0); // depth 0
        assert_eq!(h.scroll_distribution[2].reached_pct, 100.0); // depth 50
        assert_eq!(h.scroll_distribution[4].reached_pct, 50.0); // depth 100
    }

    #[test]
    fn z_test_confidence_higher_for_bigger_gap() {
        let low = two_proportion_confidence(50, 100, 49, 100);
        let high = two_proportion_confidence(80, 100, 20, 100);
        assert!(high > low);
        assert!(high > 95.0);
    }
}
