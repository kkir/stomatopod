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
        agents_dashboard, analytics, api, api_keys, auth, dashboard, events, funnels, partials,
        sentinel, sites, spans,
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
    let key_ingest_routes =
        Router::new().route("/api/v1/ingest", post(api::handle_key_ingest));

    // Analytics JSON API routes (bearer token or session auth)
    let analytics_routes = Router::new()
        .route("/api/v1/sites", get(analytics::list_sites))
        .route("/api/v1/sites/:site/pageviews", get(analytics::pageviews))
        .route("/api/v1/sites/:site/top-pages", get(analytics::top_pages))
        .route(
            "/api/v1/sites/:site/top-referrers",
            get(analytics::top_referrers),
        )
        .route("/api/v1/sites/:site/events", get(analytics::events))
        .route("/api/v1/sites/:site/funnels", get(analytics::list_funnels))
        .route(
            "/api/v1/sites/:site/funnels/:funnel_id",
            get(analytics::funnel_result),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_auth,
        ));

    // Auth routes (no auth required)
    let auth_routes = Router::new()
        .route("/login", get(auth::login_page).post(auth::login_submit))
        .route("/logout", post(auth::logout));

    // Protected dashboard routes.
    let dashboard_routes = Router::new()
        .route("/app", get(dashboard::index))
        .route("/app/docs", get(api::docs_page))
        .route(
            "/app/sites",
            get(sites::sites_list).post(sites::create_site),
        )
        .route(
            "/app/keys",
            get(api_keys::global_keys_page).post(api_keys::global_create_key),
        )
        .route(
            "/app/keys/:key_id/delete",
            post(api_keys::global_delete_key),
        )
        .route(
            "/app/sites/:site_id",
            get(dashboard::site_overview).post(sites::update_site),
        )
        .route("/app/sites/:site_id/settings", get(sites::site_settings))
        .route(
            "/app/sites/:site_id/keys",
            get(api_keys::keys_page).post(api_keys::create_key),
        )
        .route(
            "/app/sites/:site_id/keys/:key_id/delete",
            post(api_keys::delete_key),
        )
        .route("/app/sites/:site_id/events", get(events::events_list))
        .route(
            "/app/sites/:site_id/funnels",
            get(funnels::funnels_page).post(funnels::create_funnel),
        )
        .route(
            "/app/sites/:site_id/funnels/:funnel_id",
            get(funnels::funnel_detail),
        )
        // AI firewall dashboard
        .route("/app/agents", get(agents_dashboard::agents_index))
        .route("/app/agents/:agent_id", get(agents_dashboard::agent_detail))
        .route(
            "/app/agents/:agent_id/spans",
            get(agents_dashboard::agent_spans_partial),
        )
        .route("/app/incidents", get(agents_dashboard::incidents_page))
        // HTMX partial routes
        .route(
            "/app/sites/:site_id/partials/top-pages",
            get(partials::top_pages),
        )
        .route(
            "/app/sites/:site_id/partials/top-referrers",
            get(partials::top_referrers),
        )
        .route(
            "/app/sites/:site_id/partials/top-countries",
            get(partials::top_countries),
        )
        .route(
            "/app/sites/:site_id/partials/top-browsers",
            get(partials::top_browsers),
        )
        .route(
            "/app/sites/:site_id/partials/top-devices",
            get(partials::top_devices),
        )
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
