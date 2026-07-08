mod alerts;
mod campaigns;
mod compare;
mod docs;
mod events;
mod funnel_detail;
mod funnels;
mod global_alerts;
mod global_goals;
mod global_realtime;
mod goals;
mod keys;
mod not_found;
mod paths;
mod realtime;
mod retention;
mod site_keys;
mod site_overview;
mod site_settings;
mod sites_index;

pub use alerts::Alerts;
pub use campaigns::Campaigns;
pub use compare::Compare;
pub use docs::Docs;
pub use events::Events;
pub use funnel_detail::FunnelDetail;
pub use funnels::Funnels;
pub use global_alerts::GlobalAlerts;
pub use global_goals::GlobalGoals;
pub use global_realtime::GlobalRealtime;
pub use goals::Goals;
pub use keys::Keys;
pub use not_found::NotFound;
pub use paths::Paths;
pub use realtime::Realtime;
pub use retention::Retention;
pub use site_keys::SiteKeys;
pub use site_overview::SiteOverview;
pub use site_settings::SiteSettings;
pub use sites_index::SitesIndex;

use dioxus::prelude::*;

/// Shared control styles, ported from the legacy dashboard.css buttons and
/// form controls. Used across the insight/management pages so the raw
/// `button`/`a`/`input` elements (which the `Button` component can't cover,
/// e.g. `type="submit"` or `<a>` links) stay visually consistent.
pub(crate) const BTN_PRIMARY: &str = "inline-flex items-center gap-1.5 px-[15px] py-2 rounded-[10px] text-[13px] font-semibold tracking-tight cursor-pointer bg-grad-btn text-[#032621] shadow-glow";
pub(crate) const BTN_GHOST: &str = "inline-flex items-center gap-1.5 px-[13px] py-1.5 rounded-[10px] text-[12px] font-semibold tracking-tight cursor-pointer bg-text-1/3 text-text-2 border border-border-2 shadow-inner-hi hover:text-text-1 no-underline";
pub(crate) const CTRL_INPUT: &str = "bg-black/32 border border-border-2 text-text-1 rounded-[10px] px-3 py-2 text-[13px] shadow-inner-hi focus:outline-none focus:border-teal/55";

use crate::ui::api::get_json;
use crate::ui::components::form::SelectField;
use crate::ui::components::pill::FilterPill;
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
use crate::ui::types::{SiteSummary, SitesList};

/// Resolves a site's display name from the org site list, for per-site page
/// headers (there is no single-site GET endpoint). Falls back to the raw id
/// while the list loads or if the site isn't found, so a header never renders
/// blank. Call at the top of a component like any other hook.
pub(crate) fn use_site_name(site_id: String) -> String {
    let sites = use_resource(move || async move { get_json::<SitesList>("/api/v1/sites").await });
    let guard = sites.read();
    match guard.as_ref() {
        Some(Ok(list)) => list
            .sites
            .iter()
            .find(|s| s.id == site_id)
            .map(|s| s.name.clone())
            .unwrap_or(site_id),
        _ => site_id,
    }
}

/// A site picker for the global insight pages (campaigns/retention/paths/
/// alerts), which the legacy server scoped to one site via `resolve_scope`.
/// Renders nothing extra when there is only one site.
#[component]
pub(crate) fn SiteScopeSelect(
    sites: Vec<SiteSummary>,
    selected: String,
    on_select: EventHandler<String>,
) -> Element {
    if sites.len() < 2 {
        return rsx! {};
    }
    let options = sites
        .iter()
        .map(|s| (s.id.clone(), s.name.clone()))
        .collect::<Vec<_>>();
    rsx! {
        div { class: "max-w-xs mb-5",
            SelectField {
                label: "Site",
                value: selected,
                options,
                onchange: move |v| on_select.call(v),
            }
        }
    }
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
                        label: filter.clone(),
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
