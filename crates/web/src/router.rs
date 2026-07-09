use std::sync::Arc;

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use tower_http::{compression::CompressionLayer, trace::TraceLayer};

use crate::{
    middleware::{auth::require_api_auth, cors::ingest_cors},
    routes::{analytics, api, api_keys, auth, digest, insights, share_links, sites},
    state::AppState,
};

/// Builds the non-SPA router: the REST API, ingest, auth, and public share
/// pages. The Dioxus application itself (SSR + hydration + static assets) is
/// merged on top of this by [`crate::server::serve`], which owns the catch-all
/// fallback.
pub fn build_router(state: Arc<AppState>) -> Router {
    // Public ingest routes (CORS-enabled)
    let ingest_routes = Router::new()
        .route("/api/v1/event", post(api::handle_ingest))
        .route("/tracker.js", get(api::tracker_js))
        .layer(ingest_cors());

    // Public assets the login page needs before auth. The dashboard root `/`
    // is no longer a redirect: it is now served by the Dioxus SSR fallback
    // (behind `require_auth`, which redirects to `/login` when unauthenticated).
    let public_assets = Router::new()
        .route("/app.css", get(api::dashboard_css))
        .route("/llms.txt", get(api::llms_txt));

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
            "/api/v1/sites/{site}/goals",
            get(analytics::list_goals).post(analytics::create_goal),
        )
        .route(
            "/api/v1/sites/{site}/goals/{goal_id}",
            axum::routing::delete(analytics::delete_goal),
        )
        .route(
            "/api/v1/sites/{site}/goals/{goal_id}/stats",
            get(analytics::goal_stats),
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
        .route("/api/v1/sites/{site}/paths", get(analytics::paths))
        .route(
            "/api/v1/sites/{site}/annotations",
            get(analytics::list_annotations).post(analytics::create_annotation),
        )
        .route(
            "/api/v1/sites/{site}/annotations/{id}",
            axum::routing::delete(analytics::delete_annotation),
        )
        .route(
            "/api/v1/sites/{site}/funnels",
            get(analytics::list_funnels).post(analytics::create_funnel),
        )
        .route(
            "/api/v1/sites/{site}/funnels/{funnel_id}",
            get(analytics::funnel_result),
        )
        // ---- Share links (CRUD) ----
        .route(
            "/api/v1/sites/{site}/share-links",
            get(share_links::list_share_links).post(share_links::create_share_link),
        )
        .route(
            "/api/v1/sites/{site}/share-links/{id}",
            axum::routing::patch(share_links::patch_share_link)
                .delete(share_links::delete_share_link),
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
        // ---- Email digest subscription ----
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

    // Public, unauthenticated surfaces: token-scoped share dashboards and
    // one-click digest unsubscribe. No auth middleware.
    let public_share_routes = Router::new()
        .route("/share/{token}", get(share_links::public_page))
        .route(
            "/share/{token}/api/pageviews",
            get(share_links::public_pageviews),
        )
        .route(
            "/share/{token}/api/top/{dimension}",
            get(share_links::public_top),
        )
        .route("/share/{token}/api/events", get(share_links::public_events))
        .route("/share/{token}/api/goals", get(share_links::public_goals))
        .route("/digest/unsubscribe/{token}", get(digest::unsubscribe));

    // Auth routes (no auth required)
    let auth_routes = Router::new()
        .route("/login", get(auth::login_page).post(auth::login_submit))
        .route("/logout", post(auth::logout));

    let compressed = Router::new()
        .merge(ingest_routes)
        .merge(key_ingest_routes)
        .merge(analytics_routes)
        .merge(public_share_routes)
        .merge(auth_routes)
        .merge(public_assets)
        .layer(CompressionLayer::new());

    Router::new()
        .merge(compressed)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
