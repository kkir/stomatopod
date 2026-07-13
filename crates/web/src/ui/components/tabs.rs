use dioxus::prelude::*;

use crate::ui::query::DashQuery;
use crate::ui::routes::Route;

const RANGES: [&str; 4] = ["7d", "30d", "90d", "12m"];

/// The 7d/30d/90d/12m pills, port of `.range-tabs` (dashboard.css Range
/// tabs section). Navigates by swapping `range` on the current route
/// while preserving filters/compare.
#[component]
pub fn RangeTabs(active: String) -> Element {
    let route = use_route::<Route>();
    let current = route.query().cloned().unwrap_or_default();
    rsx! {
        div {
            class: "inline-flex items-center h-9 gap-0.5 p-[3px] rounded-[11px] bg-surface-2/80 border border-border-1 shadow-inner-hi shrink-0",
            role: "tablist",
            "aria-label": "Date range",
            for range in RANGES {
                Link {
                    key: "{range}",
                    to: route.with_query(current.with_range(range)),
                    class: if active == range {
                        "inline-flex items-center justify-center h-full min-w-[2.4rem] sm:min-w-[2.65rem] px-2.5 sm:px-3.5 rounded-[8px] text-[12px] sm:text-[12.5px] font-semibold bg-grad-btn text-[#032621] shadow-[inset_0_1px_0_rgba(255,255,255,.35),0_1px_6px_rgba(45,212,191,.45)] no-underline"
                    } else {
                        "inline-flex items-center justify-center h-full min-w-[2.4rem] sm:min-w-[2.65rem] px-2.5 sm:px-3.5 rounded-[8px] text-[12px] sm:text-[12.5px] font-semibold text-muted-1 hover:text-text-1 no-underline"
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
    Events,
    Funnels,
    Campaigns,
    Alerts,
    Keys,
    Settings,
}

/// Tab row shown on every per-site page.
///
/// Insight tabs (Overview, Events, Funnels, Campaigns) carry the full
/// [`DashQuery`] so range, filters, and compare survive navigation.
/// Management tabs (Alerts, Keys, Settings) stay query-free.
///
/// Optional `children` render on the right of the tab strip (e.g. filter /
/// export on Overview) so secondary actions do not take a full row below.
#[component]
pub fn SiteTabs(site_id: String, range: String, active: SiteTab, children: Element) -> Element {
    let route = use_route::<Route>();
    let current = route.query().cloned().unwrap_or_else(|| DashQuery {
        range: Some(range.clone()),
        ..Default::default()
    });
    // Prefer the live route query; fall back so pages without `?:..q` still
    // deep-link into overview with at least the requested range.
    let q = if current.range.is_some() || !current.filters.is_empty() || current.compare {
        current
    } else {
        DashQuery {
            range: Some(range),
            ..current
        }
    };

    let tabs: [(SiteTab, &str, Route); 7] = [
        (
            SiteTab::Overview,
            "Overview",
            Route::SiteOverview {
                site_id: site_id.clone(),
                q: q.clone(),
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
            SiteTab::Funnels,
            "Funnels",
            Route::Funnels {
                site_id: site_id.clone(),
                q: q.clone(),
            },
        ),
        (
            SiteTab::Campaigns,
            "Campaigns",
            Route::Campaigns {
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
        div { class: "flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between sm:gap-4 border-b border-border-1 mb-6 sm:mb-8",
            nav {
                class: "flex gap-4 sm:gap-6 min-w-0 overflow-x-auto scrollbar-none -mx-1 px-1",
                "aria-label": "Site sections",
                for (tab , label , to) in tabs {
                    Link {
                        key: "{label}",
                        to,
                        class: if tab == active {
                            "shrink-0 pb-[11px] text-[13px] sm:text-[13.5px] font-medium text-text-1 relative after:content-[''] after:absolute after:left-0 after:right-0 after:-bottom-px after:h-0.5 after:rounded-full after:bg-iri after:shadow-[0_0_10px_rgba(45,212,191,0.55)]"
                        } else {
                            "shrink-0 pb-[11px] text-[13px] sm:text-[13.5px] font-medium text-muted-1 hover:text-text-2"
                        },
                        "{label}"
                    }
                }
            }
            div { class: "flex items-center gap-1.5 shrink-0 pb-1.5 self-end sm:self-auto",
                {children}
            }
        }
    }
}

/// Segment control for switching dimensions inside a card (Pages / Entry / Exit,
/// Countries / Regions, etc.). Parent owns the active index and content.
#[component]
pub fn DimensionTabs(tabs: Vec<String>, active: usize, on_select: EventHandler<usize>) -> Element {
    rsx! {
        div { class: "inline-flex flex-wrap gap-0.5 p-[3px] rounded-[11px] bg-surface-2/80 border border-border-1 shadow-inner-hi",
            for (i, label) in tabs.into_iter().enumerate() {
                button {
                    key: "{label}",
                    r#type: "button",
                    class: if i == active {
                        "px-2.5 py-1 rounded-lg text-[11.5px] font-semibold bg-grad-btn text-[#032621] shadow-[inset_0_1px_0_rgba(255,255,255,.35),0_1px_6px_rgba(45,212,191,.45)] cursor-pointer border-0"
                    } else {
                        "px-2.5 py-1 rounded-lg text-[11.5px] font-semibold text-muted-1 hover:text-text-1 cursor-pointer bg-transparent border-0"
                    },
                    onclick: move |_| on_select.call(i),
                    "{label}"
                }
            }
        }
    }
}

/// Card with a title, optional CSV link, dimension tab strip, and a body slot.
/// Only the parent-mounted children for the active tab should be passed in so
/// inactive dimensions stay unfetched.
#[component]
pub fn TabbedCard(
    title: String,
    tabs: Vec<String>,
    active: usize,
    on_select: EventHandler<usize>,
    csv_href: Option<String>,
    children: Element,
) -> Element {
    rsx! {
        div { class: "relative bg-surface-1 border border-border-1 rounded-xl p-4 sm:p-6 shadow-sm shadow-inner-hi",
            div { class: "flex justify-between items-center mb-4 gap-3 flex-wrap",
                h2 { class: "inline-flex items-center gap-2.5 text-[15px] font-semibold tracking-tight text-text-1",
                    span {
                        class: "inline-block w-[3px] h-3.5 rounded-sm bg-iri shadow-[0_0_8px_rgba(45,212,191,0.4)]",
                        "aria-hidden": "true",
                    }
                    "{title}"
                }
                div { class: "flex items-center gap-2 flex-wrap",
                    DimensionTabs {
                        tabs,
                        active,
                        on_select,
                    }
                    if let Some(href) = csv_href {
                        a {
                            class: "csv-btn inline-flex items-center px-[9px] py-0.5 rounded-md border border-border-2 text-[11px] font-semibold tracking-[0.04em] text-muted-1 no-underline hover:text-text-1 hover:border-border-3",
                            href: "{href}",
                            "CSV"
                        }
                    }
                }
            }
            {children}
        }
    }
}
