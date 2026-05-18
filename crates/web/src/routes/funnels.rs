use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
    Form,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::{
    domain::org::Funnel,
    query::{
        funnel::{FunnelQuery, FunnelStep},
        pageviews::TimeRange,
    },
};

use crate::state::AppState;

#[derive(Deserialize, Default)]
pub struct FunnelListQuery {
    #[serde(default = "default_range")]
    pub range: String,
}

fn default_range() -> String {
    "30d".into()
}

pub async fn funnels_page(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Query(params): Query<FunnelListQuery>,
) -> impl IntoResponse {
    let site_id = match Ulid::from_string(&site_id_str) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("<p>Invalid site ID</p>".into()),
    };

    let funnels = state.meta.list_funnels(site_id).await.unwrap_or_default();
    let site = state.meta.get_site(site_id).await.ok().flatten();

    let tmpl = state.templates.get_template("funnels.html").unwrap();
    axum::response::Html(
        tmpl.render(minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            funnels => serde_json::to_value(&funnels).unwrap(),
            range => params.range,
        })
        .unwrap_or_else(|e| format!("<p>Template error: {e}</p>")),
    )
}

pub async fn funnel_detail(
    State(state): State<Arc<AppState>>,
    Path((site_id_str, funnel_id_str)): Path<(String, String)>,
    Query(params): Query<FunnelListQuery>,
) -> impl IntoResponse {
    let site_id = match Ulid::from_string(&site_id_str) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("<p>Invalid site ID</p>".into()),
    };
    let funnel_id = match Ulid::from_string(&funnel_id_str) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("<p>Invalid funnel ID</p>".into()),
    };

    let funnel = match state.meta.get_funnel(funnel_id).await.ok().flatten() {
        Some(f) => f,
        None => return axum::response::Html("<p>Funnel not found</p>".into()),
    };

    let steps: Vec<FunnelStep> = serde_json::from_str(&funnel.definition).unwrap_or_default();
    let days: i64 = match params.range.as_str() {
        "7d" => 7,
        "30d" => 30,
        "90d" => 90,
        _ => 30,
    };

    let q = FunnelQuery {
        site_id,
        range: TimeRange::last_n_days(days),
        steps,
        window_secs: 86400,
    };

    let result = state.backend.query_funnel(&q).await.unwrap_or_default();
    let site = state.meta.get_site(site_id).await.ok().flatten();

    let tmpl = state.templates.get_template("funnels.html").unwrap();
    axum::response::Html(
        tmpl.render(minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            funnel => serde_json::to_value(&funnel).unwrap(),
            result => serde_json::to_value(&result).unwrap(),
            range => params.range,
        })
        .unwrap_or_else(|e| format!("<p>Template error: {e}</p>")),
    )
}

#[derive(Deserialize)]
pub struct CreateFunnelForm {
    pub name: String,
    pub steps_json: String,
}

pub async fn create_funnel(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Form(form): Form<CreateFunnelForm>,
) -> impl IntoResponse {
    let site_id = match Ulid::from_string(&site_id_str) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid site ID").into_response(),
    };

    // Validate that steps_json parses
    if serde_json::from_str::<Vec<FunnelStep>>(&form.steps_json).is_err() {
        return (StatusCode::BAD_REQUEST, "Invalid funnel steps JSON").into_response();
    }

    let funnel = Funnel {
        id: Ulid::new(),
        site_id,
        name: form.name,
        definition: form.steps_json,
        created_at: Utc::now(),
    };

    match state.meta.create_funnel(&funnel).await {
        Ok(_) => Redirect::to(&format!("/sites/{site_id_str}/funnels/{}", funnel.id))
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
