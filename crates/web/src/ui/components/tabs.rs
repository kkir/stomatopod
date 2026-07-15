use dioxus::prelude::*;

use crate::ui::query::DashQuery;
use crate::ui::routes::Route;

const RANGES: [&str; 4] = ["7d", "30d", "90d", "12m"];
/// Compact date input matching range-tab height (mirrors page-head toolbar).
const DATE_INPUT: &str = "h-9 inline-flex items-center bg-surface-2/80 border border-border-1 text-text-1 rounded-[11px] px-2 text-[12px] font-semibold shadow-inner-hi focus:outline-none focus:border-teal/55 cursor-pointer";

/// The 7d/30d/90d/12m pills plus optional custom from/to dates. Navigates by
/// swapping range params on the current route while preserving filters/compare.
#[component]
pub fn RangeTabs(active: String) -> Element {
    let route = use_route::<Route>();
    let current = route.query().cloned().unwrap_or_default();
    let custom = current.is_custom_range();
    // When a custom window is active, no preset pill is highlighted.
    let active_preset = if custom { String::new() } else { active };
    let mut from_val = use_signal(|| current.from.clone().unwrap_or_default());
    let mut to_val = use_signal(|| current.to.clone().unwrap_or_default());
    // Keep the date inputs aligned with the route (e.g. clearing after a preset click).
    let route_from = current.from.clone().unwrap_or_default();
    let route_to = current.to.clone().unwrap_or_default();
    use_effect(use_reactive!(|(route_from, route_to)| {
        from_val.set(route_from);
        to_val.set(route_to);
    }));

    rsx! {
        div { class: "inline-flex flex-wrap items-center gap-1.5 shrink-0",
            // Navigation links (not in-page tabs): use a labelled group +
            // aria-current, not role=tablist.
            nav {
                class: "inline-flex items-center h-9 gap-0.5 p-[3px] rounded-[11px] bg-surface-2/80 border border-border-1 shadow-inner-hi shrink-0",
                "aria-label": "Date range",
                for range in RANGES {
                    {
                        let is_active = active_preset == range;
                        let range_label = match range {
                            "7d" => "Last 7 days",
                            "30d" => "Last 30 days",
                            "90d" => "Last 90 days",
                            "12m" => "Last 12 months",
                            _ => range,
                        };
                        rsx! {
                            Link {
                                key: "{range}",
                                to: route.with_query(current.with_range(range)),
                                "aria-label": "{range_label}",
                                "aria-current": if is_active { "true" },
                                class: if is_active {
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
            form {
                class: "inline-flex items-center gap-1 h-9",
                onsubmit: {
                    let route = route.clone();
                    let current = current.clone();
                    move |evt: FormEvent| {
                        evt.prevent_default();
                        let from = from_val().trim().to_string();
                        let to = to_val().trim().to_string();
                        if !from.is_empty() && !to.is_empty() {
                            navigator().push(route.with_query(current.with_custom_range(&from, &to)));
                        }
                    }
                },
                input {
                    class: "{DATE_INPUT} w-[8.25rem] max-w-[38vw]",
                    r#type: "date",
                    value: "{from_val}",
                    "aria-label": "From date",
                    oninput: move |e| from_val.set(e.value()),
                }
                span { class: "text-muted-2 text-[11px] font-semibold", "–" }
                input {
                    class: "{DATE_INPUT} w-[8.25rem] max-w-[38vw]",
                    r#type: "date",
                    value: "{to_val}",
                    "aria-label": "To date",
                    oninput: move |e| to_val.set(e.value()),
                }
                button {
                    r#type: "submit",
                    class: if custom {
                        "h-9 inline-flex items-center px-3 rounded-[11px] text-[12px] font-semibold bg-teal-soft text-teal-hi border border-teal/35 cursor-pointer shrink-0"
                    } else {
                        "h-9 inline-flex items-center px-3 rounded-[11px] text-[12px] font-semibold text-muted-1 hover:text-text-1 bg-surface-2/80 border border-border-1 shadow-inner-hi cursor-pointer shrink-0"
                    },
                    "aria-label": "Apply custom date range",
                    "Go"
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
    let q = if current.range.is_some()
        || current.is_custom_range()
        || !current.filters.is_empty()
        || current.compare
    {
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
                        "aria-current": if tab == active { "page" },
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
///
/// Uses the ARIA tabs pattern (`tablist` / `tab` / `aria-selected`). All tabs
/// stay in the tab order (small sets of 2-3) so keyboard users can Tab between
/// them without roving-tabindex focus management.
#[component]
pub fn DimensionTabs(
    tabs: Vec<String>,
    active: usize,
    on_select: EventHandler<usize>,
    #[props(default)] aria_label: Option<String>,
) -> Element {
    let list_label = aria_label.unwrap_or_else(|| "Dimensions".to_string());
    rsx! {
        div {
            class: "inline-flex flex-wrap gap-0.5 p-[3px] rounded-[11px] bg-surface-2/80 border border-border-1 shadow-inner-hi",
            role: "tablist",
            "aria-label": "{list_label}",
            for (i, label) in tabs.into_iter().enumerate() {
                {
                    let selected = i == active;
                    rsx! {
                        button {
                            key: "{label}",
                            r#type: "button",
                            role: "tab",
                            "aria-selected": if selected { "true" } else { "false" },
                            class: if selected {
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
    let dim_label = format!("{title} dimensions");
    let csv_label = format!("Export {title} CSV");
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
                        aria_label: dim_label,
                    }
                    if let Some(href) = csv_href {
                        a {
                            class: "csv-btn inline-flex items-center px-[9px] py-0.5 rounded-md border border-border-2 text-[11px] font-semibold tracking-[0.04em] text-muted-1 no-underline hover:text-text-1 hover:border-border-3",
                            href: "{href}",
                            "aria-label": "{csv_label}",
                            "CSV"
                        }
                    }
                }
            }
            div {
                role: "tabpanel",
                "aria-label": "{title}",
                {children}
            }
        }
    }
}
