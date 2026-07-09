use async_trait::async_trait;
use serde_json::json;
use stomatopod_core::domain::{
    agent::{AlertChannel, AlertChannelKind},
    incident::{Incident, IncidentTrigger},
};

use super::ssrf::validate_outbound_url;

/// Outbound destination for an incident notification.
#[async_trait]
pub trait AlertSink: Send + Sync {
    async fn dispatch(&self, channel: &AlertChannel, incident: &Incident) -> anyhow::Result<()>;
}

/// Build an HTTP client for alert delivery: short timeout, no redirects
/// (redirects to private IPs would bypass pre-flight SSRF checks).
pub fn alert_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

/// Generic webhook: POST application/json with an HMAC-blake3 signature
/// header when the channel has a secret configured. Receivers can
/// verify by re-hashing the body with the same key.
pub struct WebhookSink {
    pub client: reqwest::Client,
}

impl WebhookSink {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl AlertSink for WebhookSink {
    async fn dispatch(&self, channel: &AlertChannel, incident: &Incident) -> anyhow::Result<()> {
        validate_outbound_url(&channel.url).map_err(anyhow::Error::msg)?;
        let body = incident_payload(incident);
        let bytes = serde_json::to_vec(&body)?;
        let mut req = self
            .client
            .post(&channel.url)
            .header("content-type", "application/json");
        if let Some(secret) = &channel.secret {
            let sig = sign_blake3(secret.as_bytes(), &bytes);
            req = req.header("x-stomatopod-signature", sig);
        }
        let resp = req.body(bytes).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("webhook returned {}", resp.status());
        }
        Ok(())
    }
}

/// Slack incoming webhook formatter. Slack ignores extra headers and
/// expects a specific `blocks` shape; we keep it minimal and readable.
pub struct SlackSink {
    pub client: reqwest::Client,
}

impl SlackSink {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl AlertSink for SlackSink {
    async fn dispatch(&self, channel: &AlertChannel, incident: &Incident) -> anyhow::Result<()> {
        validate_outbound_url(&channel.url).map_err(anyhow::Error::msg)?;
        let trigger_line = format_trigger(&incident.trigger);
        let body = json!({
            "blocks": [
                {
                    "type": "header",
                    "text": {"type": "plain_text", "text": "Stomatopod Alert"}
                },
                {
                    "type": "section",
                    "fields": [
                        {"type": "mrkdwn", "text": format!("*Agent:* `{}`", incident.agent_id)},
                        {"type": "mrkdwn", "text": format!("*Trigger:* {trigger_line}")},
                        {"type": "mrkdwn", "text": format!("*Opened:* {}", incident.opened_at.to_rfc3339())},
                        {"type": "mrkdwn", "text": format!("*Status:* {}", incident.status.as_str())}
                    ]
                }
            ]
        });
        let resp = self.client.post(&channel.url).json(&body).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("slack returned {}", resp.status());
        }
        Ok(())
    }
}

/// Telegram bot sink. The channel's `url` holds the chat id and `secret`
/// holds the bot token; we POST to the Bot API `sendMessage` method.
pub struct TelegramSink {
    pub client: reqwest::Client,
}

impl TelegramSink {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl AlertSink for TelegramSink {
    async fn dispatch(&self, channel: &AlertChannel, incident: &Incident) -> anyhow::Result<()> {
        let token = channel
            .secret
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("telegram channel missing bot token"))?;
        let chat_id = &channel.url;
        // Plain text — no `parse_mode`. Trigger/agent strings can contain
        // Markdown metacharacters (e.g. `_` in event names), which the
        // Telegram entity parser rejects with a 400.
        let text = format!(
            "Stomatopod alert\nAgent: {}\nTrigger: {}\nStatus: {}",
            incident.agent_id,
            format_trigger(&incident.trigger),
            incident.status.as_str()
        );
        let api = format!("https://api.telegram.org/bot{token}/sendMessage");
        let body = json!({ "chat_id": chat_id, "text": text });
        let resp = self.client.post(api).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            // Surface Telegram's `description` so config errors are diagnosable.
            let detail = resp.text().await.unwrap_or_default();
            anyhow::bail!("telegram returned {status}: {detail}");
        }
        Ok(())
    }
}

pub fn select_sink(
    channel: &AlertChannel,
    webhook: &WebhookSink,
    slack: &SlackSink,
) -> &'static str {
    let _ = (webhook, slack);
    match channel.kind {
        AlertChannelKind::Webhook => "webhook",
        AlertChannelKind::Slack => "slack",
        AlertChannelKind::Telegram => "telegram",
    }
}

pub fn incident_payload(incident: &Incident) -> serde_json::Value {
    json!({
        "id": incident.id.to_string(),
        "site_id": incident.site_id.to_string(),
        "agent_id": incident.agent_id,
        "trigger": format_trigger(&incident.trigger),
        "trigger_kind": incident.trigger.kind_str(),
        "status": incident.status.as_str(),
        "opened_at": incident.opened_at.to_rfc3339(),
    })
}

fn format_trigger(t: &IncidentTrigger) -> String {
    match t {
        IncidentTrigger::AnalyticsAlert {
            alert_type,
            value,
            threshold,
        } => format!("{alert_type} (value {value:.1}, threshold {threshold:.1})"),
    }
}

fn sign_blake3(key: &[u8], body: &[u8]) -> String {
    // Derive a 32-byte key from the user-supplied secret (allows
    // arbitrary length secrets) then keyed-hash the body.
    let derived = blake3::derive_key("stomatopod alert signing v1", key);
    blake3::keyed_hash(&derived, body).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use stomatopod_core::domain::incident::IncidentStatus;
    use ulid::Ulid;

    fn mock_incident() -> Incident {
        Incident {
            id: Ulid::new(),
            site_id: Ulid::new(),
            agent_id: "analytics".into(),
            trigger: IncidentTrigger::AnalyticsAlert {
                alert_type: "traffic_spike".into(),
                value: 200.0,
                threshold: 100.0,
            },
            status: IncidentStatus::Open,
            opened_at: Utc::now(),
            closed_at: None,
        }
    }

    #[test]
    fn payload_contains_essentials() {
        let inc = mock_incident();
        let p = incident_payload(&inc);
        assert_eq!(p["agent_id"], "analytics");
        assert_eq!(p["trigger_kind"], "analytics_alert");
        assert!(p["trigger"].as_str().unwrap().contains("traffic_spike"));
    }

    #[test]
    fn signature_deterministic() {
        let s1 = sign_blake3(b"shh", b"hello");
        let s2 = sign_blake3(b"shh", b"hello");
        assert_eq!(s1, s2);
        let s3 = sign_blake3(b"different", b"hello");
        assert_ne!(s1, s3);
    }
}
