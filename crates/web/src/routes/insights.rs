//! Dashboard pages for the Tier-2 analytics features: the real-time view,
//! goals/conversions, and analytics alerts. All are session-authed (mounted
//! under the `require_auth` dashboard router) and server-rendered.

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    Form,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::{
    domain::{
        agent::{AlertChannel, AlertChannelKind},
        analytics_alert::{AnalyticsAlert, AnalyticsAlertConfig, AnalyticsAlertKind},
        goal::Goal,
        incident::{Incident, IncidentStatus, IncidentTrigger},
    },
    query::{analytics::GoalQuery, pageviews::Granularity},
};

use crate::{
    alerts::sinks::{AlertSink, SlackSink, TelegramSink, WebhookSink},
    error::AppError,
    extractors::{Range, SiteId},
    state::AppState,
    templates,
};

async fn load_site(
    state: &AppState,
    site_id: Ulid,
) -> Result<stomatopod_core::domain::site::Site, AppError> {
    state
        .meta
        .get_site(site_id)
        .await?
        .ok_or(AppError::NotFound("site not found"))
}

/// `?site=` selector on the global (site-less) pages. `tested` carries the
/// test-channel result flash back to the alerts page.
#[derive(Deserialize)]
pub struct SiteSel {
    site: Option<String>,
    #[serde(default)]
    tested: Option<String>,
}

/// `?tested=` flash on the per-site alerts page.
#[derive(Deserialize)]
pub struct AlertsFlash {
    #[serde(default)]
    tested: Option<String>,
}

/// Resolve the global-page scope: the full site list (for the dropdown) and
/// the selected site (the `?site=` param if valid, else the first site).
/// Returns `None` when the org has no sites yet.
async fn resolve_scope(
    state: &AppState,
    sel: Option<&str>,
) -> Result<Option<(stomatopod_core::domain::site::Site, Vec<serde_json::Value>)>, AppError> {
    let orgs = state.meta.list_orgs().await?;
    let sites = match orgs.first() {
        Some(org) => state.meta.list_sites(org.id).await?,
        None => vec![],
    };
    if sites.is_empty() {
        return Ok(None);
    }
    let wanted = sel.and_then(|s| Ulid::from_string(s).ok());
    let selected = wanted
        .and_then(|id| sites.iter().find(|s| s.id == id).cloned())
        .unwrap_or_else(|| sites[0].clone());
    let options: Vec<serde_json::Value> = sites
        .iter()
        .map(|s| serde_json::json!({ "id": s.id.to_string(), "name": s.name }))
        .collect();
    Ok(Some((selected, options)))
}

/// Per-goal completion + conversion view for a site over `range`.
async fn build_goals_view(
    state: &AppState,
    site_id: Ulid,
    range: &stomatopod_core::query::pageviews::TimeRange,
) -> Result<Vec<serde_json::Value>, AppError> {
    let goals = state.meta.list_goals(site_id).await?;
    let mut out = Vec::with_capacity(goals.len());
    for g in &goals {
        let stats = state
            .backend
            .query_goal(&GoalQuery {
                site_id,
                event_name: g.event_name.clone(),
                filters: g.parsed_filters(),
                granularity: Granularity::auto_for_range(range),
                range: range.clone(),
            })
            .await
            .unwrap_or_default();
        out.push(serde_json::json!({
            "id": g.id.to_string(),
            "name": g.name,
            "event_name": g.event_name,
            "completions": stats.completions,
            "unique_completions": stats.unique_completions,
            "conversion_rate": format!("{:.1}", stats.conversion_rate),
        }));
    }
    Ok(out)
}

