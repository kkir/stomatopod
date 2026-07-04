//! Alert-channel management JSON API. (The Tier-2 dashboard *pages* that used
//! to live here - real-time, goals, analytics alerts, campaigns, retention,
//! paths, compare - moved to the Dioxus SPA in the `/app` cutover; only the
//! channel CRUD + test-fire endpoints the SPA calls remain.)

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::domain::{
    agent::{AlertChannel, AlertChannelKind},
    incident::{Incident, IncidentStatus, IncidentTrigger},
};

use crate::{
    alerts::sinks::{AlertSink, SlackSink, TelegramSink, WebhookSink},
    middleware::auth::Principal,
    state::AppState,
};

/// Send a sample notification through a channel, used by the test-fire
/// endpoint to let operators confirm a destination is wired up correctly.
async fn dispatch_test_notification(site_id: Ulid, channel: &AlertChannel) -> Result<(), String> {
    let incident = Incident {
        id: Ulid::new(),
        site_id,
        agent_id: "test".into(),
        trigger: IncidentTrigger::AnalyticsAlert {
            alert_type: "test_notification".into(),
            value: 0.0,
            threshold: 0.0,
        },
        status: IncidentStatus::Open,
        opened_at: Utc::now(),
        closed_at: None,
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let webhook = WebhookSink::new(client.clone());
    let slack = SlackSink::new(client.clone());
    let telegram = TelegramSink::new(client);
    let sink: &dyn AlertSink = match channel.kind {
        AlertChannelKind::Webhook => &webhook,
        AlertChannelKind::Slack => &slack,
        AlertChannelKind::Telegram => &telegram,
    };
    sink.dispatch(channel, &incident)
        .await
        .map_err(|e| e.to_string())
}

// ---- Alert channels: JSON API ----
//
// Channel management is dashboard-only: destinations carry secrets
// (webhook signing keys, Telegram bot tokens) that a read/ingest-scoped API
// key shouldn't be able to configure.

/// Public view of an `AlertChannel`. Omits `secret`.
fn channel_json(c: &AlertChannel) -> serde_json::Value {
    serde_json::json!({
        "id": c.id.to_string(),
        "site_id": c.site_id.to_string(),
        "kind": c.kind.as_str(),
        "url": c.url,
        "created_at": c.created_at.to_rfc3339(),
        "last_error_at": c.last_error_at.map(|t| t.to_rfc3339()),
    })
}

fn bad_request(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "not found"})),
    )
        .into_response()
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({"error": "requires a dashboard session, not an API key"})),
    )
        .into_response()
}

/// GET /api/v1/sites/:site/alert-channels
pub async fn list_channels_api(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
) -> Response {
    if !principal.is_dashboard() {
        return forbidden();
    }
    let Ok(site_id) = Ulid::from_string(&site) else {
        return bad_request("invalid site id");
    };
    let channels = state
        .meta
        .list_alert_channels(site_id)
        .await
        .unwrap_or_default();
    Json(serde_json::json!({
        "channels": channels.iter().map(channel_json).collect::<Vec<_>>(),
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct CreateChannelBody {
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub secret: Option<String>,
}

/// POST /api/v1/sites/:site/alert-channels
pub async fn create_channel_api(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    Json(body): Json<CreateChannelBody>,
) -> Response {
    if !principal.is_dashboard() {
        return forbidden();
    }
    let Ok(site_id) = Ulid::from_string(&site) else {
        return bad_request("invalid site id");
    };
    let kind = AlertChannelKind::from_str(&body.kind);
    let url = body.url.trim().to_string();
    if url.is_empty() {
        return bad_request("destination is required");
    }
    let secret = body
        .secret
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if kind == AlertChannelKind::Telegram && secret.is_none() {
        return bad_request("telegram channel needs a bot token");
    }
    let channel = AlertChannel {
        id: Ulid::new(),
        site_id,
        kind,
        url,
        secret,
        created_at: Utc::now(),
        last_error_at: None,
    };
    match state.meta.create_alert_channel(&channel).await {
        Ok(_) => (StatusCode::CREATED, Json(channel_json(&channel))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// DELETE /api/v1/sites/:site/alert-channels/:id
pub async fn delete_channel_api(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path((site, channel_id)): Path<(String, String)>,
) -> Response {
    if !principal.is_dashboard() {
        return forbidden();
    }
    let (Ok(site_id), Ok(channel_ulid)) =
        (Ulid::from_string(&site), Ulid::from_string(&channel_id))
    else {
        return bad_request("invalid id");
    };
    let channels = state
        .meta
        .list_alert_channels(site_id)
        .await
        .unwrap_or_default();
    if !channels.iter().any(|c| c.id == channel_ulid) {
        return not_found();
    }
    match state.meta.delete_alert_channel(channel_ulid).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// POST /api/v1/sites/:site/alert-channels/:id/test — test-fire a sample
/// notification and report the outcome directly.
pub async fn test_channel_api(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path((site, channel_id)): Path<(String, String)>,
) -> Response {
    if !principal.is_dashboard() {
        return forbidden();
    }
    let (Ok(site_id), Ok(channel_ulid)) =
        (Ulid::from_string(&site), Ulid::from_string(&channel_id))
    else {
        return bad_request("invalid id");
    };
    let channels = state
        .meta
        .list_alert_channels(site_id)
        .await
        .unwrap_or_default();
    let Some(channel) = channels.into_iter().find(|c| c.id == channel_ulid) else {
        return not_found();
    };
    match dispatch_test_notification(site_id, &channel).await {
        Ok(()) => Json(serde_json::json!({"result": "ok"})).into_response(),
        Err(e) => {
            tracing::warn!(channel = %channel.id, kind = channel.kind.as_str(), "test channel failed: {e}");
            Json(serde_json::json!({"result": "fail", "error": e})).into_response()
        }
    }
}
