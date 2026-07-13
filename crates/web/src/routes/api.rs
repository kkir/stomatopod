use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, Extension, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use axum_extra::extract::cookie::CookieJar;

use stomatopod_core::domain::api_key::{ApiKey, ApiKeyScope};
use stomatopod_ingest::handler::{
    handle_ingest_inner, handle_server_ingest, IngestContext, IngestPayload, ServerEventPayload,
};
use tracing::warn;
use ulid::Ulid;

use crate::{
    middleware::auth::{verify_session, Principal, SESSION_COOKIE},
    state::{ApiKeyCacheEntry, AppState},
};

/// Browser beacon body (mirrors `IngestPayload` field names for OpenAPI).
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct BrowserIngestBody {
    /// Site public key.
    pub k: String,
    /// Event name (`pageview` or custom).
    pub n: String,
    /// Full page URL.
    pub u: String,
    /// Referrer URL.
    pub r: Option<String>,
    /// Screen width.
    pub w: Option<u16>,
    /// Screen height.
    pub h: Option<u16>,
    /// Browser language tag.
    pub l: Option<String>,
    /// Custom event properties.
    pub p: Option<serde_json::Value>,
    /// Client timestamp (Unix ms).
    pub t: Option<i64>,
}

/// Server-side custom event body (mirrors `ServerEventPayload` for OpenAPI).
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct ServerIngestBody {
    pub name: String,
    pub url: Option<String>,
    pub properties: Option<serde_json::Value>,
    pub timestamp: Option<i64>,
    pub referrer: Option<String>,
    pub session_id: Option<String>,
}

/// Session / API-key probe response (shape depends on principal).
#[derive(serde::Serialize, utoipa::ToSchema)]
#[schema(example = json!({"auth": "api_key", "org_id": "01HXYZ", "site_id": null}))]
pub struct MeResponse {
    pub auth: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// Ingest endpoint — delegates to the ingest crate using AppState fields.
#[utoipa::path(
    post,
    path = "/api/v1/event",
    tag = "ingest",
    request_body = BrowserIngestBody,
    responses(
        (status = 204, description = "Accepted"),
        (status = 401, description = "Unknown site public key"),
        (status = 429, description = "Ingest back-pressure"),
        (status = 503, description = "Ingest channel closed")
    )
)]
pub async fn handle_ingest(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<IngestPayload>,
) -> StatusCode {
    let ctx = IngestContext {
        meta: state.meta.clone(),
        tx: state.ingest_tx.clone(),
        geo: state.geo.clone(),
        site_cache: state.site_cache.clone(),
        trust_forwarded_headers: state.config.auth.trust_forwarded_headers,
    };
    handle_ingest_inner(&ctx, peer_addr, &headers, payload).await
}

