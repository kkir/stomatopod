mod alerts;
mod campaigns;
mod docs;
mod events;
mod funnel_detail;
mod funnels;
mod keys;
mod not_found;
mod site_keys;
mod site_overview;
mod site_settings;
mod sites_index;

pub use alerts::Alerts;
pub use campaigns::Campaigns;
pub use docs::Docs;
pub use events::Events;
pub use funnel_detail::FunnelDetail;
pub use funnels::Funnels;
pub use keys::Keys;
pub use not_found::NotFound;
pub use site_keys::SiteKeys;
pub use site_overview::SiteOverview;
pub use site_settings::SiteSettings;
pub use sites_index::SitesIndex;

use dioxus::prelude::*;

/// Shared control styles, ported from the legacy legacy dashboard stylesheet buttons and
/// form controls. Used across the insight/management pages so the raw
/// `button`/`a`/`input` elements (which the `Button` component can't cover,
/// e.g. `type="submit"` or `<a>` links) stay visually consistent.
pub(crate) const BTN_PRIMARY: &str = "inline-flex items-center gap-1.5 px-[15px] py-2 rounded-[10px] text-[13px] font-semibold tracking-tight cursor-pointer bg-grad-btn text-[#032621] shadow-glow";
pub(crate) const BTN_GHOST: &str = "inline-flex items-center gap-1.5 px-[13px] py-1.5 rounded-[10px] text-[12px] font-semibold tracking-tight cursor-pointer bg-text-1/3 text-text-2 border border-border-2 shadow-inner-hi hover:text-text-1 no-underline";
/// Quiet action for the site-tab bar (filter / export) - lighter than BTN_GHOST.
pub(crate) const BTN_TAB_ACTION: &str = "inline-flex items-center gap-1.5 h-7 px-2.5 rounded-lg text-[12px] font-semibold tracking-tight cursor-pointer text-muted-1 hover:text-text-1 hover:bg-text-1/5 border border-transparent hover:border-border-1 transition-colors";
pub(crate) const BTN_TAB_ACTION_ON: &str = "inline-flex items-center gap-1.5 h-7 px-2.5 rounded-lg text-[12px] font-semibold tracking-tight cursor-pointer text-teal-hi bg-teal-soft border border-teal/25";
pub(crate) const CTRL_INPUT: &str = "bg-black/32 border border-border-2 text-text-1 rounded-[10px] px-3 py-2 text-[13px] shadow-inner-hi focus:outline-none focus:border-teal/55";
/// Compact control matching range-tab height (page-head toolbar).
pub(crate) const CTRL_TOOLBAR: &str = "h-9 inline-flex items-center bg-surface-2/80 border border-border-1 text-text-1 rounded-[11px] px-3.5 text-[12.5px] font-semibold shadow-inner-hi focus:outline-none focus:border-teal/55 cursor-pointer hover:text-text-1 appearance-none";

use crate::ui::api::{get_json, ApiError};
use crate::ui::components::pill::FilterPill;
use crate::ui::query::{humanize_filter, DashQuery};
use crate::ui::routes::Route;
use crate::ui::types::{SiteSummary, SitesList};

/// Last successful org site list. Survives route remounts so per-site tab
/// switches can keep showing the resolved name while a fresh fetch is pending
/// (without this, headers flash the raw ULID every navigation).
static SITES_CACHE: GlobalSignal<Option<SitesList>> = Signal::global(|| None);

fn site_from_list(list: &SitesList, site_id: &str) -> Option<SiteSummary> {
    list.sites.iter().find(|s| s.id == site_id).cloned()
}

/// Store a successful sites list for later mounts (and drop it after mutations).
pub(crate) fn remember_sites(list: SitesList) {
    *SITES_CACHE.write() = Some(list);
}

/// Clear the shared list after create/delete so the next fetch is authoritative.
pub(crate) fn invalidate_sites_cache() {
    *SITES_CACHE.write() = None;
}

/// Shared fetch of `/api/v1/sites`. On success updates [`SITES_CACHE`].
pub(crate) fn use_sites_list() -> Resource<Result<SitesList, ApiError>> {
    use_resource(move || async move {
        let result = get_json::<SitesList>("/api/v1/sites").await;
        if let Ok(ref list) = result {
            remember_sites(list.clone());
        }
        result
    })
}

/// Resolve a site from an in-flight list resource, falling back to the shared
/// cache while the resource is still pending.
pub(crate) fn site_from_resource(
    sites: &Resource<Result<SitesList, ApiError>>,
    site_id: &str,
) -> Option<SiteSummary> {
    if let Some(Ok(list)) = sites.read().as_ref() {
        return site_from_list(list, site_id);
    }
    SITES_CACHE
        .read()
        .as_ref()
        .and_then(|list| site_from_list(list, site_id))
}

/// Resolves a site's display name from the org site list, for per-site page
/// headers (there is no single-site GET endpoint). Prefers an in-flight
/// fetch, then the shared cache, then the raw id so a header never renders
/// blank and tab switches do not flash the ULID. Call at the top of a
/// component like any other hook.
pub(crate) fn use_site_name(site_id: String) -> String {
    use_site_summary(site_id.clone())
        .map(|s| s.name)
        .unwrap_or(site_id)
}

/// Resolves a full [`SiteSummary`] from the shared sites list / cache.
pub(crate) fn use_site_summary(site_id: String) -> Option<SiteSummary> {
    let sites = use_sites_list();
    site_from_resource(&sites, &site_id)
}

/// Builds `/api/v1/sites/{site_id}/{endpoint}[?qs]` for a site-scoped GET,
/// reusing a [`DashQuery`]'s rendered query string (see `query.rs`).
pub(crate) fn site_api_url(site_id: &str, endpoint: &str, qs: &str) -> String {
    if qs.is_empty() {
        format!("/api/v1/sites/{site_id}/{endpoint}")
    } else {
        format!("/api/v1/sites/{site_id}/{endpoint}?{qs}")
    }
}

/// The CSV-export variant of [`site_api_url`], appending `format=csv`.
pub(crate) fn site_csv_url(site_id: &str, endpoint: &str, qs: &str) -> String {
    if qs.is_empty() {
        format!("/api/v1/sites/{site_id}/{endpoint}?format=csv")
    } else {
        format!("/api/v1/sites/{site_id}/{endpoint}?{qs}&format=csv")
    }
}

/// Active-filter chips for any page carrying a [`DashQuery`]. Removing a
/// chip navigates to the same route with that filter token dropped.
/// Renders nothing when there are no active filters.
pub(crate) fn active_filters(route: &Route, q: &DashQuery) -> Element {
    let route = route.clone();
    let q = q.clone();
    rsx! {
        if !q.filters.is_empty() {
            div { class: "flex flex-wrap gap-2 mb-4",
                for filter in q.filters.clone() {
                    FilterPill {
                        key: "{filter}",
                        label: humanize_filter(&filter),
                        on_remove: {
                            let route = route.clone();
                            let q = q.clone();
                            let filter = filter.clone();
                            move |_| {
                                navigator().push(route.with_query(q.without_filter(&filter)));
                            }
                        },
                    }
                }
            }
        }
    }
}
