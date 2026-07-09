//! Email digest: stats computation, HTML rendering, delivery, and the
//! background scheduler.
//!
//! The scheduler ticks hourly and, when a cadence is due (weekly: Monday
//! 08:00 UTC; monthly: 1st 08:00 UTC), computes per-site stats for every
//! enabled subscription, renders a plain-HTML email, and hands it to a
//! [`DigestSender`]. Delivery is abstracted behind the trait so tests can
//! capture sends and self-hosted deployments can no-op until a provider is
//! configured.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use stomatopod_core::{
    domain::digest::{due_cadences, DigestFrequency, DigestSubscription},
    query::pageviews::{Granularity, PageviewsQuery, TimeRange, TopListField},
    traits::{MetaStore, StorageBackend},
};
use tracing::{info, warn};
use ulid::Ulid;

/// Auto-disable a subscription after this many consecutive bounces.
pub const BOUNCE_DISABLE_THRESHOLD: u32 = 3;

/// A rendered email ready to hand to a transport.
#[derive(Debug, Clone)]
pub struct DigestEmail {
    pub to: String,
    pub subject: String,
    pub html: String,
}

/// Pluggable email transport. Implementations must be cheap to clone-share
/// behind an `Arc`.
#[async_trait]
pub trait DigestSender: Send + Sync {
    async fn send(&self, email: DigestEmail) -> Result<(), String>;
}

/// Default sender: logs the recipient + subject and drops the body. Used
/// when no email provider is configured so the scheduler stays inert.
pub struct LogSender;

#[async_trait]
impl DigestSender for LogSender {
    async fn send(&self, email: DigestEmail) -> Result<(), String> {
        info!(to = %email.to, subject = %email.subject, "digest email (not sent: no provider)");
        Ok(())
    }
}

/// Headline numbers for one site over one window, plus the prior-window
/// comparison the email renders as delta badges.
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