/// Alert + channel view models for a site.
async fn build_alerts_view(
    state: &AppState,
    site_id: Ulid,
) -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>), AppError> {
    let alerts = state.meta.list_analytics_alerts(site_id).await?;
    let channels = state.meta.list_alert_channels(site_id).await?;
    let mut alerts_view = Vec::with_capacity(alerts.len());
    for a in &alerts {
        let last = state
            .meta
            .last_analytics_alert_fire(a.id)
            .await
            .ok()
            .flatten()
            .map(|f| f.fired_at.to_rfc3339());
        alerts_view.push(serde_json::json!({
            "id": a.id.to_string(),
            "type": a.kind.as_str(),
            "threshold": a.config.threshold,
            "window_minutes": a.config.window_minutes,
            "goal_event_name": a.config.goal_event_name,
            "enabled": a.enabled,
            "last_fired": last,
        }));
    }
    let channels_view: Vec<serde_json::Value> = channels
        .iter()
        .map(|c| serde_json::json!({ "id": c.id.to_string(), "url": c.url, "kind": c.kind.as_str() }))
        .collect();
    Ok((alerts_view, channels_view))
}

// ---- Global (site-less) pages with a site-filter dropdown ----

/// GET /app/realtime — real-time view with a site selector.
pub async fn realtime_global(
    State(state): State<Arc<AppState>>,
    Query(sel): Query<SiteSel>,
) -> Result<Response, AppError> {
    let Some((site, sites)) = resolve_scope(&state, sel.site.as_deref()).await? else {
        return Ok(Redirect::to("/app/sites").into_response());
    };
    let html = templates::render(
        &state,
        "realtime.jinja",
        minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            range => "30d",
            sites => sites,
            scope_path => "/app/realtime",
        },
    )?;
    Ok(html.into_response())
}

/// GET /app/goals — goals across sites, filtered by a site selector.
pub async fn goals_global(
    State(state): State<Arc<AppState>>,
    Query(sel): Query<SiteSel>,
    Range { range, label }: Range,
) -> Result<Response, AppError> {
    let Some((site, sites)) = resolve_scope(&state, sel.site.as_deref()).await? else {
        return Ok(Redirect::to("/app/sites").into_response());
    };
    let goals = build_goals_view(&state, site.id, &range).await?;
    let html = templates::render(
        &state,
        "goals.jinja",
        minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            goals => goals,
            range => label,
            sites => sites,
            scope_path => "/app/goals",
        },
    )?;
    Ok(html.into_response())
}

/// GET /app/alerts — analytics alerts across sites, filtered by a selector.
pub async fn alerts_global(
    State(state): State<Arc<AppState>>,
    Query(sel): Query<SiteSel>,
    Range { label, .. }: Range,
) -> Result<Response, AppError> {
    let Some((site, sites)) = resolve_scope(&state, sel.site.as_deref()).await? else {
        return Ok(Redirect::to("/app/sites").into_response());
    };
    let (alerts, channels) = build_alerts_view(&state, site.id).await?;
    let html = templates::render(
        &state,
        "alerts.jinja",
        minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            alerts => alerts,
            channels => channels,
            range => label,
            sites => sites,
            scope_path => "/app/alerts",
            tested => sel.tested,
        },
    )?;
    Ok(html.into_response())
}

// ---- Real-time ----

/// GET /app/sites/:site_id/realtime — page shell that polls the panel.
pub async fn realtime_page(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Range { label, .. }: Range,
) -> Result<Response, AppError> {
    let site = load_site(&state, site_id).await?;
    let html = templates::render(
        &state,
        "realtime.jinja",
        minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            range => label,
        },
    )?;
    Ok(html.into_response())
}

/// GET /app/sites/:site_id/partials/realtime — htmx-polled inner panel.
pub async fn realtime_panel(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
) -> Result<Response, AppError> {
    let snapshot = state.backend.query_realtime(site_id, 30).await?;
    let html = templates::render(
        &state,
        "partials/realtime_panel.jinja",
        minijinja::context! {
            rt => serde_json::to_value(&snapshot).unwrap(),
        },
    )?;
    Ok(html.into_response())
}

// ---- Goals ----

