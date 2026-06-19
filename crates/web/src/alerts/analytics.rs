//! Analytics alert evaluation.
//!
//! Reuses the AI-firewall alert sinks (webhook/Slack): a triggered analytics
//! condition is turned into an [`Incident`] with an `AnalyticsAlert` trigger
//! and dispatched to the alert's configured channel. A background loop polls
//! enabled alerts on a fixed interval and enforces a per-alert cooldown.

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use stomatopod_core::{
    domain::{
        agent::AlertChannelKind,
        analytics_alert::{AnalyticsAlert, AnalyticsAlertFire, AnalyticsAlertKind},
        incident::{Incident, IncidentStatus, IncidentTrigger},
    },
    query::{
        analytics::GoalQuery,
        pageviews::{Granularity, PageviewsQuery, TimeRange, TopListField},
    },
    traits::{MetaStore, StorageBackend},
};
use tracing::{info, warn};
use ulid::Ulid;

use super::sinks::{AlertSink, SlackSink, TelegramSink, WebhookSink};

/// Don't re-fire the same alert within this window.
const COOLDOWN: Duration = Duration::from_secs(3600);

/// The observed metric and the threshold it crossed, for the fire payload.
#[derive(Debug, Clone, Copy)]
pub struct AlertSignal {
    pub value: f64,
    pub threshold: f64,
}

/// Pure trigger decision: given the kind/threshold and the measured `current`
/// and `baseline` metrics, decide whether the alert fires and with what value.
/// Kept side-effect-free so it can be tested directly.
pub fn decide(
    kind: AnalyticsAlertKind,
    threshold: f64,
    current: f64,
    baseline: f64,
) -> Option<AlertSignal> {
    match kind {
        AnalyticsAlertKind::TrafficSpike => {
            // New sites with no baseline can't spike-compare.
            if baseline <= 0.0 {
                return None;
            }
            let change = (current - baseline) / baseline * 100.0;
            (change >= threshold).then_some(AlertSignal {
                value: change,
                threshold,
            })
        }
        AnalyticsAlertKind::TrafficDrop => {
            if baseline <= 0.0 {
                return None;
            }
            let drop = (baseline - current) / baseline * 100.0;
            (drop >= threshold).then_some(AlertSignal {
                value: drop,
                threshold,
            })
        }
        AnalyticsAlertKind::GoalThreshold => (current >= threshold).then_some(AlertSignal {
            value: current,
            threshold,
        }),
        AnalyticsAlertKind::NewReferrerSpike => (current > threshold).then_some(AlertSignal {
            value: current,
            threshold,
        }),
    }
}

/// Gather the metrics an alert needs and run [`decide`] against them.
pub async fn evaluate_alert(
    alert: &AnalyticsAlert,
    backend: &Arc<dyn StorageBackend>,
    now: DateTime<Utc>,
) -> Option<AlertSignal> {
    let threshold = alert.config.threshold;
    let window = alert.config.window_minutes.max(1) as i64;

    let (current, baseline) = match alert.kind {
        AnalyticsAlertKind::TrafficSpike | AnalyticsAlertKind::TrafficDrop => {
            let range = TimeRange {
                start: now - chrono::Duration::minutes(window),
                end: now,
            };
            let cur = backend
                .query_pageviews(&pv_query(alert.site_id, range.clone()))
                .await
                .ok()?
                .total_pageviews as f64;
            let base = backend
                .query_pageviews(&pv_query(alert.site_id, range.previous()))
                .await
                .ok()?
                .total_pageviews as f64;
            (cur, base)
        }
        AnalyticsAlertKind::GoalThreshold => {
            let event_name = alert.config.goal_event_name.clone()?;
            // Cumulative count since midnight UTC today.
            let start = now.date_naive().and_hms_opt(0, 0, 0)?.and_utc();
            let q = GoalQuery {
                site_id: alert.site_id,
                event_name,
                filters: vec![],
                granularity: Granularity::Day,
                range: TimeRange { start, end: now },
            };
            let completions = backend.query_goal(&q).await.ok()?.completions as f64;
            (completions, 0.0)
        }
        AnalyticsAlertKind::NewReferrerSpike => {
            let range = TimeRange {
                start: now - chrono::Duration::minutes(window),
                end: now,
            };
            let top = backend
                .query_top_list(alert.site_id, TopListField::Referrer, &range, 1, &[])
                .await
                .ok()?;
            let share = top.rows.first().map(|r| r.pct).unwrap_or(0.0);
            (share, 0.0)
        }
    };

    decide(alert.kind, threshold, current, baseline)
}

