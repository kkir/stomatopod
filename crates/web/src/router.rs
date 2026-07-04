use std::sync::Arc;

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use tower_http::{compression::CompressionLayer, trace::TraceLayer};

use crate::{
    middleware::{
        auth::{require_api_auth, require_auth},
        cors::ingest_cors,
    },
    routes::{
        agents_dashboard, analytics, api, api_keys, auth, digest, insights, sentinel, share_links,
        sites, spans, ui,
    },
    state::AppState,
};

pub fn build_router(state: Arc<AppState>) -> Router {
    // Public ingest routes (CORS-enabled)
    let ingest_routes = Router::new()
        .route("/api/v1/event", post(api::handle_ingest))
        .route("/tracker.js", get(api::tracker_js))
        .layer(ingest_cors());

    // Redirect / to /login — marketing site is out of scope for self-hosted.
    // The stylesheet is public: the login page needs it before auth.
    let root_redirect = Router::new()
        .route(
            "/",
            get(|| async { axum::response::Redirect::to("/login") }),
        )
        .route("/app.css", get(api::dashboard_css))
        .route("/llms.txt", get(api::llms_txt));

    // Sentinel span ingest. Bearer-auth'd via sentinel_tokens (handler
    // checks the header itself; no middleware needed). No CORS since
    // calls come from sidecars, not browsers.
    let span_ingest_routes =
        Router::new().route("/api/v1/spans", post(spans::handle_span_ingest_route));

    // Server-side custom event ingest. Bearer-auth'd inline via an ingest
    // API key (handler resolves the site from the key). No CORS — calls
    // come from backends, not browsers.
    let key_ingest_routes = Router::new().route("/api/v1/ingest", post(api::handle_key_ingest));

    // Analytics JSON API routes (bearer token or session auth)
    let analytics_routes = Router::new()
        .route(
            "/api/v1/sites",
            get(analytics::list_sites).post(sites::create_site_api),
        )
        .route(
            "/api/v1/sites/:site",
            axum::routing::patch(sites::patch_site_api),
        )
        .route("/api/v1/me", get(api::me))
        .route("/api/v1/docs", get(api::docs_api))
        .route("/api/v1/sites/:site/pageviews", get(analytics::pageviews))
        .route("/api/v1/sites/:site/top-pages", get(analytics::top_pages))
        .route(
            "/api/v1/sites/:site/top-referrers",
            get(analytics::top_referrers),
        )
        .route("/api/v1/sites/:site/top-os", get(analytics::top_os))
        .route(
            "/api/v1/sites/:site/top-regions",
            get(analytics::top_regions),
        )
        .route(
            "/api/v1/sites/:site/top-countries",
            get(analytics::top_countries),
        )
        .route(
            "/api/v1/sites/:site/top-browsers",
            get(analytics::top_browsers),
        )
        .route(
            "/api/v1/sites/:site/top-devices",
            get(analytics::top_devices),
        )
        .route("/api/v1/sites/:site/events", get(analytics::events))
        .route(
            "/api/v1/sites/:site/top-entry-pages",
            get(analytics::top_entry_pages),
        )
        .route(
            "/api/v1/sites/:site/top-exit-pages",
            get(analytics::top_exit_pages),
        )
        .route("/api/v1/sites/:site/realtime", get(analytics::realtime))
        .route(
            "/api/v1/sites/:site/export/events",
            get(analytics::export_events),
        )
        .route(
            "/api/v1/sites/:site/export/sessions",
            get(analytics::export_sessions),
        )
        .route(
            "/api/v1/sites/:site/goals",
            get(analytics::list_goals).post(analytics::create_goal),
        )
        .route(
            "/api/v1/sites/:site/goals/:goal_id",
            axum::routing::delete(analytics::delete_goal),
        )
        .route(
            "/api/v1/sites/:site/goals/:goal_id/stats",
            get(analytics::goal_stats),
        )
        .route(
            "/api/v1/sites/:site/analytics-alerts",
            get(analytics::list_analytics_alerts).post(analytics::create_analytics_alert),
        )
        .route(
            "/api/v1/sites/:site/analytics-alerts/:id",
            axum::routing::patch(analytics::patch_analytics_alert)
                .delete(analytics::delete_analytics_alert),
        )
        .route("/api/v1/sites/:site/campaigns", get(analytics::campaigns))
        .route("/api/v1/sites/:site/retention", get(analytics::retention))
        .route("/api/v1/sites/:site/paths", get(analytics::paths))
        .route(
            "/api/v1/sites/:site/annotations",
            get(analytics::list_annotations).post(analytics::create_annotation),
        )
        .route(
            "/api/v1/sites/:site/annotations/:id",
            axum::routing::delete(analytics::delete_annotation),
        )
        .route(
            "/api/v1/sites/:site/funnels",
            get(analytics::list_funnels).post(analytics::create_funnel),
        )
        .route(
            "/api/v1/sites/:site/funnels/:funnel_id",
            get(analytics::funnel_result),
        )
        // ---- Tier-4 analytics ----
        .route("/api/v1/sites/:site/vitals", get(analytics::vitals))
        .route(
            "/api/v1/sites/:site/vitals/pages",
            get(analytics::vitals_pages),
        )
        .route("/api/v1/sites/:site/scroll", get(analytics::scroll))
        .route(
            "/api/v1/sites/:site/scroll/pages",
            get(analytics::scroll_pages),
        )
        .route("/api/v1/sites/:site/search", get(analytics::search))
        .route(
            "/api/v1/sites/:site/search/zero-results",
            get(analytics::search_zero_results),
        )
        .route(
            "/api/v1/sites/:site/search/timeseries",
            get(analytics::search_timeseries),
        )
        .route("/api/v1/sites/:site/revenue", get(analytics::revenue))
        .route(
            "/api/v1/sites/:site/revenue/timeseries",
            get(analytics::revenue_timeseries),
        )
        .route(
            "/api/v1/sites/:site/revenue/pages",
            get(analytics::revenue_pages),
        )
        .route(
            "/api/v1/sites/:site/revenue/breakdown",
            get(analytics::revenue_breakdown),
        )
        .route(
            "/api/v1/sites/:site/experiments",
            get(analytics::experiments),
        )
        .route(
            "/api/v1/sites/:site/experiments/:experiment",
            get(analytics::experiment_result),
        )
        .route(
            "/api/v1/sites/:site/heatmaps/clicks",
            get(analytics::heatmap_clicks),
        )
        .route(
            "/api/v1/sites/:site/heatmaps/scroll",
            get(analytics::heatmap_scroll),
        )
        // ---- Share links (CRUD) ----
        .route(
            "/api/v1/sites/:site/share-links",
            get(share_links::list_share_links).post(share_links::create_share_link),
        )
        .route(
            "/api/v1/sites/:site/share-links/:id",
            axum::routing::patch(share_links::patch_share_link)
                .delete(share_links::delete_share_link),
        )
        // ---- API keys (CRUD): global + per-site ----
        .route(
            "/api/v1/keys",
            get(api_keys::list_keys_api).post(api_keys::create_key_api),
        )
        .route(
            "/api/v1/keys/:key_id",
            axum::routing::delete(api_keys::delete_key_api),
        )
        .route(
            "/api/v1/sites/:site/keys",
            get(api_keys::list_site_keys_api).post(api_keys::create_site_key_api),
        )
        .route(
            "/api/v1/sites/:site/keys/:key_id",
            axum::routing::delete(api_keys::delete_site_key_api),
        )
        // ---- Alert channels (CRUD) + test-fire ----
        .route(
            "/api/v1/sites/:site/alert-channels",
            get(insights::list_channels_api).post(insights::create_channel_api),
        )
        .route(
            "/api/v1/sites/:site/alert-channels/:id",
            axum::routing::delete(insights::delete_channel_api),
        )
        .route(
            "/api/v1/sites/:site/alert-channels/:id/test",
            post(insights::test_channel_api),
        )
        // ---- Email digest subscription ----
        .route(
            "/api/v1/sites/:site/digest-subscription",
            get(digest::get_subscription)
                .put(digest::put_subscription)
                .delete(digest::delete_subscription),
        )
        .route(
            "/api/v1/sites/:site/digest-subscription/test",
            post(digest::send_test),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_auth,
        ));

    // Public, unauthenticated surfaces: token-scoped share dashboards and
    // one-click digest unsubscribe. No auth middleware.
    let public_share_routes = Router::new()
        .route("/share/:token", get(share_links::public_page))
        .route(
            "/share/:token/api/pageviews",
            get(share_links::public_pageviews),
        )
        .route(
            "/share/:token/api/top/:dimension",
            get(share_links::public_top),
        )
        .route("/share/:token/api/events", get(share_links::public_events))
        .route("/share/:token/api/goals", get(share_links::public_goals))
        .route("/digest/unsubscribe/:token", get(digest::unsubscribe));

    // Auth routes (no auth required)
    let auth_routes = Router::new()
        .route("/login", get(auth::login_page).post(auth::login_submit))
        .route("/logout", post(auth::logout));

    // Protected dashboard: the Dioxus SPA now owns `/app` and every
    // client-side route beneath it. The legacy AI-firewall "agents"
    // dashboard is not part of the SPA migration (its nav is commented out),
    // so its concrete routes are kept alongside the SPA fallback. matchit
    // 0.7 forbids overlapping a `/app/*path` catch-all with these concrete
    // routes, so the SPA is served via a nested `fallback` instead.
    let app_routes = Router::new()
        .route("/agents", get(agents_dashboard::agents_index))
        .route("/agents/:agent_id", get(agents_dashboard::agent_detail))
        .route(
            "/agents/:agent_id/spans",
            get(agents_dashboard::agent_spans_partial),
        )
        .route("/incidents", get(agents_dashboard::incidents_page))
        // Anything else under /app (the shell, hashed assets, and every SPA
        // client route) is served by the bundle built by `mise run ui:bundle`.
        .fallback(ui::serve_ui);

    let dashboard_routes = Router::new()
        .nest("/app", app_routes)
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    // Sentinel SSE control stream — bearer-auth'd inline. MUST be
    // attached OUTSIDE the CompressionLayer; gzip would buffer SSE
    // chunks indefinitely and break keep-alive.
    let sentinel_stream =
        Router::new().route("/api/v1/sentinel/stream", get(sentinel::stream_handler));

    // Operator control endpoint — session/bearer auth via require_api_auth.
    let sentinel_control = Router::new()
        .route("/api/v1/sentinel/control", post(sentinel::control_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_auth,
        ));

    let compressed = Router::new()
        .merge(ingest_routes)
        .merge(span_ingest_routes)
        .merge(key_ingest_routes)
        .merge(analytics_routes)
        .merge(public_share_routes)
        .merge(sentinel_control)
        .merge(auth_routes)
        .merge(root_redirect)
        .merge(dashboard_routes)
        .layer(CompressionLayer::new());

    Router::new()
        .merge(compressed)
        .merge(sentinel_stream)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