fn delta_badge(delta: Option<f64>) -> String {
    match delta {
        Some(d) if d >= 0.0 => format!("<span style=\"color:#2e7d32\">+{d:.0}%</span>"),
        Some(d) => format!("<span style=\"color:#c62828\">{d:.0}%</span>"),
        None => "<span style=\"color:#888\">-</span>".to_string(),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn rows_table(rows: &[(String, u64)]) -> String {
    if rows.is_empty() {
        return "<p style=\"color:#888\">No data</p>".to_string();
    }
    let body: String = rows
        .iter()
        .map(|(label, count)| {
            format!(
                "<tr><td style=\"padding:4px 8px\">{}</td>\
                 <td style=\"padding:4px 8px;text-align:right\">{}</td></tr>",
                html_escape(label),
                count
            )
        })
        .collect();
    format!("<table style=\"width:100%;border-collapse:collapse\">{body}</table>")
}

/// Render the plain-HTML digest email body.
pub fn render_digest_html(
    site_domain: &str,
    period_label: &str,
    stats: &DigestStats,
    dashboard_url: &str,
    unsubscribe_url: &str,
) -> String {
    let activity = if stats.is_empty() {
        "<p style=\"font-size:15px;color:#888\">No activity this period.</p>".to_string()
    } else {
        format!(
            "<table style=\"width:100%;margin:16px 0\"><tr>\
               <td style=\"text-align:center\"><div style=\"font-size:28px;font-weight:700\">{pv}</div>\
                 <div style=\"color:#666\">Pageviews {pvd}</div></td>\
               <td style=\"text-align:center\"><div style=\"font-size:28px;font-weight:700\">{se}</div>\
                 <div style=\"color:#666\">Sessions {sed}</div></td>\
               <td style=\"text-align:center\"><div style=\"font-size:28px;font-weight:700\">{br:.0}%</div>\
                 <div style=\"color:#666\">Bounce rate</div></td>\
             </tr></table>",
            pv = stats.pageviews,
            pvd = delta_badge(stats.pageviews_delta_pct()),
            se = stats.sessions,
            sed = delta_badge(stats.sessions_delta_pct()),
            br = stats.bounce_rate,
        )
    };

    let country = stats
        .top_country
        .as_deref()
        .map(|c| {
            format!(
                "<p style=\"font-size:15px\">Top country: <strong>{}</strong></p>",
                html_escape(c)
            )
        })
        .unwrap_or_default();

    format!(
        "<div style=\"font-family:system-ui,sans-serif;max-width:600px;margin:0 auto;color:#222\">\
           <h1 style=\"font-size:20px\">{domain} - {period}</h1>\
           {activity}\
           {country}\
           <h2 style=\"font-size:16px;margin-top:24px\">Top pages</h2>{pages}\
           <h2 style=\"font-size:16px;margin-top:24px\">Top referrers</h2>{refs}\
           <p style=\"margin-top:24px\"><a href=\"{dash}\">View full dashboard →</a></p>\
           <hr style=\"border:none;border-top:1px solid #eee;margin:24px 0\">\
           <p style=\"font-size:12px;color:#999\">Powered by Stomatopod · \
             <a href=\"{unsub}\">Unsubscribe</a></p>\
         </div>",
        domain = html_escape(site_domain),
        period = html_escape(period_label),
        activity = activity,
        country = country,
        pages = rows_table(&stats.top_pages),
        refs = rows_table(&stats.top_referrers),
        dash = dashboard_url,
        unsub = unsubscribe_url,
    )
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

/// Build a single digest email for a subscription + cadence. Returns `None`
/// when the subscriber's site or user can't be resolved.
pub async fn build_digest_email(
    backend: &Arc<dyn StorageBackend>,
    meta: &Arc<dyn MetaStore>,
    base_url: &str,
    secret: &str,
    sub: &DigestSubscription,
    cadence: DigestFrequency,
) -> Option<DigestEmail> {
    let site = meta.get_site(sub.site_id).await.ok().flatten()?;
    let user = meta.get_user(sub.user_id).await.ok().flatten()?;
    let (range, label) = cadence_range(cadence);
    let stats = compute_digest_stats(backend, meta, sub.site_id, &range).await;
    let dashboard_url = format!("{base_url}/app/sites/{}", sub.site_id);
    let unsubscribe_url = format!(
        "{base_url}/digest/unsubscribe/{}",
        unsubscribe_token(secret, sub.id)
    );
    let html = render_digest_html(
        &site.domain,
        label,
        &stats,
        &dashboard_url,
        &unsubscribe_url,
    );
    let subject = format!("Your {} analytics — {}", site.domain, label);
    Some(DigestEmail {
        to: user.email,
        subject,
        html,
    })
}

/// Sign a one-click unsubscribe token for a subscription id.
pub fn unsubscribe_token(secret: &str, sub_id: Ulid) -> String {
    let key = blake3::derive_key("stomatopod digest unsubscribe v1", secret.as_bytes());
    let id = sub_id.to_string();
    let mac = blake3::keyed_hash(&key, id.as_bytes());
    format!("{}.{}", id, hex::encode(&mac.as_bytes()[..16]))
}

/// Verify an unsubscribe token, returning the subscription id on success.
pub fn verify_unsubscribe_token(secret: &str, token: &str) -> Option<Ulid> {
    let (id, sig) = token.split_once('.')?;
    let key = blake3::derive_key("stomatopod digest unsubscribe v1", secret.as_bytes());
    let expected = hex::encode(&blake3::keyed_hash(&key, id.as_bytes()).as_bytes()[..16]);
    if crate::middleware::auth::constant_time_eq(sig.as_bytes(), expected.as_bytes()) {
        Ulid::from_string(id).ok()
    } else {
        None
    }
}

/// Deliver every due digest for `cadence`, returning the number sent. A
/// `Both` subscription is delivered for whichever concrete cadence is due.
pub async fn dispatch_cadence(
    meta: &Arc<dyn MetaStore>,
    backend: &Arc<dyn StorageBackend>,
    sender: &Arc<dyn DigestSender>,
    base_url: &str,
    secret: &str,
    cadence: DigestFrequency,
) -> usize {
    let subs = match meta.list_enabled_digest_subscriptions().await {
        Ok(s) => s,
        Err(e) => {
            warn!("digest: listing subscriptions failed: {e}");
            return 0;
        }
    };
    let mut sent = 0;
    for sub in subs {
        let wants = match cadence {
            DigestFrequency::Weekly => sub.frequency.wants_weekly(),
            DigestFrequency::Monthly => sub.frequency.wants_monthly(),
            DigestFrequency::Both => continue,
        };
        if !wants {
            continue;
        }
        if let Some(email) =
            build_digest_email(backend, meta, base_url, secret, &sub, cadence).await
        {
            match sender.send(email).await {
                Ok(()) => sent += 1,
                Err(e) => {
                    warn!("digest: send failed for {}: {e}", sub.id);
                    let _ = meta
                        .record_digest_bounce(sub.id, BOUNCE_DISABLE_THRESHOLD)
                        .await;
                }
            }
        }
    }
    sent
}

/// Background loop: tick hourly and dispatch any digest cadence due now.
pub async fn run_digest_scheduler(
    meta: Arc<dyn MetaStore>,
    backend: Arc<dyn StorageBackend>,
    sender: Arc<dyn DigestSender>,
    base_url: String,
    secret: String,
    period: Duration,
) {
    let mut ticker = tokio::time::interval(period);
    loop {
        ticker.tick().await;
        let due = due_cadences(Utc::now());
        for cadence in due {
            let n = dispatch_cadence(&meta, &backend, &sender, &base_url, &secret, cadence).await;
            if n > 0 {
                info!("digest: dispatched {n} {} email(s)", cadence.as_str());
            }
        }
    }
}
