use std::sync::Arc;

use stomatopod_core::{
    error::StoreError,
    query::pageviews::{PageviewsQuery, PageviewsResult, TopList, TopListField},
    traits::StorageBackend,
};

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
    let range = &q.range;

    // Six concurrent queries: pageviews plus one per `TopListField` variant.
    // `backend` is `&Arc<dyn …>` so each call site borrows it without cloning;
    // `range` is borrowed too, so no `TimeRange` clones across the join.
    let top = |field| backend.query_top_list(site_id, field, range, limit);
    let (pageviews, top_pages, top_referrers, top_countries, top_browsers, top_devices) = tokio::try_join!(
        backend.query_pageviews(q),
        top(TopListField::Page),
        top(TopListField::Referrer),
        top(TopListField::Country),
        top(TopListField::Browser),
        top(TopListField::Device),
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
