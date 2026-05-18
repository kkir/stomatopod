use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::query::pageviews::TimeRange;

use crate::state::AppState;

#[derive(Deserialize, Default)]
pub struct PartialQuery {
    #[serde(default = "default_range")]
    pub range: String,
}

fn default_range() -> String {
    "30d".into()
}

fn parse_days(range: &str) -> i64 {
    match range {
        "7d" => 7,
        "30d" => 30,
        "90d" => 90,
        _ => 30,
    }
}

fn render_rows(state: &AppState, template: &str, rows: &serde_json::Value) -> axum::response::Html<String> {
    let tmpl = state.templates.get_template(template).unwrap();
    axum::response::Html(
        tmpl.render(minijinja::context! { rows => rows })
            .unwrap_or_default(),
    )
}

pub async fn top_pages(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Query(params): Query<PartialQuery>,
) -> impl IntoResponse {
    let site_id = match Ulid::from_string(&site_id_str) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("<p>Invalid site ID</p>".into()),
    };
    let range = TimeRange::last_n_days(parse_days(&params.range));
    let result = state.backend.query_top_pages(site_id, &range, 20).await.unwrap_or_default();
    let rows = serde_json::to_value(&result.rows).unwrap();
    render_rows(&state, "partials/top_pages.html", &rows)
}

pub async fn top_referrers(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Query(params): Query<PartialQuery>,
) -> impl IntoResponse {
    let site_id = match Ulid::from_string(&site_id_str) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("<p>Invalid site ID</p>".into()),
    };
    let range = TimeRange::last_n_days(parse_days(&params.range));
    let result = state.backend.query_top_referrers(site_id, &range, 20).await.unwrap_or_default();
    let rows = serde_json::to_value(&result.rows).unwrap();
    render_rows(&state, "partials/top_referrers.html", &rows)
}

pub async fn top_countries(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Query(params): Query<PartialQuery>,
) -> impl IntoResponse {
    let site_id = match Ulid::from_string(&site_id_str) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("<p>Invalid site ID</p>".into()),
    };
    let range = TimeRange::last_n_days(parse_days(&params.range));
    let result = state.backend.query_top_countries(site_id, &range, 20).await.unwrap_or_default();
    let rows = serde_json::to_value(&result.rows).unwrap();
    render_rows(&state, "partials/top_countries.html", &rows)
}

pub async fn top_browsers(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Query(params): Query<PartialQuery>,
) -> impl IntoResponse {
    let site_id = match Ulid::from_string(&site_id_str) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("<p>Invalid site ID</p>".into()),
    };
    let range = TimeRange::last_n_days(parse_days(&params.range));
    let result = state.backend.query_top_browsers(site_id, &range, 20).await.unwrap_or_default();
    let rows = serde_json::to_value(&result.rows).unwrap();
    render_rows(&state, "partials/top_browsers.html", &rows)
}

pub async fn top_devices(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Query(params): Query<PartialQuery>,
) -> impl IntoResponse {
    let site_id = match Ulid::from_string(&site_id_str) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("<p>Invalid site ID</p>".into()),
    };
    let range = TimeRange::last_n_days(parse_days(&params.range));
    let result = state.backend.query_top_devices(site_id, &range, 20).await.unwrap_or_default();
    let rows = serde_json::to_value(&result.rows).unwrap();
    render_rows(&state, "partials/top_devices.html", &rows)
}