fn pv_query(site_id: Ulid, range: TimeRange) -> PageviewsQuery {
    PageviewsQuery {
        site_id,
        range,
        granularity: Granularity::Hour,
        filters: vec![],
    }
}

/// Build the incident dispatched when an alert fires.
fn alert_incident(alert: &AnalyticsAlert, sig: AlertSignal, now: DateTime<Utc>) -> Incident {
    Incident {
        id: Ulid::new(),
        site_id: alert.site_id,
        agent_id: "analytics".into(),
        trigger: IncidentTrigger::AnalyticsAlert {
            alert_type: alert.kind.as_str().into(),
            value: sig.value,
            threshold: sig.threshold,
        },
        status: IncidentStatus::Open,
        opened_at: now,
        closed_at: None,
    }
}

/// Has the alert fired within the cooldown window?
async fn in_cooldown(meta: &Arc<dyn MetaStore>, alert_id: Ulid, now: DateTime<Utc>) -> bool {
    match meta.last_analytics_alert_fire(alert_id).await {
        Ok(Some(fire)) => (now - fire.fired_at)
            .to_std()
            .map(|d| d < COOLDOWN)
            .unwrap_or(false),
        _ => false,
    }
}

/// Evaluate one alert end-to-end: measure, cooldown-check, record the fire,
/// and dispatch to its channel. Returns true if it fired.
#[allow(clippy::too_many_arguments)]
pub async fn process_alert(
    alert: &AnalyticsAlert,
    backend: &Arc<dyn StorageBackend>,
    meta: &Arc<dyn MetaStore>,
    webhook: &WebhookSink,
    slack: &SlackSink,
    telegram: &TelegramSink,
    now: DateTime<Utc>,
) -> bool {
    let Some(sig) = evaluate_alert(alert, backend, now).await else {
        return false;
    };
    if in_cooldown(meta, alert.id, now).await {
        return false;
    }

    let incident = alert_incident(alert, sig, now);
    let fire = AnalyticsAlertFire {
        id: Ulid::new(),
        alert_id: alert.id,
        fired_at: now,
        payload: serde_json::json!({
            "type": alert.kind.as_str(),
            "value": sig.value,
            "threshold": sig.threshold,
        }),
    };
    if let Err(e) = meta.record_analytics_alert_fire(&fire).await {
        warn!("failed to record analytics alert fire: {e}");
    }

    // Dispatch to the alert's specific channel (best effort).
    if let Ok(channels) = meta.list_alert_channels(alert.site_id).await {
        if let Some(ch) = channels.into_iter().find(|c| c.id == alert.channel_id) {
            let sink: &dyn AlertSink = match ch.kind {
                AlertChannelKind::Webhook => webhook,
                AlertChannelKind::Slack => slack,
                AlertChannelKind::Telegram => telegram,
            };
            if let Err(e) = sink.dispatch(&ch, &incident).await {
                warn!("analytics alert dispatch failed: {e}");
            }
        }
    }
    true
}

/// Background loop: every `period`, evaluate all enabled analytics alerts.
pub async fn run_analytics_alert_evaluator(
    meta: Arc<dyn MetaStore>,
    backend: Arc<dyn StorageBackend>,
    period: Duration,
) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("reqwest client");
    let webhook = WebhookSink::new(client.clone());
    let slack = SlackSink::new(client.clone());
    let telegram = TelegramSink::new(client);
    let mut ticker = tokio::time::interval(period);

    loop {
        ticker.tick().await;
        let alerts = match meta.list_enabled_analytics_alerts().await {
            Ok(a) => a,
            Err(e) => {
                warn!("analytics alert evaluator: list failed: {e}");
                continue;
            }
        };
        let now = Utc::now();
        let mut fired = 0;
        for alert in &alerts {
            if process_alert(alert, &backend, &meta, &webhook, &slack, &telegram, now).await {
                fired += 1;
            }
        }
        if fired > 0 {
            info!("analytics alert evaluator: {fired} alert(s) fired");
        }
    }
}
