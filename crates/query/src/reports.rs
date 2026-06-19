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
    pub top_os: TopList,
    pub top_regions: TopList,
    /// Pageview totals for the immediately preceding, equal-length period.
    /// `Some` only when comparison was requested; powers the delta badges.
    pub comparison: Option<PageviewsResult>,
}

pub async fn fetch_dashboard(
    backend: &Arc<dyn StorageBackend>,
    q: &PageviewsQuery,
    limit: u32,
    compare: bool,
) -> Result<DashboardReport, StoreError> {
    let site_id = q.site_id;
    let range = &q.range;
    let filters = q.filters.as_slice();

    // Concurrent queries: pageviews plus one per `TopListField` variant. All
    // share the active filter set. `backend`/`range` are borrowed so nothing
    // is cloned across the join.
    let top = |field| backend.query_top_list(site_id, field, range, limit, filters);
    let (
        pageviews,
        top_pages,
        top_referrers,
        top_countries,
        top_browsers,
        top_devices,
        top_os,
        top_regions,
    ) = tokio::try_join!(
        backend.query_pageviews(q),
        top(TopListField::Page),
        top(TopListField::Referrer),
        top(TopListField::Country),
        top(TopListField::Browser),
        top(TopListField::Device),
        top(TopListField::Os),
        top(TopListField::Region),
    )?;

    // Period-over-period: re-run the pageviews query over the preceding
    // window of equal length. Filters and granularity carry over so the two
    // numbers are comparable.
    let comparison = if compare {
        let prev = PageviewsQuery {
            site_id,
            range: range.previous(),
            granularity: q.granularity,
            filters: q.filters.clone(),
        };
        Some(backend.query_pageviews(&prev).await?)
    } else {
        None
    };

    Ok(DashboardReport {
        pageviews,
        top_pages,
        top_referrers,
        top_countries,
        top_browsers,
        top_devices,
        top_os,
        top_regions,
        comparison,
    })
}
