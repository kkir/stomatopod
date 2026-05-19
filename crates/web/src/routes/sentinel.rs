use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures_util::stream::Stream;
use stomatopod_core::domain::{
    control::{ControlCommand, ControlEnvelope},
    incident::{Incident, IncidentStatus, IncidentTrigger},
};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tracing::warn;
use ulid::Ulid;

use crate::state::AppState;

/// `GET /api/v1/sentinel/stream` — sidecars subscribe over SSE.
///
/// Auth: `Authorization: Bearer <sentinel_token>`. The token's site
/// determines which broadcast channel to subscribe to.
pub async fn stream_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let token_hash = stomatopod_ingest::span_handler::token_hash(bearer);
    let site_id = if let Some(id) = state.sentinel_token_cache.get(&token_hash) {
        *id
    } else {
        match state.meta.get_sentinel_token_by_hash(&token_hash).await {
            Ok(Some(tok)) => {
                state
                    .sentinel_token_cache
                    .insert(token_hash.clone(), tok.site_id);
                tok.site_id
            }
            Ok(None) => return Err(StatusCode::UNAUTHORIZED),
            Err(e) => {
                warn!("sentinel token lookup error: {e}");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    };

    let rx = state.control_channel(site_id).subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(env) => match serde_json::to_string(&env) {
            Ok(payload) => Some(Ok::<_, Infallible>(Event::default().data(payload))),
            Err(_) => None,
        },
        Err(_) => None,
    });

    Ok(Sse::new(stream).keep_alive(
        // nginx default proxy_read_timeout is 60s, ALB is 60s; emit a
        // comment line every 15s so intermediaries don't reap us.
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

/// `POST /api/v1/sentinel/control` — operators issue Kill/Hint/Resume
/// commands. Routed through the per-site broadcast channel to all
/// connected sidecars.
#[derive(serde::Deserialize)]
pub struct ControlRequest {
    pub site_id: Ulid,
    pub agent_id: String,
    #[serde(flatten)]
    pub command: ControlCommand,
}

pub async fn control_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ControlRequest>,
) -> impl IntoResponse {
    let seq = state.next_control_seq();
    let env = ControlEnvelope {
        seq,
        agent_id: req.agent_id.clone(),
        command: req.command.clone(),
    };

    // Record an Incident row for visibility in the dashboard. Failures
    // here don't block the command publish.
    let trigger = match &req.command {
        ControlCommand::Kill { .. } | ControlCommand::Hint { .. } => IncidentTrigger::Manual,
        ControlCommand::Resume => IncidentTrigger::Manual,
    };
    let incident = Incident {
        id: Ulid::new(),
        site_id: req.site_id,
        agent_id: req.agent_id.clone(),
        trigger,
        status: IncidentStatus::Open,
        opened_at: chrono::Utc::now(),
        closed_at: None,
    };
    if let Err(e) = state.meta.record_incident(&incident).await {
        warn!("control: record_incident failed: {e}");
    }
    state.alerts.dispatch(incident.clone());

    let tx = state.control_channel(req.site_id);
    let receivers = tx.send(env).unwrap_or(0);
    Json(serde_json::json!({
        "seq": seq,
        "delivered_to": receivers,
    }))
}
