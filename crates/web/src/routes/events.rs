use axum::{
    extract::{Query, State},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::sync::Arc;

use stomatopod_core::query::events::EventQuery;

use crate::{
    error::AppError,
    extractors::{Range, SiteId},
    state::AppState,
    templates,
};

#[derive(Deserialize, Default)]
pub struct EventsQuery {
    pub name: Option<String>,
}

pub async fn events_list(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Range { range, label }: Range,
    Query(params): Query<EventsQuery>,
) -> Result<Response, AppError> {
    let q = EventQuery {
        site_id,
        range,
        event_name: params.name.clone(),
        filters: vec![],
        limit: 50,
    };

    let result = state.backend.query_custom_events(&q).await?;
    let site = state
        .meta
        .get_site(site_id)
        .await?
        .ok_or(AppError::NotFound("site not found"))?;

    let html = templates::render(
        &state,
        "events.jinja",
        minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            range => label,
            events => serde_json::to_value(&result.rows).unwrap(),
        },
    )?;
    Ok(html.into_response())
}
