use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertChannel {
    pub id: Ulid,
    pub site_id: Ulid,
    pub kind: AlertChannelKind,
    /// Destination URL. For webhooks/Slack this is the POST target.
    pub url: String,
    /// Optional shared secret used to sign outgoing payloads.
    pub secret: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_error_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertChannelKind {
    Webhook,
    Slack,
    /// Telegram bot: `url` holds the chat id, `secret` the bot token.
    Telegram,
}

impl AlertChannelKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertChannelKind::Webhook => "webhook",
            AlertChannelKind::Slack => "slack",
            AlertChannelKind::Telegram => "telegram",
        }
    }

    /// Parse the stored token; unknown values fall back to `Webhook` (the
    /// most permissive sink), matching the backend row decoders.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "slack" => AlertChannelKind::Slack,
            "telegram" => AlertChannelKind::Telegram,
            _ => AlertChannelKind::Webhook,
        }
    }
}
