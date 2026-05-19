use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use chrono::{Duration, Utc};
use minijinja::context;
use serde::Deserialize;
use stomatopod_core::{domain::incident::IncidentTrigger, query::spans::SpanQuery};
use ulid::Ulid;

use crate::state::AppState;

/// Helper: fetch the first site for the bootstrap org and return its
/// id. Self-hosted single-tenant simplification.
async fn first_site_id(state: &Arc<AppState>) -> Option<Ulid> {
    let orgs = state.meta.list_orgs().await.ok()?;
    let org = orgs.into_iter().next()?;
    let mut sites = state.meta.list_sites(org.id).await.ok()?;
    sites.pop().map(|s| s.id)
}

pub async fn agents_index(State(state): State<Arc<AppState>>) -> Response {
    let Some(site_id) = first_site_id(&state).await else {
        return Html("<p>No sites configured yet.</p>").into_response();
    };
    let since = Utc::now() - Duration::hours(24);
    let summaries = state
        .agent_store
        .summarize_agents(site_id, since)
        .await
        .unwrap_or_default();
    let tmpl = state.templates.get_template("agents.html").unwrap();
    let html = tmpl
        .render(context! {
            summaries => serde_json::to_value(&summaries).unwrap()
        })
        .unwrap_or_else(|e| format!("template error: {e}"));
    Html(html).into_response()
}

pub async fn agent_detail(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> Response {
    let Some(site_id) = first_site_id(&state).await else {
        return (StatusCode::NOT_FOUND, "no site").into_response();
    };
    let since = Utc::now() - Duration::hours(24);
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
    let tmpl = state.templates.get_template("agent.html").unwrap();
    let html = tmpl
        .render(context! {
            site_id => site_id.to_string(),
            agent_id => agent_id,
            total_spans => spans,
            total_input_tokens => in_tok,
            total_output_tokens => out_tok,
            total_cost_usd => cost,
        })
        .unwrap_or_else(|e| format!("template error: {e}"));
    Html(html).into_response()
}

#[derive(Deserialize)]
pub struct SpansPartialQuery {
    pub site_id: String,
}

pub async fn agent_spans_partial(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Query(q): Query<SpansPartialQuery>,
) -> Response {
    let Ok(site_id) = Ulid::from_string(&q.site_id) else {
        return (StatusCode::BAD_REQUEST, "bad site_id").into_response();
    };
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
    let tmpl = state
        .templates
        .get_template("partials/agent_spans.html")
        .unwrap();
    Html(
        tmpl.render(context! { rows => serde_json::to_value(&rows).unwrap() })
            .unwrap_or_else(|e| format!("template error: {e}")),
    )
    .into_response()
}

pub async fn incidents_page(State(state): State<Arc<AppState>>) -> Response {
    let Some(site_id) = first_site_id(&state).await else {
        return Html("<p>No sites configured.</p>").into_response();
    };
    let incidents = state
        .meta
        .list_incidents(site_id, 100)
        .await
        .unwrap_or_default();
    let view: Vec<serde_json::Value> = incidents
        .into_iter()
        .map(|i| {
            let trigger_summary = match &i.trigger {
                IncidentTrigger::Repetition { count, args_hash } => {
                    format!("repetition ({count}× {args_hash})")
                }
                IncidentTrigger::TokenVelocity { tokens_per_sec } => {
                    format!("token velocity {tokens_per_sec:.0}/s")
                }
                IncidentTrigger::CostThreshold { usd } => {
                    format!("cost ${usd:.2}")
                }
                IncidentTrigger::Manual => "manual".into(),
            };
            serde_json::json!({
                "opened_at": i.opened_at.to_rfc3339(),
                "agent_id": i.agent_id,
                "trigger_summary": trigger_summary,
                "status": i.status.as_str(),
            })
        })
        .collect();
    let tmpl = state.templates.get_template("incidents.html").unwrap();
    Html(
        tmpl.render(context! { incidents => view })
            .unwrap_or_else(|e| format!("template error: {e}")),
    )
    .into_response()
}
