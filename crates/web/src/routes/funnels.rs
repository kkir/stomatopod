use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    Form,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::{
    domain::org::Funnel,
    query::funnel::{FunnelQuery, FunnelStep},
};

use crate::{
    error::AppError,
    extractors::{Range, SiteId},
    state::AppState,
    templates,
};

pub async fn funnels_page(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Range { label, .. }: Range,
) -> Result<Response, AppError> {
    let funnels = state.meta.list_funnels(site_id).await?;
    let site = state
        .meta
        .get_site(site_id)
        .await?
        .ok_or(AppError::NotFound("site not found"))?;

    let html = templates::render(
        &state,
        "funnels.jinja",
        minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            funnels => serde_json::to_value(&funnels).unwrap(),
            range => label,
        },
    )?;
    Ok(html.into_response())
}

pub async fn funnel_detail(
    State(state): State<Arc<AppState>>,
    Path((site_id_str, funnel_id_str)): Path<(String, String)>,
    Range { range, label }: Range,
) -> Result<Response, AppError> {
    let site_id =
        Ulid::from_string(&site_id_str).map_err(|_| AppError::BadRequest("invalid site id"))?;
    let funnel_id =
        Ulid::from_string(&funnel_id_str).map_err(|_| AppError::BadRequest("invalid funnel id"))?;

    let funnel = state
        .meta
        .get_funnel(funnel_id)
        .await?
        .ok_or(AppError::NotFound("funnel not found"))?;

    let steps: Vec<FunnelStep> = serde_json::from_str(&funnel.definition).unwrap_or_default();
    let q = FunnelQuery {
        site_id,
        range,
        steps,
        window_secs: 86400,
    };

    let result = state.backend.query_funnel(&q).await?;
    let site = state
        .meta
        .get_site(site_id)
        .await?
        .ok_or(AppError::NotFound("site not found"))?;

    let html = templates::render(
        &state,
        "funnels.jinja",
        minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            funnel => serde_json::to_value(&funnel).unwrap(),
            result => serde_json::to_value(&result).unwrap(),
            range => label,
        },
    )?;
    Ok(html.into_response())
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
        Ok(_) => {
            Redirect::to(&format!("/app/sites/{site_id_str}/funnels/{}", funnel.id)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
