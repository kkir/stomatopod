use axum::response::IntoResponse;
use std::sync::Arc;

use axum::extract::State;
use stomatopod_core::query::pageviews::TopListField;

use crate::{
    error::AppError,
    extractors::{DashQuery, SiteId},
    state::AppState,
    templates,
};

async fn top_partial(
    state: &AppState,
    site_id: ulid::Ulid,
    dq: &DashQuery,
    field: TopListField,
) -> Result<axum::response::Html<String>, AppError> {
    let result = state
        .backend
        .query_top_list(site_id, field, &dq.range, 20, &dq.filters)
        .await?;
    let rows = serde_json::to_value(&result.rows).unwrap();
    templates::render(
        state,
        field.template_partial(),
        minijinja::context! { rows => rows },
    )
}

pub async fn top_pages(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    dq: DashQuery,
) -> Result<impl IntoResponse, AppError> {
    top_partial(&state, site_id, &dq, TopListField::Page).await
}

pub async fn top_referrers(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    dq: DashQuery,
) -> Result<impl IntoResponse, AppError> {
    top_partial(&state, site_id, &dq, TopListField::Referrer).await
}

pub async fn top_countries(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    dq: DashQuery,
) -> Result<impl IntoResponse, AppError> {
    top_partial(&state, site_id, &dq, TopListField::Country).await
}

pub async fn top_browsers(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    dq: DashQuery,
) -> Result<impl IntoResponse, AppError> {
    top_partial(&state, site_id, &dq, TopListField::Browser).await
}

pub async fn top_devices(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    dq: DashQuery,
) -> Result<impl IntoResponse, AppError> {
    top_partial(&state, site_id, &dq, TopListField::Device).await
}

pub async fn top_os(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    dq: DashQuery,
) -> Result<impl IntoResponse, AppError> {
    top_partial(&state, site_id, &dq, TopListField::Os).await
}

pub async fn top_regions(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    dq: DashQuery,
) -> Result<impl IntoResponse, AppError> {
    top_partial(&state, site_id, &dq, TopListField::Region).await
}