/// `POST /api/v1/ingest` — server-side custom event ingest.
///
/// Authentication: `Authorization: Bearer <ingest_key>` against the
/// `api_keys` table (scope = ingest). Distinct from `/api/v1/event`, which
/// uses a public site key in the body for browser beacons.
#[utoipa::path(
    post,
    path = "/api/v1/ingest",
    tag = "ingest",
    security(("ingest_key" = [])),
    request_body = ServerIngestBody,
    responses(
        (status = 204, description = "Accepted"),
        (status = 400, description = "Empty event name"),
        (status = 401, description = "Missing or invalid ingest key"),
        (status = 429, description = "Ingest back-pressure")
    )
)]
pub async fn handle_key_ingest(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ServerEventPayload>,
) -> StatusCode {
    let Some(bearer) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    else {
        return StatusCode::UNAUTHORIZED;
    };

    let key_hash = ApiKey::hash(bearer);
    let entry = match resolve_api_key(&state, &key_hash).await {
        Ok(Some(e)) => e,
        Ok(None) => return StatusCode::UNAUTHORIZED,
        Err(e) => {
            warn!("api key lookup error: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    // Ingest keys must be ingest-scoped and bound to a site.
    let site_id = match (entry.scope, entry.site_id) {
        (ApiKeyScope::Ingest, Some(site_id)) => site_id,
        _ => return StatusCode::UNAUTHORIZED,
    };

    touch_api_key(&state, entry.key_id);

    handle_server_ingest(&state.ingest_tx, site_id, payload).await
}

/// Resolve a key hash via the in-memory cache, falling back to the store
/// and populating the cache on a miss. Shared by the ingest and read paths.
pub async fn resolve_api_key(
    state: &Arc<AppState>,
    key_hash: &str,
) -> Result<Option<ApiKeyCacheEntry>, stomatopod_core::error::StoreError> {
    if let Some(e) = state.api_key_cache.get(key_hash) {
        return Ok(Some(*e));
    }
    match state.meta.get_api_key_by_hash(key_hash).await? {
        Some(key) => {
            let entry = ApiKeyCacheEntry {
                org_id: key.org_id,
                site_id: key.site_id,
                key_id: key.id,
                scope: key.scope,
            };
            state.api_key_cache.insert(key_hash.to_string(), entry);
            Ok(Some(entry))
        }
        None => Ok(None),
    }
}

/// Fire-and-forget `last_used_at` update, including on cache hits.
pub fn touch_api_key(state: &Arc<AppState>, key_id: Ulid) {
    let meta = state.meta.clone();
    tokio::spawn(async move {
        if let Err(e) = meta.touch_api_key(key_id).await {
            warn!("touch_api_key failed: {e}");
        }
    });
}

/// Resolve the signed-session user id carried by a `Principal`. Bearer
/// principals already carry it; the cookie-based `Session` principal only
/// records that a valid cookie was present (see `require_api_auth`), so this
/// re-derives the user id from the cookie itself.
fn session_user_id(state: &AppState, principal: &Principal, jar: &CookieJar) -> Option<Ulid> {
    let raw = match principal {
        Principal::User(uid) => Some(uid.clone()),
        Principal::Session => jar
            .get(SESSION_COOKIE)
            .and_then(|c| verify_session(&state.config.auth.secret_key, c.value())),
        Principal::ApiKey { .. } => None,
    }?;
    Ulid::from_string(&raw).ok()
}

/// `GET /api/v1/me` — session probe so the SPA can confirm it is logged in
/// and learn who the current user is. API-key principals get a reduced view
/// (org/site scope, no user row) since keys aren't tied to a specific user.
#[utoipa::path(
    get,
    path = "/api/v1/me",
    tag = "meta",
    security(("read_key" = [])),
    responses(
        (status = 200, description = "Current principal", body = MeResponse),
        (status = 401, description = "Unauthenticated")
    )
)]
pub async fn me(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    jar: CookieJar,
) -> Response {
    if let Principal::ApiKey { org_id, site_id } = &principal {
        return Json(serde_json::json!({
            "auth": "api_key",
            "org_id": org_id.to_string(),
            "site_id": site_id.map(|s| s.to_string()),
        }))
        .into_response();
    }

    let Some(user_id) = session_user_id(&state, &principal, &jar) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "authentication required"})),
        )
            .into_response();
    };

    match state.meta.get_user(user_id).await {
        Ok(Some(user)) => Json(serde_json::json!({
            "auth": "session",
            "id": user.id.to_string(),
            "org_id": user.org_id.to_string(),
            "email": user.email,
            "role": serde_json::to_value(user.role).unwrap_or_default(),
        }))
        .into_response(),
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "authentication required"})),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

static TRACKER: &str = include_str!("../../../../assets/tracker.js");

pub async fn tracker_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        TRACKER,
    )
}

static DOCS_MD: &str = include_str!("../../../../assets/docs.md");

/// `GET /llms.txt` — canonical product/API documentation as Markdown, for LLM
/// agents and other machine consumers. Public (no secrets), following the
/// llms.txt convention.
pub async fn llms_txt() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/markdown; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=600"),
        ],
        DOCS_MD,
    )
}

/// One entry in the docs table of contents (right-side anchor nav).
#[derive(serde::Serialize)]
struct TocItem {
    level: u8,
    text: String,
    slug: String,
}

/// URL-safe anchor slug from heading text: lowercase alphanumerics, other
/// runs collapsed to single dashes. Keep in lockstep with UI deep-link
/// constants in `ui/docs_anchors.rs`.
fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = true; // suppress leading dash
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// Rendered docs: the markdown body as HTML plus the H2/H3 anchor-nav TOC.
/// Shared by the human-facing page and the JSON API so the pulldown-cmark
/// walk (heading-slug assignment included) lives in exactly one place.
struct RenderedDocs {
    body: String,
    toc: Vec<TocItem>,
}

