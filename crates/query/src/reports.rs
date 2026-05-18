use std::sync::Arc;

use stomatopod_core::{
    error::StoreError,
    query::pageviews::{PageviewsQuery, PageviewsResult, TimeRange, TopList},
    traits::StorageBackend,
};
use ulid::Ulid;

/// Fetches all dashboard data for a site in parallel.
pub struct DashboardReport {
    pub pageviews: PageviewsResult,
    pub top_pages: TopList,
    pub top_referrers: TopList,
    pub top_countries: TopList,
    pub top_browsers: TopList,
    pub top_devices: TopList,
}

pub async fn fetch_dashboard(
    backend: &Arc<dyn StorageBackend>,
    q: &PageviewsQuery,
    limit: u32,
) -> Result<DashboardReport, StoreError> {
    let site_id = q.site_id;
    let range = q.range.clone();
    let b = backend.clone();

    let (pageviews, top_pages, top_referrers, top_countries, top_browsers, top_devices) = tokio::try_join!(
        backend.query_pageviews(q),
        b.query_top_pages(site_id, &range, limit),
        {
            let b = backend.clone();
            let r = range.clone();
            async move { b.query_top_referrers(site_id, &r, limit).await }
        },
        {
            let b = backend.clone();
            let r = range.clone();
            async move { b.query_top_countries(site_id, &r, limit).await }
        },
        {
            let b = backend.clone();
            let r = range.clone();
            async move { b.query_top_browsers(site_id, &r, limit).await }
        },
        {
            let b = backend.clone();
            let r = range.clone();
            async move { b.query_top_devices(site_id, &r, limit).await }
        },
    )?;

    Ok(DashboardReport {
        pageviews,
        top_pages,
        top_referrers,
        top_countries,
        top_browsers,
        top_devices,
    })
}
