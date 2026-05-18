use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::query::pageviews::{Granularity, PageviewsQuery, TimeRange};
use stomatopod_query::reports::fetch_dashboard;

use crate::state::AppState;

#[derive(Deserialize, Default)]
pub struct DashboardQuery {
    #[serde(default = "default_range")]
    pub range: String,
}

fn default_range() -> String {
    "30d".into()
}

pub async fn index(State(state): State<Arc<AppState>>) -> Response {
    let sites = match state.meta.list_orgs().await {
        Ok(orgs) if !orgs.is_empty() => state.meta.list_sites(orgs[0].id).await.unwrap_or_default(),
        _ => vec![],
    };

    if sites.is_empty() {
        let tmpl = state.templates.get_template("index.html").unwrap();
        return axum::response::Html(
            tmpl.render(minijinja::context! { sites => [] as [i32;0] })
                .unwrap(),
        )
        .into_response();
    }

    Redirect::to(&format!("/sites/{}", sites[0].id)).into_response()
}

pub async fn site_overview(
    State(state): State<Arc<AppState>>,
    Path(site_id_str): Path<String>,
    Query(params): Query<DashboardQuery>,
) -> Response {
    let site_id = match Ulid::from_string(&site_id_str) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::response::Html("<p>Invalid site ID</p>".to_string()),
            )
                .into_response()
        }
    };

    let range = parse_range(&params.range);
    let query = PageviewsQuery {
        site_id,
        range: range.clone(),
        granularity: Granularity::Day,
        filters: vec![],
    };

    let report = match fetch_dashboard(&state.backend, &query, 20).await {
        Ok(r) => r,
        Err(e) => return axum::response::Html(format!("<p>Query error: {e}</p>")).into_response(),
    };

    let site = match state.meta.get_site(site_id).await.ok().flatten() {
        Some(s) => s,
        None => {
            return (
                StatusCode::NOT_FOUND,
                axum::response::Html("<p>Site not found</p>".to_string()),
            )
                .into_response()
        }
    };

    let tmpl = state.templates.get_template("site.html").unwrap();
    let html = tmpl
        .render(minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            range => params.range,
            total_pageviews => report.pageviews.total_pageviews,
            total_sessions => report.pageviews.total_sessions,
            bounce_rate => format!("{:.1}%", report.pageviews.bounce_rate * 100.0),
            top_pages => serde_json::to_value(&report.top_pages.rows).unwrap(),
            top_referrers => serde_json::to_value(&report.top_referrers.rows).unwrap(),
            top_countries => serde_json::to_value(&report.top_countries.rows).unwrap(),
            top_browsers => serde_json::to_value(&report.top_browsers.rows).unwrap(),
            top_devices => serde_json::to_value(&report.top_devices.rows).unwrap(),
            buckets => serde_json::to_value(&report.pageviews.buckets).unwrap(),
        })
        .unwrap_or_else(|e| format!("<p>Template error: {e}</p>"));

    axum::response::Html(html).into_response()
}

fn parse_range(range: &str) -> TimeRange {
    let days: i64 = match range {
        "7d" => 7,
        "30d" => 30,
        "90d" => 90,
        "12m" => 365,
        _ => 30,
    };
    TimeRange::last_n_days(days)
}
