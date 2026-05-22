use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::query::pageviews::{TimeRange, TopListField};

use crate::state::AppState;

#[derive(Deserialize, Default)]
pub struct PartialQuery {
    #[serde(default = "default_range")]
    pub range: String,
}

fn default_range() -> String {
    "30d".into()
}

fn render_rows(
    state: &AppState,
    template: &str,
    rows: &serde_json::Value,
) -> axum::response::Html<String> {
    let tmpl = state.templates.get_template(template).unwrap();
    axum::response::Html(
        tmpl.render(minijinja::context! { rows => rows })
            .unwrap_or_default(),
    )
}

async fn top_partial(
    state: &AppState,
    site_id_str: &str,
    range_label: &str,
    field: TopListField,
) -> axum::response::Html<String> {
    let Ok(site_id) = Ulid::from_string(site_id_str) else {
        return axum::response::Html("<p>Invalid site ID</p>".into());
    };
    let range = TimeRange::from_label(range_label);
    let result = state
        .backend
        .query_top_list(site_id, field, &range, 20)
        .await
        .unwrap_or_default();
    let rows = serde_json::to_value(&result.rows).unwrap();
    render_rows(state, field.template_partial(), &rows)
}

pub async fn top_pages(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Query(params): Query<PartialQuery>,
) -> impl IntoResponse {
    top_partial(&state, &site_id_str, &params.range, TopListField::Page).await
}

pub async fn top_referrers(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Query(params): Query<PartialQuery>,
) -> impl IntoResponse {
    top_partial(&state, &site_id_str, &params.range, TopListField::Referrer).await
}

pub async fn top_countries(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Query(params): Query<PartialQuery>,
) -> impl IntoResponse {
    top_partial(&state, &site_id_str, &params.range, TopListField::Country).await
}

pub async fn top_browsers(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Query(params): Query<PartialQuery>,
) -> impl IntoResponse {
    top_partial(&state, &site_id_str, &params.range, TopListField::Browser).await
}

pub async fn top_devices(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Query(params): Query<PartialQuery>,
) -> impl IntoResponse {
    top_partial(&state, &site_id_str, &params.range, TopListField::Device).await
}
