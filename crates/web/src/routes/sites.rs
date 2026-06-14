use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    Form,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::domain::site::Site;

use crate::{
    error::AppError,
    extractors::{Range, SiteId},
    state::AppState,
    templates,
};

#[derive(Deserialize)]
pub struct CreateSiteForm {
    pub domain: String,
    pub name: String,
    #[serde(default = "default_tz")]
    pub timezone: String,
}

#[derive(Deserialize)]
pub struct UpdateSiteForm {
    pub domain: String,
    pub name: String,
    #[serde(default = "default_tz")]
    pub timezone: String,
    pub is_active: Option<String>,
}

fn default_tz() -> String {
    "UTC".into()
}

const TIMEZONES: &[&str] = &[
    "UTC",
    "America/Los_Angeles",
    "America/Denver",
    "America/Chicago",
    "America/New_York",
    "America/Toronto",
    "America/Sao_Paulo",
    "Europe/London",
    "Europe/Berlin",
    "Europe/Paris",
    "Europe/Moscow",
    "Africa/Johannesburg",
    "Asia/Dubai",
    "Asia/Kolkata",
    "Asia/Singapore",
    "Asia/Tokyo",
    "Australia/Sydney",
    "Pacific/Auckland",
];

pub async fn sites_list(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let orgs = state.meta.list_orgs().await.unwrap_or_default();
    let org_id = orgs.first().map(|o| o.id).unwrap_or_default();
    let sites = state.meta.list_sites(org_id).await.unwrap_or_default();

    let tmpl = state.templates.get_template("index.jinja").unwrap();
    axum::response::Html(
        tmpl.render(minijinja::context! {
            sites => serde_json::to_value(&sites).unwrap(),
        })
        .unwrap(),
    )
}

pub async fn create_site(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CreateSiteForm>,
) -> impl IntoResponse {
    let orgs = state.meta.list_orgs().await.unwrap_or_default();
    let org_id = match orgs.first() {
        Some(o) => o.id,
        None => return (StatusCode::BAD_REQUEST, "No organization found").into_response(),
    };

    let site = Site {
        id: Ulid::new(),
        org_id,
        domain: form.domain,
        name: form.name,
        timezone: form.timezone,
        public_key: generate_api_key(),
        created_at: Utc::now(),
        is_active: true,
    };

    match state.meta.create_site(&site).await {
        Ok(_) => Redirect::to(&format!("/app/sites/{}", site.id)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn site_settings(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Range { label, .. }: Range,
) -> Result<Response, AppError> {
    let site = state
        .meta
        .get_site(site_id)
        .await?
        .ok_or(AppError::NotFound("site not found"))?;

    let created_ago = relative_time(site.created_at);
    let html = templates::render(
        &state,
        "site_settings.jinja",
        minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            range => label,
            created_ago => created_ago,
            timezones => TIMEZONES,
        },
    )?;
    Ok(html.into_response())
}

pub async fn update_site(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Form(form): Form<UpdateSiteForm>,
) -> impl IntoResponse {
    let mut site = match state.meta.get_site(site_id).await {
        Ok(Some(site)) => site,
        Ok(None) => return (StatusCode::NOT_FOUND, "Site not found").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    site.domain = form.domain;
    site.name = form.name;
    site.timezone = form.timezone;
    site.is_active = form.is_active.is_some();

    match state.meta.update_site(&site).await {
        Ok(_) => Redirect::to(&format!("/app/sites/{site_id}/settings")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn relative_time(ts: DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(ts);
    if delta.num_seconds() < 60 {
        "just now".into()
    } else if delta.num_minutes() < 60 {
        let n = delta.num_minutes();
        format!("{n} minute{} ago", if n == 1 { "" } else { "s" })
    } else if delta.num_hours() < 24 {
        let n = delta.num_hours();
        format!("{n} hour{} ago", if n == 1 { "" } else { "s" })
    } else if delta.num_days() < 30 {
        let n = delta.num_days();
        format!("{n} day{} ago", if n == 1 { "" } else { "s" })
    } else if delta.num_days() < 365 {
        let n = delta.num_days() / 30;
        format!("{n} month{} ago", if n == 1 { "" } else { "s" })
    } else {
        let n = delta.num_days() / 365;
        format!("{n} year{} ago", if n == 1 { "" } else { "s" })
    }
}

fn generate_api_key() -> String {
    use blake3::Hasher;
    let mut h = Hasher::new();
    h.update(&Ulid::new().to_bytes());
    h.update(
        &std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_le_bytes(),
    );
    let hash = h.finalize();
    // 32 hex chars = 128 bits of entropy
    hex::encode(&hash.as_bytes()[..16])
}
