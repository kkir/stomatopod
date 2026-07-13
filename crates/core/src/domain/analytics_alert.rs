use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// What kind of analytics condition an alert watches for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalyticsAlertKind {
    /// Pageviews in the window exceed the baseline by `threshold` percent.
    TrafficSpike,
    /// Pageviews in the window fall below the baseline by `threshold` percent.
    TrafficDrop,
    /// A single referrer accounts for more than `threshold` percent of
    /// traffic in the window (viral spike detection).
    NewReferrerSpike,
}

impl AnalyticsAlertKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AnalyticsAlertKind::TrafficSpike => "traffic_spike",
            AnalyticsAlertKind::TrafficDrop => "traffic_drop",
            AnalyticsAlertKind::NewReferrerSpike => "new_referrer_spike",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "traffic_spike" => AnalyticsAlertKind::TrafficSpike,
            "traffic_drop" => AnalyticsAlertKind::TrafficDrop,
            "new_referrer_spike" => AnalyticsAlertKind::NewReferrerSpike,
            _ => return None,
        })
    }
}

/// Type-specific tuning. Stored as a JSON blob (`config` column) so the
/// schema needn't change as alert kinds gain parameters.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnalyticsAlertConfig {
    /// Percent change (spike/drop/referrer).
    pub threshold: f64,
    /// Evaluation window in minutes.
    #[serde(default)]
    pub window_minutes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsAlert {
    pub id: Ulid,
    pub site_id: Ulid,
    pub kind: AnalyticsAlertKind,
    pub config: AnalyticsAlertConfig,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

/// A recorded firing of an alert. Powers the 1-hour cooldown and the
/// "last fired" UI column.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsAlertFire {
    pub id: Ulid,
    pub alert_id: Ulid,
    pub fired_at: DateTime<Utc>,
    pub payload: serde_json::Value,
}

/// Tasteful starter rules for a new site. Enabled so they start working as
/// soon as a notification destination is configured.
///
/// Thresholds are percent points as used by the evaluator:
/// - spike/drop: percent change vs the previous window baseline
/// - referrer spike: share of traffic from a single referrer
pub fn default_analytics_alerts(site_id: Ulid) -> Vec<AnalyticsAlert> {
    let now = Utc::now();
    // Stagger created_at so list order (DESC) is stable and readable.
    let mk = |kind: AnalyticsAlertKind, threshold: f64, window_minutes: u32, secs_ago: i64| {
        AnalyticsAlert {
            id: Ulid::new(),
            site_id,
            kind,
            config: AnalyticsAlertConfig {
                threshold,
                window_minutes,
            },
            enabled: true,
            created_at: now - chrono::Duration::seconds(secs_ago),
        }
    };
    vec![
        // 2x traffic vs the prior window over an hour - catches launches,
        // outages recovering, and sudden campaign traffic.
        mk(AnalyticsAlertKind::TrafficSpike, 100.0, 60, 0),
        // ~half the usual volume - outages, broken deploy, tracking loss.
        mk(AnalyticsAlertKind::TrafficDrop, 50.0, 60, 1),
        // One referrer driving over a third of hour-window traffic - viral
        // posts, bot scrapes, or a broken inbound campaign.
        mk(AnalyticsAlertKind::NewReferrerSpike, 35.0, 60, 2),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_alerts_cover_common_anomalies() {
        let site = Ulid::new();
        let alerts = default_analytics_alerts(site);
        assert_eq!(alerts.len(), 3);
        assert!(alerts.iter().all(|a| a.site_id == site && a.enabled));
        let kinds: Vec<_> = alerts.iter().map(|a| a.kind).collect();
        assert!(kinds.contains(&AnalyticsAlertKind::TrafficSpike));
        assert!(kinds.contains(&AnalyticsAlertKind::TrafficDrop));
        assert!(kinds.contains(&AnalyticsAlertKind::NewReferrerSpike));
    }
}
