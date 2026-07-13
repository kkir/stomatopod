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

use stomatopod_core::domain::site::Site;

use crate::{middleware::auth::Principal, state::AppState};

fn default_tz() -> String {
    "UTC".into()
}

/// Build a new `Site` domain object, shared by the site-creation API.
fn build_site(org_id: Ulid, domain: String, name: String, timezone: String) -> Site {
    Site {
        id: Ulid::new(),
        org_id,
        domain,
        name,
        timezone,
        public_key: generate_api_key(),
        created_at: Utc::now(),
        is_active: true,
    }
}

// ---- JSON API ----

fn site_json(site: &Site) -> serde_json::Value {
    serde_json::json!({
        "id": site.id.to_string(),
        "org_id": site.org_id.to_string(),
        "domain": site.domain,
        "name": site.name,
        "timezone": site.timezone,
        "public_key": site.public_key,
        "created_at": site.created_at.to_rfc3339(),
        "is_active": site.is_active,
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
        Json(serde_json::json!({"error": "site not found"})),
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

#[derive(Deserialize)]
pub struct CreateSiteBody {
    pub domain: String,
    pub name: String,
    #[serde(default = "default_tz")]
    pub timezone: String,
}

/// POST /api/v1/sites — create a site. Dashboard principals only: minting a
/// new site isn't a "read" operation and API keys are read/ingest-scoped.
pub async fn create_site_api(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<CreateSiteBody>,
) -> Response {
    if !principal.is_dashboard() {
        return forbidden();
    }
    let domain = body.domain.trim().to_string();
    let name = body.name.trim().to_string();
    if domain.is_empty() || name.is_empty() {
        return bad_request("domain and name are required");
    }
    let orgs = state.meta.list_orgs().await.unwrap_or_default();
    let Some(org) = orgs.first() else {
        return bad_request("no organization found");
    };
    let site = build_site(org.id, domain, name, body.timezone);
    // create_site also inserts the three starter analytics_alerts rows.
    match state.meta.create_site(&site).await {
        Ok(_) => (StatusCode::CREATED, Json(site_json(&site))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct PatchSiteBody {
    pub domain: Option<String>,
    pub name: Option<String>,
    pub timezone: Option<String>,
    pub is_active: Option<bool>,
}

/// PATCH /api/v1/sites/:site — update settings fields. Dashboard principals
/// only, matching `create_site_api`.
pub async fn patch_site_api(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    Json(body): Json<PatchSiteBody>,
) -> Response {
    if !principal.is_dashboard() {
        return forbidden();
    }
    let Ok(site_id) = Ulid::from_string(&site) else {
        return bad_request("invalid site id");
    };
    let mut site_row = match state.meta.get_site(site_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if let Some(d) = body.domain {
        site_row.domain = d;
    }
    if let Some(n) = body.name {
        site_row.name = n;
    }
    if let Some(tz) = body.timezone {
        let tz = tz.trim().to_string();
        if tz.is_empty() {
            return bad_request("timezone must not be empty");
        }
        // Accept IANA-looking names (Area/Location) or UTC; avoid free-form junk.
        if tz != "UTC" && !tz.contains('/') {
            return bad_request("timezone must be an IANA name (e.g. America/New_York) or UTC");
        }
        site_row.timezone = tz;
    }
    if let Some(active) = body.is_active {
        site_row.is_active = active;
    }
    match state.meta.update_site(&site_row).await {
        Ok(_) => {
            // Evict any cached public key so deactivate / key rotation take
            // effect without waiting for a process restart.
            state.site_cache.retain(|_, id| *id != site_id);
            if !site_row.is_active {
                state.site_cache.retain(|k, _| k != &site_row.public_key);
            }
            Json(site_json(&site_row)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn generate_api_key() -> String {
    // 32 hex chars = 128 bits of OS CSPRNG entropy (public site key for beacons).
    let mut raw = [0u8; 16];
    getrandom::getrandom(&mut raw).expect("OS RNG");
    hex::encode(raw)
}
