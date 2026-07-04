use dioxus::prelude::*;

use crate::query::DashQuery;
use crate::routes::Route;

const RANGES: [&str; 4] = ["7d", "30d", "90d", "12m"];

/// The 7d/30d/90d/12m pills, port of `.range-tabs` (dashboard.css Range
/// tabs section). Navigates by swapping `range` on the current route
/// while preserving filters/compare.
#[component]
pub fn RangeTabs(active: String) -> Element {
    let route = use_route::<Route>();
    let current = route.query().cloned().unwrap_or_default();
    rsx! {
        div { class: "inline-flex gap-0.5 p-[3px] rounded-[11px] bg-surface-2/80 border border-border-1 shadow-inner-hi",
            for range in RANGES {
                Link {
                    key: "{range}",
                    to: route.with_query(current.with_range(range)),
                    class: if active == range {
                        "px-3 py-1.5 rounded-lg text-xs font-semibold bg-grad-btn text-[#032621] shadow-[inset_0_1px_0_rgba(255,255,255,.35),0_1px_6px_rgba(45,212,191,.45)]"
                    } else {
                        "px-3 py-1.5 rounded-lg text-xs font-semibold text-muted-1 hover:text-text-1"
                    },
                    "{range}"
                }
            }
        }
    }
}

/// Which per-site tab is current, port of the top-level `.tabs` row
/// repeated across every per-site page.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SiteTab {
    Overview,
    Realtime,
    Events,
    Goals,
    Funnels,
    Alerts,
    Keys,
    Settings,
}

/// The Overview/Real-time/Events/Goals/Funnels/Alerts/API Keys/Settings
/// tab row shown on every per-site page.
#[component]
pub fn SiteTabs(site_id: String, range: String, active: SiteTab) -> Element {
    let q = DashQuery {
        range: Some(range),
        ..Default::default()
    };
    let tabs: [(SiteTab, &str, Route); 8] = [
        (
            SiteTab::Overview,
            "Overview",
            Route::SiteOverview {
                site_id: site_id.clone(),
                q: q.clone(),
            },
        ),
        (
            SiteTab::Realtime,
            "Real-time",
            Route::Realtime {
                site_id: site_id.clone(),
            },
        ),
        (
            SiteTab::Events,
            "Events",
            Route::Events {
                site_id: site_id.clone(),
                q: q.clone(),
            },
        ),
        (
            SiteTab::Goals,
            "Goals",
            Route::Goals {
                site_id: site_id.clone(),
                q: q.clone(),
            },
        ),
        (
            SiteTab::Funnels,
            "Funnels",
            Route::Funnels {
                site_id: site_id.clone(),
                q: q.clone(),
            },
        ),
        (
            SiteTab::Alerts,
            "Alerts",
            Route::Alerts {
                site_id: site_id.clone(),
            },
        ),
        (
            SiteTab::Keys,
            "API Keys",
            Route::SiteKeys {
                site_id: site_id.clone(),
            },
        ),
        (
            SiteTab::Settings,
            "Settings",
            Route::SiteSettings { site_id },
        ),
    ];
    rsx! {
        div { class: "flex gap-f4 border-b border-border-1 mb-7 relative",
            for (tab , label , to) in tabs {
                Link {
                    key: "{label}",
                    to,
                    class: if tab == active {
                        "pb-[11px] text-[13.5px] font-medium text-text-1 relative after:content-[''] after:absolute after:left-0 after:right-0 after:-bottom-px after:h-0.5 after:rounded-full after:bg-iri after:shadow-[0_0_10px_rgba(45,212,191,0.55)]"
                    } else {
                        "pb-[11px] text-[13.5px] font-medium text-muted-1 hover:text-text-2"
                    },
                    "{label}"
                }
            }
        }
    }
}
