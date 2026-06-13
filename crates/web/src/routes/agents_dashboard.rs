use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
};
use chrono::{Duration, Utc};
use minijinja::context;
use serde::Deserialize;
use stomatopod_core::{domain::incident::IncidentTrigger, query::spans::SpanQuery};
use ulid::Ulid;

use crate::{error::AppError, state::AppState, templates};

/// Helper: fetch the first site for the bootstrap org and return its
/// id. Self-hosted single-tenant simplification.
async fn first_site_id(state: &Arc<AppState>) -> Option<Ulid> {
    let orgs = state.meta.list_orgs().await.ok()?;
    let org = orgs.into_iter().next()?;
    let sites = state.meta.list_sites(org.id).await.ok()?;
    sites.into_iter().next().map(|s| s.id)
}

pub async fn agents_index(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    let Some(site_id) = first_site_id(&state).await else {
        return Ok(Html("<p>No sites configured yet.</p>").into_response());
    };
    let since = Utc::now() - Duration::hours(24);
    // `unwrap_or_default()` is intentional: when the site has zero spans
    // the Parquet table isn't yet registered with DataFusion, which
    // surfaces as a query error rather than an empty result. Render the
    // empty-state page in that case instead of returning 500.
    let summaries = state
        .agent_store
        .summarize_agents(site_id, since)
        .await
        .unwrap_or_default();
    let html = templates::render(
        &state,
        "agents.jinja",
        context! {
            summaries => serde_json::to_value(&summaries).unwrap(),
        },
    )?;
    Ok(html.into_response())
}

pub async fn agent_detail(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> Result<Response, AppError> {
    let Some(site_id) = first_site_id(&state).await else {
        return Err(AppError::NotFound("no site configured"));
    };
    let since = Utc::now() - Duration::hours(24);
    // Same empty-state handling as `agents_index` — treat a missing
    // spans table as zero spans rather than a server error.
    let summaries = state
        .agent_store
        .summarize_agents(site_id, since)
        .await
        .unwrap_or_default();
    let summary = summaries.into_iter().find(|s| s.agent_id == agent_id);
    let (spans, in_tok, out_tok, cost) = match summary {
        Some(s) => (
            s.total_spans,
            s.total_input_tokens,
            s.total_output_tokens,
            s.total_cost_usd,
        ),
        None => (0u64, 0u64, 0u64, 0.0f64),
    };
    let html = templates::render(
        &state,
        "agent.jinja",
        context! {
            site_id => site_id.to_string(),
            agent_id => agent_id,
            total_spans => spans,
            total_input_tokens => in_tok,
            total_output_tokens => out_tok,
            total_cost_usd => cost,
        },
    )?;
    Ok(html.into_response())
}

#[derive(Deserialize)]
pub struct SpansPartialQuery {
    pub site_id: String,
}

pub async fn agent_spans_partial(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Query(q): Query<SpansPartialQuery>,
) -> Result<Response, AppError> {
    let site_id =
        Ulid::from_string(&q.site_id).map_err(|_| AppError::BadRequest("invalid site id"))?;
    let now = Utc::now();
    let query = SpanQuery {
        site_id,
        agent_id: Some(agent_id),
        session_id: None,
        since: now - Duration::hours(24),
        until: now,
        limit: 100,
    };
    let rows = state
        .agent_store
        .query_spans(&query)
        .await
        .unwrap_or_default();
    let html = templates::render(
        &state,
        "partials/agent_spans.jinja",
        context! { rows => serde_json::to_value(&rows).unwrap() },
    )?;
    Ok(html.into_response())
}

/// JSON-friendly view of an incident row for the dashboard template.
/// Centralised so the template doesn't need to dispatch on the
/// `IncidentTrigger` variant tag itself.
#[derive(serde::Serialize)]
struct IncidentView {
    opened_at: String,
    agent_id: String,
    trigger_summary: String,
    status: &'static str,
}

pub async fn incidents_page(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    let Some(site_id) = first_site_id(&state).await else {
        return Ok(Html("<p>No sites configured.</p>").into_response());
    };
    let incidents = state.meta.list_incidents(site_id, 100).await?;
    let view: Vec<IncidentView> = incidents
        .into_iter()
        .map(|i| IncidentView {
            opened_at: i.opened_at.to_rfc3339(),
            agent_id: i.agent_id,
            trigger_summary: summarize_trigger(&i.trigger),
            status: i.status.as_str(),
        })
        .collect();
    let html = templates::render(
        &state,
        "incidents.jinja",
        context! { incidents => serde_json::to_value(&view).unwrap() },
    )?;
    Ok(html.into_response())
}

fn summarize_trigger(t: &IncidentTrigger) -> String {
    match t {
        IncidentTrigger::Repetition { count, args_hash } => {
            format!("repetition ({count}× {args_hash})")
        }
        IncidentTrigger::TokenVelocity { tokens_per_sec } => {
            format!("token velocity {tokens_per_sec:.0}/s")
        }
        IncidentTrigger::CostThreshold { usd } => format!("cost ${usd:.2}"),
        IncidentTrigger::Manual => "manual".into(),
    }
}
