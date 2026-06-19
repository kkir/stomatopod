use std::{sync::Arc, time::Duration};

use stomatopod_core::{
    domain::{agent::AlertChannelKind, incident::Incident},
    traits::MetaStore,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::sinks::{AlertSink, SlackSink, TelegramSink, WebhookSink};

/// Wraps the producer side of the dispatch channel. Cloneable; callers
/// can stash it in AppState and emit incidents from anywhere.
#[derive(Clone)]
pub struct AlertDispatcher {
    tx: mpsc::Sender<Incident>,
}

impl AlertDispatcher {
    pub fn channel() -> (Self, mpsc::Receiver<Incident>) {
        let (tx, rx) = mpsc::channel(256);
        (Self { tx }, rx)
    }

    pub fn dispatch(&self, incident: Incident) {
        if let Err(e) = self.tx.try_send(incident) {
            warn!("alert channel full or closed: {e}");
        }
    }
}

/// Background worker. For each incident, fan out to every alert channel
/// configured for its site, with exponential-backoff retries on 5xx /
/// transport errors.
pub async fn run_alert_dispatcher(mut rx: mpsc::Receiver<Incident>, meta: Arc<dyn MetaStore>) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("reqwest client");
    let webhook = WebhookSink::new(client.clone());
    let slack = SlackSink::new(client.clone());
    let telegram = TelegramSink::new(client.clone());

    while let Some(incident) = rx.recv().await {
        let channels = match meta.list_alert_channels(incident.site_id).await {
            Ok(c) => c,
            Err(e) => {
                warn!("alert dispatch: list_alert_channels failed: {e}");
                continue;
            }
        };
        for ch in channels {
            let sink: &dyn AlertSink = match ch.kind {
                AlertChannelKind::Webhook => &webhook,
                AlertChannelKind::Slack => &slack,
                AlertChannelKind::Telegram => &telegram,
            };
            dispatch_with_retry(sink, &ch, &incident).await;
        }
    }
}

async fn dispatch_with_retry(
    sink: &dyn AlertSink,
    ch: &stomatopod_core::domain::agent::AlertChannel,
    incident: &Incident,
) {
    let mut delay = Duration::from_secs(1);
    for attempt in 1..=3 {
        match sink.dispatch(ch, incident).await {
            Ok(()) => {
                info!(
                    channel = %ch.url,
                    attempt,
                    "alert delivered for incident {}",
                    incident.id
                );
                return;
            }
            Err(e) => {
                warn!(
                    channel = %ch.url,
                    attempt,
                    "alert dispatch failed: {e}"
                );
                if attempt < 3 {
                    tokio::time::sleep(delay).await;
                    delay *= 5;
                }
            }
        }
    }
    warn!(channel = %ch.url, "alert giving up after 3 attempts");
}
