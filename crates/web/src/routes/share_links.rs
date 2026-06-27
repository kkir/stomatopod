//! Public dashboard share links.
//!
//! Two surfaces live here:
//! * Authenticated CRUD under `/api/v1/sites/:site/share-links` for owners
//!   to mint, list, edit, and revoke links.
//! * Unauthenticated, token-scoped read access under `/share/:token` — an
//!   HTML shell plus a JSON API mirror confined to one site's aggregate
//!   reports (never raw events, sessions, keys, or settings).

use std::sync::Arc;

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use ulid::Ulid;

use stomatopod_core::{
    domain::share_link::ShareLink,
    query::{
        analytics::GoalQuery,
        events::EventQuery,
        pageviews::{Granularity, PageviewsQuery, TimeRange, TopListField},
    },
};

use crate::{middleware::auth::Principal, state::AppState};

// ---- Shared helpers (mirrors analytics.rs site resolution) ----

async fn resolve_site_id(state: &AppState, site: &str) -> Option<Ulid> {
    if let Ok(id) = Ulid::from_string(site) {
        return Some(id);
    }
    state
        .meta
        .get_site_by_domain(site)
        .await
        .ok()
        .flatten()
        .map(|s| s.id)
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
        Json(serde_json::json!({"error": "not authorized for this site"})),
    )
        .into_response()
}

/// Resolve `{site}` and enforce that `principal` may manage it. Mirrors the
/// authorization used by the analytics CRUD endpoints.
async fn resolve_authorized_site(
    state: &AppState,
    principal: &Principal,
    site: &str,
) -> Result<Ulid, Response> {
    let site_id = resolve_site_id(state, site).await.ok_or_else(not_found)?;
    if let Principal::ApiKey {
        org_id,
        site_id: key_site,
    } = principal
    {
        if let Some(ks) = key_site {
            if *ks != site_id {
                return Err(forbidden());
            }
        }
        let site_org = state
            .meta
            .get_site(site_id)
            .await
            .ok()
            .flatten()
            .map(|s| s.org_id);
        if site_org != Some(*org_id) {
            return Err(forbidden());
        }
    }
    Ok(site_id)
}

/// Generate a 32-byte URL-safe random token (hex-encoded, 64 chars).
fn generate_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf).expect("OS RNG");
    hex::encode(buf)
}

// ---- Authenticated CRUD ----

#[derive(Deserialize)]
pub struct CreateShareLinkBody {
    pub label: Option<String>,
    /// RFC3339 timestamp; omitted/null means no expiry.
    pub expires_at: Option<String>,
}

#[allow(clippy::result_large_err)]
fn parse_expiry(raw: Option<&str>) -> Result<Option<chrono::DateTime<chrono::Utc>>, Response> {
    match raw {
        None | Some("") => Ok(None),
        Some(s) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| Some(d.with_timezone(&chrono::Utc)))
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "invalid expires_at (want RFC3339)"})),
                )
                    .into_response()
            }),
    }
}

fn share_link_json(link: &ShareLink) -> serde_json::Value {
    serde_json::json!({
        "id": link.id.to_string(),
        "site_id": link.site_id.to_string(),
        "token": link.token,
        "label": link.label,
        "expires_at": link.expires_at.map(|d| d.to_rfc3339()),
        "created_at": link.created_at.to_rfc3339(),
    })
}

