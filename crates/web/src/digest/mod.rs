//! Analytics digest: stats computation, plain-text rendering, and delivery
//! through each site's configured notification channels (Slack, Telegram,
//! webhook).
//!
//! The scheduler ticks hourly and, when a cadence is due (weekly: Monday
//! 08:00 UTC; monthly: 1st 08:00 UTC), computes per-site stats for every
//! enabled subscription and posts a summary to that site's alert channels.
//! Delivery is abstracted behind [`DigestNotifier`] so tests can capture
//! sends without hitting the network.

use std::{collections::HashSet, sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use stomatopod_core::{
    domain::{
        agent::AlertChannel,
        digest::{due_cadences, DigestFrequency, DigestSubscription},
    },
    query::pageviews::{Granularity, PageviewsQuery, TimeRange, TopListField},
    traits::{MetaStore, StorageBackend},
};
use tracing::{info, warn};
use ulid::Ulid;

use crate::alerts::sinks::{alert_http_client, deliver_text};

/// Auto-disable a subscription after this many consecutive delivery failures.
pub const BOUNCE_DISABLE_THRESHOLD: u32 = 3;

/// A rendered digest ready to hand to notification channels.
#[derive(Debug, Clone)]
pub struct DigestMessage {
    pub site_id: Ulid,
    pub subject: String,
    pub text: String,
}

/// Pluggable digest transport. Production uses [`ChannelNotifier`]; tests
/// capture messages without outbound HTTP.
#[async_trait]
pub trait DigestNotifier: Send + Sync {
    /// Deliver `msg` to every channel in `channels`. Returns `Ok` if at least
    /// one channel accepted the message (or if the implementation does not
    /// require channels, e.g. a test capture sink).
    async fn send(&self, channels: &[AlertChannel], msg: DigestMessage) -> Result<(), String>;
}

/// Production notifier: posts through Slack / Telegram / webhook sinks.
pub struct ChannelNotifier;

#[async_trait]
impl DigestNotifier for ChannelNotifier {
    async fn send(&self, channels: &[AlertChannel], msg: DigestMessage) -> Result<(), String> {
        if channels.is_empty() {
            return Err("no notification channels configured for this site".into());
        }
        let client = alert_http_client();
        let mut any_ok = false;
        let mut last_err: Option<String> = None;
        for ch in channels {
            match deliver_text(ch, &msg.subject, &msg.text, &client).await {
                Ok(()) => any_ok = true,
                Err(e) => {
                    warn!(
                        site_id = %msg.site_id,
                        channel = %ch.id,
                        kind = ch.kind.as_str(),
                        "digest channel delivery failed: {e}"
                    );
                    last_err = Some(e.to_string());
                }
            }
        }
        if any_ok {
            Ok(())
        } else {
            Err(last_err.unwrap_or_else(|| "all channel deliveries failed".into()))
        }
    }
}

/// Headline numbers for one site over one window, plus the prior-window
/// comparison rendered as delta badges.
#[derive(Debug, Clone, Default)]
pub struct DigestStats {
    pub pageviews: u64,
    pub prev_pageviews: u64,
    pub sessions: u64,
    pub prev_sessions: u64,
    pub bounce_rate: f64,
    pub top_pages: Vec<(String, u64)>,
    pub top_referrers: Vec<(String, u64)>,
    pub top_country: Option<String>,
}

impl DigestStats {
    /// Percent change vs the prior period, `None` when there's no baseline.
    pub fn pageviews_delta_pct(&self) -> Option<f64> {
        pct_delta(self.pageviews, self.prev_pageviews)
    }
    pub fn sessions_delta_pct(&self) -> Option<f64> {
        pct_delta(self.sessions, self.prev_sessions)
    }
    /// True when the site saw no traffic in the window.
    pub fn is_empty(&self) -> bool {
        self.pageviews == 0 && self.sessions == 0
    }
}

fn pct_delta(now: u64, prev: u64) -> Option<f64> {
    if prev == 0 {
        return None;
    }
    Some((now as f64 - prev as f64) / prev as f64 * 100.0)
}

/// Compute digest stats for a site over `range`, including the prior-period
/// totals for delta badges.
pub async fn compute_digest_stats(
    backend: &Arc<dyn StorageBackend>,
    _meta: &Arc<dyn MetaStore>,
    site_id: Ulid,
    range: &TimeRange,
) -> DigestStats {
    let prev = range.previous();

    let cur = backend
        .query_pageviews(&PageviewsQuery {
            site_id,
            range: range.clone(),
            granularity: Granularity::Day,
            filters: vec![],
        })
        .await
        .unwrap_or_default();
    let prior = backend
        .query_pageviews(&PageviewsQuery {
            site_id,
            range: prev.clone(),
            granularity: Granularity::Day,
            filters: vec![],
        })
        .await
        .unwrap_or_default();

    let top_pages = backend
        .query_top_list(site_id, TopListField::Page, range, 3, &[])
        .await
        .map(|t| t.rows.into_iter().map(|r| (r.value, r.pageviews)).collect())
        .unwrap_or_default();
    let top_referrers = backend
        .query_top_list(site_id, TopListField::Referrer, range, 3, &[])
        .await
        .map(|t| t.rows.into_iter().map(|r| (r.value, r.pageviews)).collect())
        .unwrap_or_default();
    let top_country = backend
        .query_top_list(site_id, TopListField::Country, range, 1, &[])
        .await
        .ok()
        .and_then(|t| t.rows.into_iter().next().map(|r| r.value));

    DigestStats {
        pageviews: cur.total_pageviews,
        prev_pageviews: prior.total_pageviews,
        sessions: cur.total_sessions,
        prev_sessions: prior.total_sessions,
        bounce_rate: cur.bounce_rate,
        top_pages,
        top_referrers,
        top_country,
    }
}

fn delta_label(delta: Option<f64>) -> String {
    match delta {
        Some(d) if d >= 0.0 => format!("(+{d:.0}%)"),
        Some(d) => format!("({d:.0}%)"),
        None => String::new(),
    }
}

fn format_rows(rows: &[(String, u64)]) -> String {
    if rows.is_empty() {
        return "  (none)".into();
    }
    rows.iter()
        .map(|(label, count)| format!("  • {label}: {count}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render a plain-text digest body suitable for Slack, Telegram, or webhooks.
pub fn render_digest_text(
    site_domain: &str,
    period_label: &str,
    stats: &DigestStats,
    dashboard_url: &str,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("{site_domain} - {period_label}\n\n"));

    if stats.is_empty() {
        out.push_str("No activity this period.\n");
    } else {
        out.push_str(&format!(
            "Pageviews: {} {}\nSessions: {} {}\nBounce rate: {:.0}%\n",
            stats.pageviews,
            delta_label(stats.pageviews_delta_pct()),
            stats.sessions,
            delta_label(stats.sessions_delta_pct()),
            stats.bounce_rate,
        ));
        if let Some(c) = stats.top_country.as_deref() {
            out.push_str(&format!("Top country: {c}\n"));
        }
        out.push_str("\nTop pages:\n");
        out.push_str(&format_rows(&stats.top_pages));
        out.push_str("\n\nTop referrers:\n");
        out.push_str(&format_rows(&stats.top_referrers));
        out.push('\n');
    }

    out.push_str(&format!("\nDashboard: {dashboard_url}"));
    out
}

/// Window + human label for a cadence (weekly = 7d, monthly = 30d).
fn cadence_range(cadence: DigestFrequency) -> (TimeRange, &'static str) {
    match cadence {
        DigestFrequency::Monthly => (TimeRange::from_label("30d"), "last 30 days"),
        // Both is only used for subscription storage; a delivery is always
        // for a concrete weekly or monthly cadence.
        _ => (TimeRange::from_label("7d"), "last 7 days"),
    }
}

/// Build a single digest message for a site + cadence. Returns `None` when
/// the site can't be resolved.
pub async fn build_digest_message(
    backend: &Arc<dyn StorageBackend>,
    meta: &Arc<dyn MetaStore>,
    base_url: &str,
    site_id: Ulid,
    cadence: DigestFrequency,
) -> Option<DigestMessage> {
    let site = meta.get_site(site_id).await.ok().flatten()?;
    let (range, label) = cadence_range(cadence);
    let stats = compute_digest_stats(backend, meta, site_id, &range).await;
    let dashboard_url = format!("{base_url}/app/sites/{site_id}");
    let text = render_digest_text(&site.domain, label, &stats, &dashboard_url);
    let subject = format!("Your {} analytics - {}", site.domain, label);
    Some(DigestMessage {
        site_id,
        subject,
        text,
    })
}

/// Deliver every due digest for `cadence`, returning the number of sites
/// successfully notified. One message is sent per site that has at least one
/// matching subscription (even if multiple users opted in).
pub async fn dispatch_cadence(
    meta: &Arc<dyn MetaStore>,
    backend: &Arc<dyn StorageBackend>,
    notifier: &Arc<dyn DigestNotifier>,
    base_url: &str,
    cadence: DigestFrequency,
) -> usize {
    let subs = match meta.list_enabled_digest_subscriptions().await {
        Ok(s) => s,
        Err(e) => {
            warn!("digest: listing subscriptions failed: {e}");
            return 0;
        }
    };

    // One delivery per site per cadence (dedupe multi-user opt-ins).
    let mut site_subs: Vec<(Ulid, DigestSubscription)> = Vec::new();
    let mut seen = HashSet::new();
    for sub in subs {
        let wants = match cadence {
            DigestFrequency::Weekly => sub.frequency.wants_weekly(),
            DigestFrequency::Monthly => sub.frequency.wants_monthly(),
            DigestFrequency::Both => continue,
        };
        if !wants {
            continue;
        }
        if seen.insert(sub.site_id) {
            site_subs.push((sub.site_id, sub));
        }
    }

    let mut sent = 0;
    for (site_id, sub) in site_subs {
        let channels = match meta.list_alert_channels(site_id).await {
            Ok(c) => c,
            Err(e) => {
                warn!("digest: listing channels for {site_id} failed: {e}");
                continue;
            }
        };
        let Some(msg) = build_digest_message(backend, meta, base_url, site_id, cadence).await
        else {
            continue;
        };
        match notifier.send(&channels, msg).await {
            Ok(()) => sent += 1,
            Err(e) => {
                warn!("digest: send failed for site {site_id}: {e}");
                let _ = meta
                    .record_digest_bounce(sub.id, BOUNCE_DISABLE_THRESHOLD)
                    .await;
            }
        }
    }
    sent
}

/// Background loop: tick hourly and dispatch any digest cadence due now.
pub async fn run_digest_scheduler(
    meta: Arc<dyn MetaStore>,
    backend: Arc<dyn StorageBackend>,
    notifier: Arc<dyn DigestNotifier>,
    base_url: String,
    period: Duration,
) {
    let mut ticker = tokio::time::interval(period);
    loop {
        ticker.tick().await;
        let due = due_cadences(Utc::now());
        for cadence in due {
            let n = dispatch_cadence(&meta, &backend, &notifier, &base_url, cadence).await;
            if n > 0 {
                info!(
                    "digest: dispatched {n} {} digest(s) via notification channels",
                    cadence.as_str()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_includes_stats_and_dashboard() {
        let stats = DigestStats {
            pageviews: 100,
            prev_pageviews: 80,
            sessions: 40,
            prev_sessions: 50,
            bounce_rate: 33.0,
            top_pages: vec![("/".into(), 60), ("/about".into(), 20)],
            top_referrers: vec![("google".into(), 30)],
            top_country: Some("US".into()),
        };
        let text = render_digest_text("example.com", "last 7 days", &stats, "http://x/dash");
        assert!(text.contains("example.com - last 7 days"));
        assert!(text.contains("Pageviews: 100 (+25%)"));
        assert!(text.contains("Sessions: 40 (-20%)"));
        assert!(text.contains("Top country: US"));
        assert!(text.contains("• /: 60"));
        assert!(text.contains("Dashboard: http://x/dash"));
    }

    #[test]
    fn render_empty_period() {
        let text = render_digest_text(
            "example.com",
            "last 7 days",
            &DigestStats::default(),
            "http://x",
        );
        assert!(text.contains("No activity this period"));
    }
}
