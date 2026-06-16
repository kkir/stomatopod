use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};

use stomatopod_core::domain::api_key::{ApiKey, ApiKeyScope};
use stomatopod_ingest::handler::{
    handle_ingest_inner, handle_server_ingest, IngestContext, IngestPayload, ServerEventPayload,
};
use tracing::warn;
use ulid::Ulid;

use crate::state::{ApiKeyCacheEntry, AppState};

/// Ingest endpoint — delegates to the ingest crate using AppState fields.
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
    };
    handle_ingest_inner(&ctx, peer_addr, &headers, payload).await
}

/// `POST /api/v1/ingest` — server-side custom event ingest.
///
/// Authentication: `Authorization: Bearer <ingest_key>` against the
/// `api_keys` table (scope = ingest). Distinct from `/api/v1/event`, which
/// uses a public site key in the body for browser beacons.
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
/// runs collapsed to single dashes.
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

/// `GET /app/docs` — the same documentation rendered to HTML for humans, with
/// a right-side anchor nav built from the H2/H3 headings.
pub async fn docs_page(
    State(state): State<Arc<AppState>>,
) -> Result<axum::response::Response, crate::error::AppError> {
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
            toc.push(TocItem { level: lvl, text, slug });
        }
        i = j + 1;
    }

    let mut body = String::new();
    html::push_html(&mut body, events.into_iter());

    let html = crate::templates::render(
        &state,
        "docs.jinja",
        minijinja::context! { content => body, toc => toc },
    )?;
    Ok(html.into_response())
}

static DASHBOARD_CSS: &str = include_str!("../../../../assets/dashboard.css");

/// Shared dashboard stylesheet. Short max-age (vs the tracker's immutable
/// day) so a binary upgrade doesn't leave browsers on stale styles for long.
pub async fn dashboard_css() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=600"),
        ],
        DASHBOARD_CSS,
    )
}