/// GET /app/sites/:site_id/goals — goal list with 30d conversion stats.
pub async fn goals_page(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Range { range, label }: Range,
) -> Result<Response, AppError> {
    let site = load_site(&state, site_id).await?;
    let goals = build_goals_view(&state, site_id, &range).await?;
    let html = templates::render(
        &state,
        "goals.jinja",
        minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            goals => goals,
            range => label,
        },
    )?;
    Ok(html.into_response())
}

#[derive(Deserialize)]
pub struct CreateGoalForm {
    pub name: String,
    pub event_name: String,
}

/// POST /app/sites/:site_id/goals — create a goal, then back to the list.
pub async fn create_goal(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Form(form): Form<CreateGoalForm>,
) -> Result<Response, AppError> {
    if form.name.trim().is_empty() || form.event_name.trim().is_empty() {
        return Err(AppError::BadRequest("name and event are required"));
    }
    let goal = Goal {
        id: Ulid::new(),
        site_id,
        name: form.name,
        event_name: form.event_name,
        filters: None,
        created_at: Utc::now(),
    };
    state.meta.create_goal(&goal).await?;
    Ok(Redirect::to(&format!("/app/sites/{site_id}/goals")).into_response())
}

/// POST /app/sites/:site_id/goals/:goal_id/delete
pub async fn delete_goal(
    State(state): State<Arc<AppState>>,
    Path((site_id_str, goal_id_str)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let site_id =
        Ulid::from_string(&site_id_str).map_err(|_| AppError::BadRequest("invalid site id"))?;
    let goal_id =
        Ulid::from_string(&goal_id_str).map_err(|_| AppError::BadRequest("invalid goal id"))?;
    // Only delete a goal that belongs to this site.
    if let Some(g) = state.meta.get_goal(goal_id).await? {
        if g.site_id == site_id {
            state.meta.delete_goal(goal_id).await?;
        }
    }
    Ok(Redirect::to(&format!("/app/sites/{site_id}/goals")).into_response())
}

// ---- Analytics alerts ----

/// GET /app/sites/:site_id/alerts — alert list + create form.
pub async fn alerts_page(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Range { label, .. }: Range,
    Query(flash): Query<AlertsFlash>,
) -> Result<Response, AppError> {
    let site = load_site(&state, site_id).await?;
    let (alerts, channels) = build_alerts_view(&state, site_id).await?;
    let html = templates::render(
        &state,
        "alerts.jinja",
        minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            alerts => alerts,
            channels => channels,
            range => label,
            tested => flash.tested,
        },
    )?;
    Ok(html.into_response())
}

#[derive(Deserialize)]
pub struct CreateAlertForm {
    #[serde(rename = "type")]
    pub alert_type: String,
    pub threshold: f64,
    #[serde(default)]
    pub window_minutes: u32,
    #[serde(default)]
    pub goal_event_name: Option<String>,
    pub channel_id: String,
}

/// POST /app/sites/:site_id/alerts — create an analytics alert.
pub async fn create_alert(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Form(form): Form<CreateAlertForm>,
) -> Result<Response, AppError> {
    let kind = AnalyticsAlertKind::from_str(&form.alert_type)
        .ok_or(AppError::BadRequest("unknown alert type"))?;
    let channel_id =
        Ulid::from_string(&form.channel_id).map_err(|_| AppError::BadRequest("invalid channel"))?;
    // The channel must belong to this site.
    let channels = state.meta.list_alert_channels(site_id).await?;
    if !channels.iter().any(|c| c.id == channel_id) {
        return Err(AppError::BadRequest("channel does not belong to this site"));
    }
    let goal_event_name = form.goal_event_name.filter(|s| !s.trim().is_empty());
    let alert = AnalyticsAlert {
        id: Ulid::new(),
        site_id,
        kind,
        config: AnalyticsAlertConfig {
            threshold: form.threshold,
            window_minutes: if form.window_minutes == 0 {
                60
            } else {
                form.window_minutes
            },
            goal_event_name,
        },
        channel_id,
        enabled: true,
        created_at: Utc::now(),
    };
    state.meta.create_analytics_alert(&alert).await?;
    Ok(Redirect::to(&format!("/app/sites/{site_id}/alerts")).into_response())
}