/// POST /api/v1/sites/:site/share-links
pub async fn create_share_link(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
    Json(body): Json<CreateShareLinkBody>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let expires_at = match parse_expiry(body.expires_at.as_deref()) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let created_by = match &principal {
        Principal::User(uid) => uid.clone(),
        _ => "session".to_string(),
    };
    let link = ShareLink {
        id: Ulid::new(),
        site_id,
        token: generate_token(),
        label: body.label.filter(|s| !s.trim().is_empty()),
        expires_at,
        created_by,
        created_at: chrono::Utc::now(),
    };
    match state.meta.create_share_link(&link).await {
        Ok(_) => {
            let mut json = share_link_json(&link);
            // Convenience: the full public URL the operator can copy.
            json["url"] = serde_json::Value::String(format!(
                "{}/share/{}",
                state.config.public_base_url(),
                link.token
            ));
            (StatusCode::CREATED, Json(json)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/v1/sites/:site/share-links
pub async fn list_share_links(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(site): Path<String>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match state.meta.list_share_links(site_id).await {
        Ok(links) => {
            let base = state.config.public_base_url();
            let items: Vec<serde_json::Value> = links
                .iter()
                .map(|l| {
                    let mut j = share_link_json(l);
                    j["url"] = serde_json::Value::String(format!("{base}/share/{}", l.token));
                    j
                })
                .collect();
            Json(serde_json::json!({ "share_links": items })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub struct PatchShareLinkBody {
    /// Present-and-null clears the label; absent leaves it unchanged.
    #[serde(default, deserialize_with = "double_option")]
    pub label: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub expires_at: Option<Option<String>>,
}

/// Distinguish an absent JSON key from an explicit `null`.
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(de).map(Some)
}

/// PATCH /api/v1/sites/:site/share-links/:id  — update label/expiry
pub async fn patch_share_link(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path((site, id)): Path<(String, String)>,
    Json(body): Json<PatchShareLinkBody>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let link_id = match Ulid::from_string(&id) {
        Ok(v) => v,
        Err(_) => return not_found(),
    };
    let existing = match state.meta.get_share_link(link_id).await {
        Ok(Some(l)) if l.site_id == site_id => l,
        Ok(_) => return not_found(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    // Merge: a present key overrides, an absent key keeps the current value.
    let label = match body.label {
        Some(v) => v.filter(|s| !s.trim().is_empty()),
        None => existing.label.clone(),
    };
    let expires_at = match body.expires_at {
        Some(v) => match parse_expiry(v.as_deref()) {
            Ok(e) => e,
            Err(resp) => return resp,
        },
        None => existing.expires_at,
    };

    match state
        .meta
        .update_share_link(link_id, label, expires_at)
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// DELETE /api/v1/sites/:site/share-links/:id  — revoke
pub async fn delete_share_link(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path((site, id)): Path<(String, String)>,
) -> impl IntoResponse {
    let site_id = match resolve_authorized_site(&state, &principal, &site).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let link_id = match Ulid::from_string(&id) {
        Ok(v) => v,
        Err(_) => return not_found(),
    };
    match state.meta.get_share_link(link_id).await {
        Ok(Some(l)) if l.site_id == site_id => {}
        Ok(_) => return not_found(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
    match state.meta.delete_share_link(link_id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---- Public, token-scoped access (no auth) ----

#[derive(Deserialize, Default)]
pub struct PublicParams {
    pub range: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

impl PublicParams {
    fn range(&self) -> TimeRange {
        if let (Some(f), Some(t)) = (&self.from, &self.to) {
            if let Some(r) = TimeRange::parse_dates(f, t) {
                return r;
            }
        }
        TimeRange::from_label(self.range.as_deref().unwrap_or("30d"))
    }
}

/// Resolve a public token to a live, non-expired share link. Returns a
/// ready-to-send error response otherwise: 404 for unknown/revoked tokens
/// (don't leak existence), 410 for expired ones.
async fn resolve_token(state: &AppState, token: &str) -> Result<ShareLink, Response> {
    let link = state
        .meta
        .get_share_link_by_token(token)
        .await
        .ok()
        .flatten()
        .ok_or_else(not_found)?;
    if link.is_expired(chrono::Utc::now()) {
        return Err((
            StatusCode::GONE,
            Json(serde_json::json!({"error": "this share link has expired"})),
        )
            .into_response());
    }
    Ok(link)
}

/// GET /share/:token  — minimal read-only HTML shell. Loads data from the
/// token-scoped JSON mirror below. Kept as inline HTML so it needs no
/// template registration.
pub async fn public_page(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Response {
    let link = match resolve_token(&state, &token).await {
        Ok(l) => l,
        Err(resp) => {
            // Render the expiry message as HTML rather than JSON for the
            // browser-facing route.
            if resp.status() == StatusCode::GONE {
                return (
                    StatusCode::GONE,
                    Html(
                        "<!doctype html><meta charset=utf-8><title>Link expired</title>\
                         <body style=\"font-family:system-ui;text-align:center;padding:64px\">\
                         <h1>This share link has expired</h1>\
                         <p>Ask the site owner for a fresh link.</p>\
                         <p style=\"color:#999\">Powered by Stomatopod</p></body>"
                            .to_string(),
                    ),
                )
                    .into_response();
            }
            return (StatusCode::NOT_FOUND, Html("<!doctype html><meta charset=utf-8><title>Not found</title><body><h1>404</h1></body>".to_string())).into_response();
        }
    };
    let site = state.meta.get_site(link.site_id).await.ok().flatten();
    let domain = site.map(|s| s.domain).unwrap_or_else(|| "site".into());
    let body = format!(
        "<!doctype html><html><head><meta charset=utf-8>\
         <meta name=\"robots\" content=\"noindex\">\
         <title>{domain} — analytics</title></head>\
         <body style=\"font-family:system-ui,sans-serif;max-width:880px;margin:0 auto;padding:24px\">\
         <h1>{domain}</h1>\
         <p>Read-only analytics. Data loads from \
            <code>/share/{token}/api/*</code>.</p>\
         <p id=\"summary\">Loading…</p>\
         <script>fetch('/share/{token}/api/pageviews').then(r=>r.json()).then(d=>{{\
           document.getElementById('summary').textContent = \
             (d.total_pageviews||0)+' pageviews, '+(d.total_sessions||0)+' sessions';\
         }}).catch(()=>{{document.getElementById('summary').textContent='Unavailable';}});</script>\
         <hr style=\"margin-top:32px;border:none;border-top:1px solid #eee\">\
         <p style=\"color:#999;font-size:13px\">Powered by Stomatopod</p>\
         </body></html>",
        domain = domain,
        token = token,
    );
    Html(body).into_response()
}

/// GET /share/:token/api/pageviews
pub async fn public_pageviews(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
    Query(params): Query<PublicParams>,
) -> Response {
    let link = match resolve_token(&state, &token).await {
        Ok(l) => l,
        Err(resp) => return resp,
    };
    match state
        .backend
        .query_pageviews(&PageviewsQuery {
            site_id: link.site_id,
            range: params.range(),
            granularity: Granularity::Day,
            filters: vec![],
        })
        .await
    {
        Ok(r) => Json(serde_json::to_value(r).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /share/:token/api/top/:dimension  — pages/referrers/countries/...
pub async fn public_top(
    State(state): State<Arc<AppState>>,
    Path((token, dimension)): Path<(String, String)>,
    Query(params): Query<PublicParams>,
) -> Response {
    let link = match resolve_token(&state, &token).await {
        Ok(l) => l,
        Err(resp) => return resp,
    };
    let field = match dimension.as_str() {
        "pages" => TopListField::Page,
        "referrers" => TopListField::Referrer,
        "countries" => TopListField::Country,
        "browsers" => TopListField::Browser,
        "devices" => TopListField::Device,
        "os" => TopListField::Os,
        "regions" => TopListField::Region,
        _ => return not_found(),
    };
    match state
        .backend
        .query_top_list(link.site_id, field, &params.range(), 20, &[])
        .await
    {
        Ok(r) => Json(serde_json::to_value(r).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /share/:token/api/events  — custom events summary only.
pub async fn public_events(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
    Query(params): Query<PublicParams>,
) -> Response {
    let link = match resolve_token(&state, &token).await {
        Ok(l) => l,
        Err(resp) => return resp,
    };
    let range = params.range();
    match state
        .backend
        .query_custom_events(&EventQuery {
            site_id: link.site_id,
            range,
            event_name: None,
            filters: vec![],
            limit: 20,
        })
        .await
    {
        Ok(r) => Json(serde_json::to_value(r).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /share/:token/api/goals  — goal completions + conversion rate.
pub async fn public_goals(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
    Query(params): Query<PublicParams>,
) -> Response {
    let link = match resolve_token(&state, &token).await {
        Ok(l) => l,
        Err(resp) => return resp,
    };
    let range = params.range();
    let goals = state
        .meta
        .list_goals(link.site_id)
        .await
        .unwrap_or_default();
    let mut out = Vec::new();
    for goal in goals {
        if let Ok(stats) = state
            .backend
            .query_goal(&GoalQuery {
                site_id: link.site_id,
                event_name: goal.event_name.clone(),
                filters: vec![],
                range: range.clone(),
                granularity: Granularity::Day,
            })
            .await
        {
            out.push(serde_json::json!({
                "name": goal.name,
                "event_name": goal.event_name,
                "completions": stats.completions,
                "conversion_rate": stats.conversion_rate,
            }));
        }
    }
    Json(serde_json::json!({ "goals": out })).into_response()
}
