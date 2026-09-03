use std::sync::Arc;

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use tower_http::{compression::CompressionLayer, trace::TraceLayer};

use crate::{
    middleware::{auth::require_api_auth, cors::ingest_cors, security_headers::security_headers},
    openapi,
    routes::{analytics, api, api_keys, auth, digest, insights, sites},
    state::AppState,
};

/// Builds the non-SPA router: the REST API, ingest, and auth. The Dioxus
/// application itself (SSR + hydration + static assets) is merged on top of
/// this by the appliance server (`stomatopod-web`), which owns the catch-all
/// fallback.
///
/// There is intentionally no public user-registration or org-creation route:
/// this is a single-owner appliance bootstrapped at first start.
pub fn build_router(state: Arc<AppState>) -> Router {
    // Public ingest routes (CORS-enabled)
    let ingest_routes = Router::new()
        .route("/api/v1/event", post(api::handle_ingest))
        .route("/tracker.js", get(api::tracker_js))
        .layer(ingest_cors());

    // Public assets the login page needs before auth. `/app.css` is the same
    // compiled Tailwind bundle the Dioxus SPA uses. The dashboard root `/`
    // is served by the Dioxus SSR fallback (behind `require_auth`).
    // `/health` and `/ready` are unauthenticated probes for orchestrators.
    // `/robots.txt` and `/sitemap.xml` must be real public responses — if they
    // fall through to `require_auth` crawlers receive a login HTML bounce.
    let public_assets = Router::new()
        .route("/app.css", get(api::dashboard_css))
        .route("/llms.txt", get(api::llms_txt))
        .route("/robots.txt", get(api::robots_txt))
        .route("/sitemap.xml", get(api::sitemap_xml))
        .route("/openapi.json", get(openapi::openapi_json))
        .route("/health", get(api::health))
        .route("/ready", get(api::ready));

    // Server-side custom event ingest. Bearer-auth'd inline via an ingest
    // API key (handler resolves the site from the key). No CORS - calls
    // come from backends, not browsers.
    let key_ingest_routes = Router::new().route("/api/v1/ingest", post(api::handle_key_ingest));

    // Analytics JSON API routes (bearer token or session auth)
    let analytics_routes = Router::new()
        .route(
            "/api/v1/sites",
            get(analytics::list_sites).post(sites::create_site_api),
        )
        .route(
            "/api/v1/sites/{site}",
            axum::routing::patch(sites::patch_site_api),
        )
        .route("/api/v1/me", get(api::me))
        .route("/api/v1/me/password", post(api::change_password))
        .route("/api/v1/docs", get(api::docs_api))
        .route("/api/v1/sites/{site}/pageviews", get(analytics::pageviews))
        .route("/api/v1/sites/{site}/top-pages", get(analytics::top_pages))
        .route(
            "/api/v1/sites/{site}/top-referrers",
            get(analytics::top_referrers),
        )
        .route("/api/v1/sites/{site}/top-os", get(analytics::top_os))
        .route(
            "/api/v1/sites/{site}/top-regions",
            get(analytics::top_regions),
        )
        .route(
            "/api/v1/sites/{site}/top-countries",
            get(analytics::top_countries),
        )
        .route(
            "/api/v1/sites/{site}/top-browsers",
            get(analytics::top_browsers),
        )
        .route(
            "/api/v1/sites/{site}/top-devices",
            get(analytics::top_devices),
        )
        .route("/api/v1/sites/{site}/events", get(analytics::events))
        .route(
            "/api/v1/sites/{site}/top-entry-pages",
            get(analytics::top_entry_pages),
        )
        .route(
            "/api/v1/sites/{site}/top-exit-pages",
            get(analytics::top_exit_pages),
        )
        .route(
            "/api/v1/sites/{site}/export/events",
            get(analytics::export_events),
        )
        .route(
            "/api/v1/sites/{site}/export/sessions",
            get(analytics::export_sessions),
        )
        .route(
            "/api/v1/sites/{site}/analytics-alerts",
            get(analytics::list_analytics_alerts).post(analytics::create_analytics_alert),
        )
        .route(
            "/api/v1/sites/{site}/analytics-alerts/{id}",
            axum::routing::patch(analytics::patch_analytics_alert)
                .delete(analytics::delete_analytics_alert),
        )
        .route("/api/v1/sites/{site}/campaigns", get(analytics::campaigns))
        .route("/api/v1/sites/{site}/utm", get(analytics::utm))
        .route(
            "/api/v1/sites/{site}/funnels",
            get(analytics::list_funnels).post(analytics::create_funnel),
        )
        .route(
            "/api/v1/sites/{site}/funnels/{funnel_id}",
            get(analytics::funnel_result).delete(analytics::delete_funnel),
        )
        // ---- API keys (CRUD): global + per-site ----
        .route(
            "/api/v1/keys",
            get(api_keys::list_keys_api).post(api_keys::create_key_api),
        )
        .route(
            "/api/v1/keys/{key_id}",
            axum::routing::delete(api_keys::delete_key_api),
        )
        .route(
            "/api/v1/sites/{site}/keys",
            get(api_keys::list_site_keys_api).post(api_keys::create_site_key_api),
        )
        .route(
            "/api/v1/sites/{site}/keys/{key_id}",
            axum::routing::delete(api_keys::delete_site_key_api),
        )
        // ---- Alert channels (CRUD) + test-fire ----
        .route(
            "/api/v1/sites/{site}/alert-channels",
            get(insights::list_channels_api).post(insights::create_channel_api),
        )
        .route(
            "/api/v1/sites/{site}/alert-channels/{id}",
            axum::routing::delete(insights::delete_channel_api),
        )
        .route(
            "/api/v1/sites/{site}/alert-channels/{id}/test",
            post(insights::test_channel_api),
        )
        // ---- Analytics digest subscription ----
        .route(
            "/api/v1/sites/{site}/digest-subscription",
            get(digest::get_subscription)
                .put(digest::put_subscription)
                .delete(digest::delete_subscription),
        )
        .route(
            "/api/v1/sites/{site}/digest-subscription/test",
            post(digest::send_test),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_auth,
        ));

    // Auth routes (no auth required)
    let auth_routes = Router::new()
        .route("/login", get(auth::login_page).post(auth::login_submit))
        .route("/logout", post(auth::logout));

    let compressed = Router::new()
        .merge(ingest_routes)
        .merge(key_ingest_routes)
        .merge(analytics_routes)
        .merge(auth_routes)
        .merge(public_assets)
        .layer(CompressionLayer::new());

    Router::new()
        .merge(compressed)
        .layer(middleware::from_fn(security_headers))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
