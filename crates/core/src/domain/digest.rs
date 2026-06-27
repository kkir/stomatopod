use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// How often a user wants a site digest emailed to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestFrequency {
    Weekly,
    Monthly,
    Both,
}

impl DigestFrequency {
    pub fn as_str(&self) -> &'static str {
        match self {
            DigestFrequency::Weekly => "weekly",
            DigestFrequency::Monthly => "monthly",
            DigestFrequency::Both => "both",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "weekly" => DigestFrequency::Weekly,
            "monthly" => DigestFrequency::Monthly,
            "both" => DigestFrequency::Both,
            _ => return None,
        })
    }

    /// Whether this subscription wants the weekly cadence.
    pub fn wants_weekly(&self) -> bool {
        matches!(self, DigestFrequency::Weekly | DigestFrequency::Both)
    }

    /// Whether this subscription wants the monthly cadence.
    pub fn wants_monthly(&self) -> bool {
        matches!(self, DigestFrequency::Monthly | DigestFrequency::Both)
    }
}

/// An opt-in subscription to periodic email digests for a site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigestSubscription {
    pub id: Ulid,
    pub user_id: Ulid,
    pub site_id: Ulid,
    pub frequency: DigestFrequency,
    pub enabled: bool,
    /// Consecutive delivery bounces; the subscription is auto-disabled at 3.
    pub bounce_count: u32,
    pub created_at: DateTime<Utc>,
}

/// The digest cadences that are due to run at `now` (UTC fallback timezone):
/// weekly fires Monday at 08:00, monthly fires on the 1st at 08:00. The
/// scheduler ticks hourly, so matching the hour (not the minute) is enough.
pub fn due_cadences(now: DateTime<Utc>) -> Vec<DigestFrequency> {
    let mut out = Vec::new();
    if now.hour() == 8 {
        if now.weekday() == chrono::Weekday::Mon {
            out.push(DigestFrequency::Weekly);
        }
        if now.day() == 1 {
            out.push(DigestFrequency::Monthly);
        }
    }
    out
}
