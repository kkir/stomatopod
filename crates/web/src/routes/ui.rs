//! Serves the Dioxus SPA bundle produced by `mise run ui:bundle` (see
//! `crates/ui`). Embedded via `rust-embed`, which — by its own default
//! behavior — embeds the files at compile time in release builds and reads
//! them live from disk in debug builds, so `mise run ui:dev`-style
//! iteration doesn't require a rebuild of `stomatopod-web` per asset change.
//!
//! Mounted as the fallback of the `/app` nest in `router.rs`, behind the same
//! `require_auth` middleware as the rest of the dashboard (unauthenticated
//! requests are redirected to `/login` before any wasm loads). Any `/app/*`
//! path not matched by a concrete route (e.g. the agents dashboard) is handed
//! to this fallback, which returns the requested embedded asset or the SPA
//! shell for client-side routes like `/app/sites/abc123`.

use axum::{
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::{EmbeddedFile, RustEmbed};

#[derive(RustEmbed)]
#[folder = "ui-dist/"]
struct UiAssets;

const BUNDLE_MISSING: &str = "UI bundle not built; run mise run ui:bundle";

/// Fallback for the `/app` nest: serves an embedded asset, or the SPA shell
/// for any path the client-side router will handle. Because the router nests
/// this under `/app`, `uri.path()` is already relative to `/app` (e.g. a
/// request for `/app/assets/foo.js` arrives here as `/assets/foo.js`).
pub async fn serve_ui(uri: Uri) -> Response {
    serve_path(uri.path())
}

fn serve_path(requested: &str) -> Response {
    let requested = requested.trim_start_matches('/');
    if let Some(file) = UiAssets::get(requested) {
        return asset_response(requested, file);
    }
    // SPA routing fallback: any path not matching a real asset is a
    // client-side route, so hand back the shell and let the router in the
    // bundle take over.
    match UiAssets::get("index.html") {
        Some(file) => asset_response("index.html", file),
        None => (StatusCode::SERVICE_UNAVAILABLE, BUNDLE_MISSING).into_response(),
    }
}

fn asset_response(served_path: &str, file: EmbeddedFile) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type_for(served_path)),
            (header::CACHE_CONTROL, cache_control_for(served_path)),
        ],
        file.data.into_owned(),
    )
        .into_response()
}

/// Infer `Content-Type` from the file extension. Deliberately hand-rolled
/// (no mime-guess dependency) since the bundle only ever produces a small,
/// known set of file types.
fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Hashed assets under `assets/` are immutable (dx names them with a content
/// hash); everything else — the shell, including the SPA fallback — must
/// revalidate so a new deploy is picked up promptly.
fn cache_control_for(served_path: &str) -> &'static str {
    if served_path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}
