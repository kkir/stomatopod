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
        agents_dashboard, analytics, api, auth, dashboard, events, funnels, marketing, partials,
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

    // Public marketing site (no auth, no CORS).
    let marketing_routes = Router::new()
        .route("/", get(marketing::home))
        .route("/web-analytics", get(marketing::web_analytics))
        .route("/ai-firewall", get(marketing::ai_firewall))
        .route("/for/saas", get(marketing::for_saas))
        .route("/for/agencies", get(marketing::for_agencies))
        .route("/for/ai-teams", get(marketing::for_ai_teams))
        .route("/for/regulated", get(marketing::for_regulated));

    // Hash-busted static assets for the marketing site. The hash is part of
    // the URL (rendered into the marketing templates) so the response can be
    // cached for a year.
    let marketing_css_path = format!("/static/marketing.{}.css", state.marketing_css_hash);
    let anime_js_path = format!("/static/anime.{}.js", state.anime_js_hash);
    let marketing_assets = Router::new()
        .route(&marketing_css_path, get(api::marketing_css))
        .route(&anime_js_path, get(api::anime_js));

    // Sentinel span ingest. Bearer-auth'd via sentinel_tokens (handler
    // checks the header itself; no middleware needed). No CORS since
    // calls come from sidecars, not browsers.
    let span_ingest_routes =
        Router::new().route("/api/v1/spans", post(spans::handle_span_ingest_route));

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

    // Protected dashboard routes — live under /app so / can serve marketing.
    let dashboard_routes = Router::new()
        .route("/app", get(dashboard::index))
        .route(
            "/app/sites",
            get(sites::sites_list).post(sites::create_site),
        )
        .route("/app/sites/:site_id", get(dashboard::site_overview))
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
        .merge(analytics_routes)
        .merge(sentinel_control)
        .merge(auth_routes)
        .merge(marketing_routes)
        .merge(marketing_assets)
        .merge(dashboard_routes)
        .layer(CompressionLayer::new());

    Router::new()
        .merge(compressed)
        .merge(sentinel_stream)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
