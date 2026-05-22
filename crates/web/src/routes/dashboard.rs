use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
};
use std::sync::Arc;

use stomatopod_core::query::pageviews::{Granularity, PageviewsQuery};
use stomatopod_query::reports::fetch_dashboard;

use crate::{
    error::AppError,
    extractors::{Range, SiteId},
    state::AppState,
    templates,
};

pub async fn index(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    let orgs = state.meta.list_orgs().await?;
    let sites = match orgs.first() {
        Some(org) => state.meta.list_sites(org.id).await?,
        None => vec![],
    };

    if sites.is_empty() {
        return Ok(templates::render(
            &state,
            "index.html",
            minijinja::context! { sites => [] as [i32; 0] },
        )?
        .into_response());
    }

    Ok(Redirect::to(&format!("/app/sites/{}", sites[0].id)).into_response())
}

pub async fn site_overview(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    Range { range, label }: Range,
) -> Result<Response, AppError> {
    let query = PageviewsQuery {
        site_id,
        range,
        granularity: Granularity::Day,
        filters: vec![],
    };

    let report = fetch_dashboard(&state.backend, &query, 20).await?;

    let site = state
        .meta
        .get_site(site_id)
        .await?
        .ok_or(AppError::NotFound("site not found"))?;

    let html = templates::render(
        &state,
        "site.html",
        minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            range => label,
            total_pageviews => report.pageviews.total_pageviews,
            total_sessions => report.pageviews.total_sessions,
            bounce_rate => format!("{:.1}%", report.pageviews.bounce_rate * 100.0),
            top_pages => serde_json::to_value(&report.top_pages.rows).unwrap(),
            top_referrers => serde_json::to_value(&report.top_referrers.rows).unwrap(),
            top_countries => serde_json::to_value(&report.top_countries.rows).unwrap(),
            top_browsers => serde_json::to_value(&report.top_browsers.rows).unwrap(),
            top_devices => serde_json::to_value(&report.top_devices.rows).unwrap(),
            buckets => serde_json::to_value(&report.pageviews.buckets).unwrap(),
        },
    )?;

    Ok(html.into_response())
}
