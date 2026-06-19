use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    Form,
};
use serde::Deserialize;
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::domain::api_key::ApiKey;

use crate::{
    error::AppError,
    extractors::{Range, SiteId},
    state::AppState,
    templates,
};

#[derive(Deserialize)]
pub struct CreateKeyForm {
    pub name: String,
    /// "read" or "ingest".
    pub scope: String,
    /// Present (checkbox) → read key spans the whole org rather than this site.
    pub org_wide: Option<String>,
}

/// GET /app/sites/:site_id/keys — list keys for this site (+ org-wide read keys).
pub async fn keys_page(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Range { label, .. }: Range,
) -> Result<Response, AppError> {
    render_keys_page(&state, site_id, label, None).await
}

/// POST /app/sites/:site_id/keys — mint a key, then render the page with the
/// plaintext shown exactly once (no redirect, so the secret never hits a URL).
pub async fn create_key(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Range { label, .. }: Range,
    Form(form): Form<CreateKeyForm>,
) -> Result<Response, AppError> {
    let site = state
        .meta
        .get_site(site_id)
        .await?
        .ok_or(AppError::NotFound("site not found"))?;

    let name = form.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("key name is required"));
    }

    let (key, plaintext) = match form.scope.as_str() {
        "ingest" => ApiKey::new_ingest(site.org_id, site_id, name),
        "read" => {
            let bound = if form.org_wide.is_some() {
                None
            } else {
                Some(site_id)
            };
            ApiKey::new_read(site.org_id, bound, name)
        }
        _ => return Err(AppError::BadRequest("invalid scope")),
    };

    state.meta.create_api_key(&key).await?;
    render_keys_page(&state, site_id, label, Some(plaintext)).await
}

/// POST /app/sites/:site_id/keys/:key_id/delete — revoke a key and evict it
/// from the cache so the revocation takes effect immediately.
pub async fn delete_key(
    State(state): State<Arc<AppState>>,
    Path((site_id, key_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let Ok(key_ulid) = Ulid::from_string(&key_id) else {
        return (StatusCode::BAD_REQUEST, "invalid key id").into_response();
    };

    if let Err(e) = state.meta.delete_api_key(key_ulid).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    // Drop any cached entry for this key so it stops authenticating at once.
    state.api_key_cache.retain(|_, v| v.key_id != key_ulid);

    Redirect::to(&format!("/app/sites/{site_id}/keys")).into_response()
}

// ---- Global (org-wide) keys page ----

#[derive(Deserialize)]
pub struct KeysFilter {
    /// "all" (default), "org" (org-wide only), or a site ULID.
    #[serde(default)]
    pub filter: Option<String>,
}

#[derive(Deserialize)]
pub struct GlobalCreateKeyForm {
    pub name: String,
    /// "read" or "ingest".
    pub scope: String,
    /// "org" / empty for org-wide, or a site ULID to bind to.
    #[serde(default)]
    pub site: Option<String>,
}

/// GET /app/keys — every key in the org, filterable by site or org-level.
pub async fn global_keys_page(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(filter): axum::extract::Query<KeysFilter>,
) -> Result<Response, AppError> {
    render_global_keys_page(&state, filter.filter.as_deref(), None).await
}

/// POST /app/keys — mint a key from the global page (caller picks the site).
pub async fn global_create_key(
    State(state): State<Arc<AppState>>,
    Form(form): Form<GlobalCreateKeyForm>,
) -> Result<Response, AppError> {
    let org_id = default_org_id(&state).await?;

    let name = form.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("key name is required"));
    }

    // Resolve the target site binding from the form selection.
    let site_binding = match form.site.as_deref() {
        Some("org") | Some("") | None => None,
        Some(s) => {
            let id = Ulid::from_string(s).map_err(|_| AppError::BadRequest("invalid site"))?;
            let site = state
                .meta
                .get_site(id)
                .await?
                .ok_or(AppError::NotFound("site not found"))?;
            if site.org_id != org_id {
                return Err(AppError::BadRequest("site not in org"));
            }
            Some(id)
        }
    };

    let (key, plaintext) = match form.scope.as_str() {
        "ingest" => {
            let site_id = site_binding.ok_or(AppError::BadRequest("ingest keys require a site"))?;
            ApiKey::new_ingest(org_id, site_id, name)
        }
        "read" => ApiKey::new_read(org_id, site_binding, name),
        _ => return Err(AppError::BadRequest("invalid scope")),
    };

    state.meta.create_api_key(&key).await?;
    render_global_keys_page(&state, None, Some(plaintext)).await
}

/// POST /app/keys/:key_id/delete — revoke from the global page.
pub async fn global_delete_key(
    State(state): State<Arc<AppState>>,
    Path(key_id): Path<String>,
) -> impl IntoResponse {
    let Ok(key_ulid) = Ulid::from_string(&key_id) else {
        return (StatusCode::BAD_REQUEST, "invalid key id").into_response();
    };
    if let Err(e) = state.meta.delete_api_key(key_ulid).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    state.api_key_cache.retain(|_, v| v.key_id != key_ulid);
    Redirect::to("/app/keys").into_response()
}

async fn default_org_id(state: &Arc<AppState>) -> Result<Ulid, AppError> {
    let orgs = state.meta.list_orgs().await?;
    orgs.first()
        .map(|o| o.id)
        .ok_or(AppError::NotFound("no organization"))
}

async fn render_global_keys_page(
    state: &Arc<AppState>,
    filter: Option<&str>,
    new_key: Option<String>,
) -> Result<Response, AppError> {
    let org_id = default_org_id(state).await?;
    let sites = state.meta.list_sites(org_id).await?;
    // site_id → name, to label each key's binding in the table.
    let site_name = |id: Ulid| -> String {
        sites
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "Unknown site".into())
    };

    let active_filter = filter.unwrap_or("all");
    let keys: Vec<_> = state
        .meta
        .list_api_keys(org_id)
        .await?
        .into_iter()
        .filter(|k| match active_filter {
            "org" => k.site_id.is_none(),
            "all" => true,
            sid => Ulid::from_string(sid).ok() == k.site_id,
        })
        .map(|k| {
            let label = match k.site_id {
                Some(id) => site_name(id),
                None => "Org-wide".into(),
            };
            let mut v = serde_json::to_value(&k).unwrap();
            v["site_label"] = serde_json::Value::String(label);
            v
        })
        .collect();

    let site_options: Vec<_> = sites
        .iter()
        .map(|s| serde_json::json!({ "id": s.id.to_string(), "name": s.name }))
        .collect();

    let html = templates::render(
        state,
        "keys.jinja",
        minijinja::context! {
            keys => keys,
            sites => site_options,
            filter => active_filter,
            new_key => new_key,
        },
    )?;
    Ok(html.into_response())
}

async fn render_keys_page(
    state: &Arc<AppState>,
    site_id: Ulid,
    range: &str,
    new_key: Option<String>,
) -> Result<Response, AppError> {
    let site = state
        .meta
        .get_site(site_id)
        .await?
        .ok_or(AppError::NotFound("site not found"))?;

    // Org-level list, narrowed to this site plus org-wide read keys.
    let keys: Vec<_> = state
        .meta
        .list_api_keys(site.org_id)
        .await?
        .into_iter()
        .filter(|k| k.site_id == Some(site_id) || k.site_id.is_none())
        .map(|k| serde_json::to_value(&k).unwrap())
        .collect();

    let html = templates::render(
        state,
        "api_keys.jinja",
        minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            range => range,
            keys => keys,
            new_key => new_key,
        },
    )?;
    Ok(html.into_response())
}
