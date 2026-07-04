use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::domain::api_key::ApiKey;

use crate::{error::AppError, middleware::auth::Principal, state::AppState};

async fn default_org_id(state: &Arc<AppState>) -> Result<Ulid, AppError> {
    let orgs = state.meta.list_orgs().await?;
    orgs.first()
        .map(|o| o.id)
        .ok_or(AppError::NotFound("no organization"))
}

// ---- JSON API ----
//
// Key management (mint/revoke) is dashboard-only: API keys are read/ingest
// scoped and must not be usable to mint or revoke other keys.

/// Public view of an `ApiKey`. Omits `key_hash` — the plaintext is shown
/// once at creation time and never again, but the hash itself has no
/// business leaving the store either.
fn api_key_json(k: &ApiKey) -> serde_json::Value {
    serde_json::json!({
        "id": k.id.to_string(),
        "org_id": k.org_id.to_string(),
        "site_id": k.site_id.map(|s| s.to_string()),
        "name": k.name,
        "scope": k.scope.as_str(),
        "display_prefix": k.display_prefix,
        "created_at": k.created_at.to_rfc3339(),
        "last_used_at": k.last_used_at.map(|t| t.to_rfc3339()),
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

/// Resolve an optional `site_id` body field to a validated binding: `None`
/// (org-wide), or `Some(id)` once confirmed to belong to `org_id`.
async fn resolve_site_binding(
    state: &AppState,
    org_id: Ulid,
    site_id: Option<&str>,
) -> Result<Option<Ulid>, Response> {
    let Some(s) = site_id else {
        return Ok(None);
    };
    let id = Ulid::from_string(s).map_err(|_| bad_request("invalid site_id"))?;
    let site = state
        .meta
        .get_site(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())?
        .ok_or_else(not_found)?;
    if site.org_id != org_id {
        return Err(bad_request("site not in org"));
    }
    Ok(Some(id))
}

#[derive(Deserialize)]
pub struct CreateKeyBody {
    pub name: String,
    /// "read" or "ingest".
    pub scope: String,
    /// Omit/null for org-wide (read keys only); a site ULID to bind to.
    #[serde(default)]
    pub site_id: Option<String>,
}

/// GET /api/v1/keys — every key in the org.
pub async fn list_keys_api(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
) -> Response {
    if !principal.is_dashboard() {
        return forbidden();
    }
    let org_id = match default_org_id(&state).await {
        Ok(id) => id,
        Err(_) => return Json(serde_json::json!({ "keys": [] })).into_response(),
    };
    let keys = state.meta.list_api_keys(org_id).await.unwrap_or_default();
    Json(serde_json::json!({
        "keys": keys.iter().map(api_key_json).collect::<Vec<_>>(),
    }))
    .into_response()
}

/// POST /api/v1/keys — mint a key; caller picks the scope and site binding.
pub async fn create_key_api(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<CreateKeyBody>,
) -> Response {
    if !principal.is_dashboard() {
        return forbidden();
    }
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return bad_request("key name is required");
    }
    let org_id = match default_org_id(&state).await {
        Ok(id) => id,
        Err(_) => return bad_request("no organization found"),
    };
    let site_binding = match resolve_site_binding(&state, org_id, body.site_id.as_deref()).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let (key, plaintext) = match body.scope.as_str() {
        "ingest" => {
            let Some(site_id) = site_binding else {
                return bad_request("ingest keys require a site_id");
            };
            ApiKey::new_ingest(org_id, site_id, name)
        }
        "read" => ApiKey::new_read(org_id, site_binding, name),
        _ => return bad_request("invalid scope"),
    };
    match state.meta.create_api_key(&key).await {
        Ok(_) => {
            let mut json = api_key_json(&key);
            // Shown exactly once — the caller must persist it now.
            json["secret"] = serde_json::Value::String(plaintext);
            (StatusCode::CREATED, Json(json)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// DELETE /api/v1/keys/:key_id — revoke a key, evicting it from the cache.
pub async fn delete_key_api(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(key_id): Path<String>,
) -> Response {
    if !principal.is_dashboard() {
        return forbidden();
    }
    let Ok(key_ulid) = Ulid::from_string(&key_id) else {
        return bad_request("invalid key id");
    };
    if let Err(e) = state.meta.delete_api_key(key_ulid).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    state.api_key_cache.retain(|_, v| v.key_id != key_ulid);
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
pub struct CreateSiteKeyBody {
    pub name: String,
    /// "read" or "ingest".
    pub scope: String,
    /// Read keys only: bind to the org instead of this single site.
    #[serde(default)]
    pub org_wide: bool,
}

/// GET /api/v1/sites/:site/keys — keys bound to this site plus org-wide
/// read keys, mirroring the per-site dashboard page.
pub async fn list_site_keys_api(
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
    let site_row = match state.meta.get_site(site_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let keys: Vec<_> = state
        .meta
        .list_api_keys(site_row.org_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|k| k.site_id == Some(site_id) || k.site_id.is_none())
        .map(|k| api_key_json(&k))
        .collect();
    Json(serde_json::json!({ "keys": keys })).into_response()
}

/// POST /api/v1/sites/:site/keys — mint a key bound to this site (or
/// org-wide, for read keys with `org_wide: true`).
pub async fn create_site_key_api(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    Json(body): Json<CreateSiteKeyBody>,
) -> Response {
    if !principal.is_dashboard() {
        return forbidden();
    }
    let Ok(site_id) = Ulid::from_string(&site) else {
        return bad_request("invalid site id");
    };
    let site_row = match state.meta.get_site(site_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return bad_request("key name is required");
    }
    let (key, plaintext) = match body.scope.as_str() {
        "ingest" => ApiKey::new_ingest(site_row.org_id, site_id, name),
        "read" => {
            let bound = if body.org_wide { None } else { Some(site_id) };
            ApiKey::new_read(site_row.org_id, bound, name)
        }
        _ => return bad_request("invalid scope"),
    };
    match state.meta.create_api_key(&key).await {
        Ok(_) => {
            let mut json = api_key_json(&key);
            json["secret"] = serde_json::Value::String(plaintext);
            (StatusCode::CREATED, Json(json)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// DELETE /api/v1/sites/:site/keys/:key_id — revoke, confirming the key
/// belongs to this site (or is an org-wide key visible from it).
pub async fn delete_site_key_api(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path((site, key_id)): Path<(String, String)>,
) -> Response {
    if !principal.is_dashboard() {
        return forbidden();
    }
    let (Ok(site_id), Ok(key_ulid)) = (Ulid::from_string(&site), Ulid::from_string(&key_id)) else {
        return bad_request("invalid id");
    };
    let site_row = match state.meta.get_site(site_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let keys = state
        .meta
        .list_api_keys(site_row.org_id)
        .await
        .unwrap_or_default();
    if !keys
        .iter()
        .any(|k| k.id == key_ulid && (k.site_id == Some(site_id) || k.site_id.is_none()))
    {
        return not_found();
    }
    if let Err(e) = state.meta.delete_api_key(key_ulid).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    state.api_key_cache.retain(|_, v| v.key_id != key_ulid);
    StatusCode::NO_CONTENT.into_response()
}
