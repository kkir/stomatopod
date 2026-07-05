use std::fmt::Write as _;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use stomatopod_core::{domain::incident::IncidentTrigger, query::spans::SpanQuery};
use ulid::Ulid;

use crate::{error::AppError, server::html, state::AppState};

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
        return Ok(Html(html::dashboard_page(
            "Agents",
            r#"<div class="card"><p class="muted">No sites configured yet.</p></div>"#,
        ))
        .into_response());
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

    let mut body = String::from(
        r#"<div class="page-head"><h1 class="page-title">Sentinel Agents</h1></div><div class="card">"#,
    );
    if summaries.is_empty() {
        body.push_str(
            r#"<p class="muted">No agent activity in the last 24 hours. Run a sidecar pointed at this server to start streaming spans.</p>"#,
        );
    } else {
        body.push_str(
            "<table><thead><tr><th>Agent</th><th>Last seen</th><th>Spans</th><th>Input tokens</th><th>Output tokens</th><th>Cost (USD)</th></tr></thead><tbody>",
        );
        for a in &summaries {
            let id = html::escape(&a.agent_id);
            let _ = write!(
                body,
                r#"<tr><td><a href="/agents/{id}">{id}</a></td><td>{last}</td><td>{spans}</td><td>{in_tok}</td><td>{out_tok}</td><td>${cost:.4}</td></tr>"#,
                last = a.last_seen_at,
                spans = a.total_spans,
                in_tok = a.total_input_tokens,
                out_tok = a.total_output_tokens,
                cost = a.total_cost_usd,
            );
        }
        body.push_str("</tbody></table>");
    }
    body.push_str("</div>");
    Ok(Html(html::dashboard_page("Agents", &body)).into_response())
}

