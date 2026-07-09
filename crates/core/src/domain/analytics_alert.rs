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
    pub channel_id: Ulid,
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
