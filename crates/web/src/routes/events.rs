use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::query::{events::EventQuery, pageviews::TimeRange};

use crate::state::AppState;

#[derive(Deserialize, Default)]
pub struct EventsQuery {
    #[serde(default = "default_range")]
    pub range: String,
    pub name: Option<String>,
}

fn default_range() -> String {
    "30d".into()
}

pub async fn events_list(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Query(params): Query<EventsQuery>,
) -> impl IntoResponse {
    let site_id = match Ulid::from_string(&site_id_str) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("<p>Invalid site ID</p>".into()),
    };

    let days: i64 = match params.range.as_str() {
        "7d" => 7,
        "30d" => 30,
        "90d" => 90,
        _ => 30,
    };

    let q = EventQuery {
        site_id,
        range: TimeRange::last_n_days(days),
        event_name: params.name.clone(),
        filters: vec![],
        limit: 50,
    };

    let result = state.backend.query_custom_events(&q).await.unwrap_or_default();
    let site = state.meta.get_site(site_id).await.ok().flatten();

    let tmpl = state.templates.get_template("events.html").unwrap();
    axum::response::Html(
        tmpl.render(minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            range => params.range,
            events => serde_json::to_value(&result.rows).unwrap(),
        })
        .unwrap_or_else(|e| format!("<p>Template error: {e}</p>")),
    )
}
