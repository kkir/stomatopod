use axum::response::IntoResponse;
use std::sync::Arc;

use axum::extract::State;
use stomatopod_core::query::pageviews::TopListField;

use crate::{
    error::AppError,
    extractors::{Range, SiteId},
    state::AppState,
    templates,
};

async fn top_partial(
    state: &AppState,
    site_id: ulid::Ulid,
    range: stomatopod_core::query::pageviews::TimeRange,
    field: TopListField,
) -> Result<axum::response::Html<String>, AppError> {
    let result = state
        .backend
        .query_top_list(site_id, field, &range, 20)
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
    Range { range, .. }: Range,
) -> Result<impl IntoResponse, AppError> {
    top_partial(&state, site_id, range, TopListField::Page).await
}

pub async fn top_referrers(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Range { range, .. }: Range,
) -> Result<impl IntoResponse, AppError> {
    top_partial(&state, site_id, range, TopListField::Referrer).await
}

pub async fn top_countries(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Range { range, .. }: Range,
) -> Result<impl IntoResponse, AppError> {
    top_partial(&state, site_id, range, TopListField::Country).await
}

pub async fn top_browsers(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Range { range, .. }: Range,
) -> Result<impl IntoResponse, AppError> {
    top_partial(&state, site_id, range, TopListField::Browser).await
}

pub async fn top_devices(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Range { range, .. }: Range,
) -> Result<impl IntoResponse, AppError> {
    top_partial(&state, site_id, range, TopListField::Device).await
}