/// Render `DOCS_MD` to HTML, slugging H1-H6 headings as anchor ids and
/// collecting H2/H3 headings into a table of contents.
fn render_docs() -> RenderedDocs {
    use pulldown_cmark::{html, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
    use std::collections::HashMap;

    let mut events: Vec<Event> = Parser::new_ext(DOCS_MD, Options::all()).collect();
    let mut toc: Vec<TocItem> = Vec::new();
    let mut seen: HashMap<String, u32> = HashMap::new();

    let mut i = 0;
    while i < events.len() {
        let Event::Start(Tag::Heading { level, .. }) = events[i] else {
            i += 1;
            continue;
        };

        // Accumulate the heading's text up to its closing tag.
        let mut text = String::new();
        let mut j = i + 1;
        while j < events.len() {
            match &events[j] {
                Event::Text(t) | Event::Code(t) => text.push_str(t),
                Event::End(TagEnd::Heading(_)) => break,
                _ => {}
            }
            j += 1;
        }

        // Unique slug, then stamp it onto the heading so the anchor resolves.
        let base = slugify(&text);
        let n = seen.entry(base.clone()).or_insert(0);
        let slug = if *n == 0 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        *n += 1;
        if let Event::Start(Tag::Heading { id, .. }) = &mut events[i] {
            *id = Some(CowStr::from(slug.clone()));
        }

        let lvl = match level {
            HeadingLevel::H1 => 1,
            HeadingLevel::H2 => 2,
            HeadingLevel::H3 => 3,
            HeadingLevel::H4 => 4,
            HeadingLevel::H5 => 5,
            HeadingLevel::H6 => 6,
        };
        if lvl == 2 || lvl == 3 {
            toc.push(TocItem {
                level: lvl,
                text,
                slug,
            });
        }
        i = j + 1;
    }

    let mut body = String::new();
    html::push_html(&mut body, events.into_iter());
    RenderedDocs { body, toc }
}

/// `GET /api/v1/docs` — the same documentation as pulldown-cmark-rendered
/// HTML, for the SPA to inject with `dangerous_inner_html`.
pub async fn docs_api() -> impl IntoResponse {
    let RenderedDocs { body, toc } = render_docs();
    Json(serde_json::json!({ "html": body, "toc": toc }))
}

/// Shared app stylesheet (compiled Tailwind used by both the login page and
/// the Dioxus SPA). Resolves the hashed file under `DIOXUS_PUBLIC_PATH` /
/// sibling release layout. Short max-age so a rebuild is picked up quickly.
pub async fn dashboard_css() -> impl IntoResponse {
    let body = resolve_shared_css().unwrap_or_default();
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=600"),
        ],
        body,
    )
}

/// Locate the newest `tailwind*.css` under the Dioxus public assets directory.
fn resolve_shared_css() -> Option<String> {
    let candidates = shared_css_search_dirs();
    let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for dir in candidates {
        let assets = dir.join("assets");
        let Ok(entries) = std::fs::read_dir(&assets) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !(name.starts_with("tailwind") && name.ends_with(".css")) {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            match &best {
                Some((t, _)) if *t >= modified => {}
                _ => best = Some((modified, path)),
            }
        }
    }
    best.and_then(|(_, path)| std::fs::read_to_string(path).ok())
}

fn shared_css_search_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(p) = std::env::var("DIOXUS_PUBLIC_PATH") {
        dirs.push(std::path::PathBuf::from(p));
    }
    // `dx build --release` places assets next to the server binary.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("public"));
            dirs.push(parent.to_path_buf());
        }
    }
    // Dev fallback when running from the workspace root without env set.
    dirs.push(std::path::PathBuf::from(
        "target/dx/stomatopod/debug/web/public",
    ));
    dirs.push(std::path::PathBuf::from(
        "target/dx/stomatopod/release/web/public",
    ));
    dirs
}

#[cfg(test)]
mod docs_slug_tests {
    use super::{render_docs, slugify};
    use crate::ui::docs_anchors::{EMITTING_CUSTOM_EVENTS, INSTALLING_THE_BROWSER_TRACKER};

    #[test]
    fn known_heading_slugs() {
        let cases = [
            (
                "Installing the browser tracker",
                INSTALLING_THE_BROWSER_TRACKER,
            ),
            ("Emitting custom events", EMITTING_CUSTOM_EVENTS),
            (
                "Manual events from the browser",
                "manual-events-from-the-browser",
            ),
            ("Creating funnels", "creating-funnels"),
            ("Authentication", "authentication"),
            ("Querying analytics", "querying-analytics"),
            ("Concepts", "concepts"),
        ];
        for (heading, expected) in cases {
            assert_eq!(slugify(heading), expected, "heading {heading:?}");
        }
    }

    #[test]
    fn rendered_docs_stamp_ids_matching_toc_and_ui_deep_links() {
        let rendered = render_docs();
        assert!(
            !rendered.toc.is_empty(),
            "docs TOC should list H2/H3 headings"
        );
        for item in &rendered.toc {
            let needle = format!("id=\"{}\"", item.slug);
            assert!(
                rendered.body.contains(&needle),
                "heading id missing in HTML for slug {}",
                item.slug
            );
        }
        // UI deep links must resolve to real anchors in the rendered HTML.
        for slug in [INSTALLING_THE_BROWSER_TRACKER, EMITTING_CUSTOM_EVENTS] {
            assert!(
                rendered.body.contains(&format!("id=\"{slug}\"")),
                "UI deep-link slug {slug} missing from docs HTML"
            );
            assert!(
                rendered.toc.iter().any(|t| t.slug == slug),
                "UI deep-link slug {slug} missing from docs TOC"
            );
        }
    }
}