pub async fn agent_detail(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> Result<Response, AppError> {
    let Some(site_id) = first_site_id(&state).await else {
        return Err(AppError::NotFound("no site configured"));
    };
    let since = Utc::now() - Duration::hours(24);
    // Same empty-state handling as `agents_index` - treat a missing
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

    let aid = html::escape(&agent_id);
    let sid = site_id.to_string();
    // ids are surfaced to the client via data-* attributes so the inline
    // script never interpolates user-supplied strings into a JSON literal.
    let body = format!(
        r#"<div class="page-head" data-site-id="{sid}" data-agent-id="{aid}" id="agent-header">
  <div>
    <h1 class="page-title">Agent <code>{aid}</code></h1>
    <p class="page-sub">Site <code>{sid}</code></p>
  </div>
  <div class="btn-row">
    <button id="kill-btn" class="btn btn-danger">Kill agent</button>
    <button id="hint-btn" class="btn btn-ghost">Send hint…</button>
    <span id="kill-status" class="muted"></span>
  </div>
</div>
<div class="stat-grid mb-3">
  <div class="card stat"><div class="label">Spans (24h)</div><div class="value">{spans}</div></div>
  <div class="card stat"><div class="label">Input tokens (24h)</div><div class="value">{in_tok}</div></div>
  <div class="card stat"><div class="label">Output tokens (24h)</div><div class="value">{out_tok}</div></div>
  <div class="card stat"><div class="label">Cost (24h)</div><div class="value">${cost:.4}</div></div>
</div>
<div class="card" hx-get="/agents/{aid}/spans?site_id={sid}" hx-trigger="load, every 2s" hx-swap="innerHTML">
  <p class="muted">Loading spans…</p>
</div>
<dialog id="hint-modal">
  <h2>Send hint</h2>
  <form id="hint-form">
    <textarea name="message" rows="4" required></textarea>
    <div class="dialog-actions">
      <button type="button" id="hint-cancel" class="btn btn-ghost">Cancel</button>
      <button type="submit" class="btn btn-primary">Send</button>
    </div>
  </form>
</dialog>
<script>
  (function () {{
    const hdr = document.getElementById("agent-header");
    const siteId = hdr.dataset.siteId;
    const agentId = hdr.dataset.agentId;
    const modal = document.getElementById("hint-modal");
    async function postControl(body) {{
      await fetch("/api/v1/sentinel/control", {{
        method: "POST",
        headers: {{ "content-type": "application/json" }},
        body: JSON.stringify(body),
        credentials: "same-origin",
      }});
    }}
    document.getElementById("kill-btn").addEventListener("click", async () => {{
      await postControl({{ site_id: siteId, agent_id: agentId, command: "kill", reason: "manual kill from dashboard" }});
      document.getElementById("kill-status").textContent = "Kill sent";
    }});
    document.getElementById("hint-btn").addEventListener("click", () => modal.showModal());
    document.getElementById("hint-cancel").addEventListener("click", () => modal.close());
    document.getElementById("hint-form").addEventListener("submit", async (e) => {{
      e.preventDefault();
      await postControl({{ site_id: siteId, agent_id: agentId, command: "hint", message: e.target.message.value }});
      modal.close();
    }});
  }})();
</script>"#,
    );
    Ok(Html(html::dashboard_page(&format!("Agent {agent_id}"), &body)).into_response())
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

    if rows.is_empty() {
        return Ok(Html(r#"<p class="muted">No spans yet.</p>"#.to_string()).into_response());
    }

    let mut out = String::from(
        "<table><thead><tr><th>Started</th><th>Kind</th><th>Model</th><th>Tool</th><th>Stop</th><th>In</th><th>Out</th><th>Cost</th></tr></thead><tbody>",
    );
    for r in &rows {
        let _ = write!(
            out,
            r#"<tr><td>{started}</td><td>{kind}</td><td><code>{model}</code></td><td>{tool}</td><td>{stop}</td><td>{in_tok}</td><td>{out_tok}</td><td>${cost:.5}</td></tr>"#,
            started = r.started_at,
            kind = html::escape(&r.kind),
            model = html::escape(&r.model),
            tool = html::escape(r.tool_name.as_deref().unwrap_or("")),
            stop = html::escape(r.stop_reason.as_deref().unwrap_or("")),
            in_tok = r.input_tokens,
            out_tok = r.output_tokens,
            cost = r.cost_usd,
        );
    }
    out.push_str("</tbody></table>");
    Ok(Html(out).into_response())
}

/// JSON-friendly view of an incident row for rendering.
struct IncidentView {
    opened_at: String,
    agent_id: String,
    trigger_summary: String,
    status: &'static str,
}

pub async fn incidents_page(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    let Some(site_id) = first_site_id(&state).await else {
        return Ok(Html(html::dashboard_page(
            "Incidents",
            r#"<div class="card"><p class="muted">No sites configured.</p></div>"#,
        ))
        .into_response());
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

    let mut body = String::from(
        r#"<div class="page-head"><h1 class="page-title">Incidents</h1></div><div class="card">"#,
    );
    if view.is_empty() {
        body.push_str(r#"<p class="muted">No incidents recorded.</p>"#);
    } else {
        body.push_str(
            "<table><thead><tr><th>Opened</th><th>Agent</th><th>Trigger</th><th>Status</th></tr></thead><tbody>",
        );
        for inc in &view {
            let _ = write!(
                body,
                "<tr><td>{opened}</td><td>{agent}</td><td>{trigger}</td><td>{status}</td></tr>",
                opened = html::escape(&inc.opened_at),
                agent = html::escape(&inc.agent_id),
                trigger = html::escape(&inc.trigger_summary),
                status = inc.status,
            );
        }
        body.push_str("</tbody></table>");
    }
    body.push_str("</div>");
    Ok(Html(html::dashboard_page("Incidents", &body)).into_response())
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
        IncidentTrigger::AnalyticsAlert {
            alert_type, value, ..
        } => format!("{alert_type} ({value:.1})"),
        IncidentTrigger::Manual => "manual".into(),
    }
}
