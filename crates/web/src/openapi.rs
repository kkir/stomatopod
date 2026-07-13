//! OpenAPI 3 document derived from handler annotations and schema types.
//!
//! Served at `GET /openapi.json` (public). Narrative docs stay in
//! `assets/docs.md` (`/llms.txt`); this file is the machine contract for
//! typed clients and codegen.

use axum::{
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use utoipa::{
    openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    Modify, OpenApi,
};

use crate::routes::{
    analytics::{
        AnalyticsParams, CreateFunnelBody, ErrorBody, EventsParams, FunnelSchema, FunnelStepSchema,
        PageviewsResultSchema, SitesResponse, TopListSchema,
    },
    api::{BrowserIngestBody, MeResponse, ServerIngestBody},
};

/// OpenAPI document for the public + read analytics API (and funnel create).
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Stomatopod API",
        version = "1.0.0",
        description = "Privacy-friendly, cookieless web analytics.\n\n\
            **Auth:** Bearer ingest keys (`sk_live_…`) for `POST /api/v1/ingest`; \
            Bearer read keys (`rk_…`) or a dashboard session cookie for analytics.\n\n\
            **Narrative docs:** `/llms.txt` (Markdown). **This document:** machine-readable contract.",
        license(name = "AGPL-3.0")
    ),
    paths(
        crate::routes::api::handle_ingest,
        crate::routes::api::handle_key_ingest,
        crate::routes::api::me,
        crate::routes::analytics::list_sites,
        crate::routes::analytics::pageviews,
        crate::routes::analytics::top_pages,
        crate::routes::analytics::top_referrers,
        crate::routes::analytics::top_os,
        crate::routes::analytics::top_regions,
        crate::routes::analytics::top_countries,
        crate::routes::analytics::top_browsers,
        crate::routes::analytics::top_devices,
        crate::routes::analytics::top_entry_pages,
        crate::routes::analytics::top_exit_pages,
        crate::routes::analytics::events,
        crate::routes::analytics::list_funnels,
        crate::routes::analytics::create_funnel,
        crate::routes::analytics::funnel_result,
        openapi_json,
    ),
    components(schemas(
        BrowserIngestBody,
        ServerIngestBody,
        AnalyticsParams,
        EventsParams,
        CreateFunnelBody,
        FunnelStepSchema,
        FunnelSchema,
        SitesResponse,
        PageviewsResultSchema,
        TopListSchema,
        ErrorBody,
        MeResponse,
    )),
    tags(
        (name = "ingest", description = "Event ingestion (browser beacon + server-side)"),
        (name = "analytics", description = "Read analytics with a read API key or session"),
        (name = "funnels", description = "Funnel definitions and results"),
        (name = "meta", description = "Session probe and machine docs"),
    ),
    modifiers(&SecurityAddon),
    servers(
        (url = "/", description = "This Stomatopod host")
    )
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "read_key",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("rk_…")
                    .description(Some(
                        "Read-scoped API key (rk_…). Also accepts a dashboard session cookie.",
                    ))
                    .build(),
            ),
        );
        components.add_security_scheme(
            "ingest_key",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("sk_live_…")
                    .description(Some(
                        "Ingest-scoped API key (sk_live_…) bound to a site.",
                    ))
                    .build(),
            ),
        );
    }
}

/// `GET /openapi.json` — OpenAPI 3 document (public, no secrets).
#[utoipa::path(
    get,
    path = "/openapi.json",
    tag = "meta",
    responses(
        (status = 200, description = "OpenAPI 3 JSON document", content_type = "application/json")
    )
)]
pub async fn openapi_json() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "public, max-age=600")],
        Json(ApiDoc::openapi()),
    )
}