/// POST /app/sites/:site_id/alerts/:id/delete
pub async fn delete_alert(
    State(state): State<Arc<AppState>>,
    Path((site_id_str, alert_id_str)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let site_id =
        Ulid::from_string(&site_id_str).map_err(|_| AppError::BadRequest("invalid site id"))?;
    let alert_id =
        Ulid::from_string(&alert_id_str).map_err(|_| AppError::BadRequest("invalid alert id"))?;
    if let Some(a) = state.meta.get_analytics_alert(alert_id).await? {
        if a.site_id == site_id {
            state.meta.delete_analytics_alert(alert_id).await?;
        }
    }
    Ok(Redirect::to(&format!("/app/sites/{site_id}/alerts")).into_response())
}

// ---- Alert channels ----

#[derive(Deserialize)]
pub struct CreateChannelForm {
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub secret: Option<String>,
}

/// POST /app/sites/:site_id/channels — create a webhook/Slack/Telegram channel.
pub async fn create_channel(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Form(form): Form<CreateChannelForm>,
) -> Result<Response, AppError> {
    let kind = AlertChannelKind::from_str(&form.kind);
    let url = form.url.trim().to_string();
    if url.is_empty() {
        // For Telegram this field is the chat id; for webhooks/Slack the URL.
        return Err(AppError::BadRequest("destination is required"));
    }
    let secret = form
        .secret
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    // Telegram needs the bot token in `secret`.
    if kind == AlertChannelKind::Telegram && secret.is_none() {
        return Err(AppError::BadRequest("telegram channel needs a bot token"));
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
    state.meta.create_alert_channel(&channel).await?;
    Ok(Redirect::to(&format!("/app/sites/{site_id}/alerts")).into_response())
}

/// POST /app/sites/:site_id/channels/:channel_id/delete
pub async fn delete_channel(
    State(state): State<Arc<AppState>>,
    Path((site_id_str, channel_id_str)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let site_id =
        Ulid::from_string(&site_id_str).map_err(|_| AppError::BadRequest("invalid site id"))?;
    let channel_id = Ulid::from_string(&channel_id_str)
        .map_err(|_| AppError::BadRequest("invalid channel id"))?;
    // Only delete a channel that belongs to this site.
    let channels = state.meta.list_alert_channels(site_id).await?;
    if channels.iter().any(|c| c.id == channel_id) {
        state.meta.delete_alert_channel(channel_id).await?;
    }
    Ok(Redirect::to(&format!("/app/sites/{site_id}/alerts")).into_response())
}

/// POST /app/sites/:site_id/channels/:channel_id/test — send a sample
/// notification through the channel and report the result via `?tested=`.
pub async fn test_channel(
    State(state): State<Arc<AppState>>,
    Path((site_id_str, channel_id_str)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let site_id =
        Ulid::from_string(&site_id_str).map_err(|_| AppError::BadRequest("invalid site id"))?;
    let channel_id = Ulid::from_string(&channel_id_str)
        .map_err(|_| AppError::BadRequest("invalid channel id"))?;
    let channels = state.meta.list_alert_channels(site_id).await?;
    let Some(channel) = channels.into_iter().find(|c| c.id == channel_id) else {
        return Ok(
            Redirect::to(&format!("/app/sites/{site_id}/alerts?tested=fail")).into_response(),
        );
    };

    // A sample incident, dispatched once (no retry) through the channel's sink.
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
    let outcome = if sink.dispatch(&channel, &incident).await.is_ok() {
        "ok"
    } else {
        "fail"
    };
    Ok(Redirect::to(&format!("/app/sites/{site_id}/alerts?tested={outcome}")).into_response())
}
