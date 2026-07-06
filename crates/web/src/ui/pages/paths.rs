use dioxus::prelude::*;

use crate::ui::api::get_json;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::tabs::RangeTabs;
use crate::ui::pages::{active_filters, SiteScopeSelect};
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
use crate::ui::types::{PathReport, SiteSummary, SitesList};

/// The top navigation sequences for one selected site + range. Split from
/// [`Paths`] so `selected` initializes after the site list loads.
#[component]
fn PathsInner(sites: Vec<SiteSummary>, range: String) -> Element {
    let first = sites.first().map(|s| s.id.clone()).unwrap_or_default();
    let mut selected = use_signal(|| first.clone());

    let data = use_resource({
        let range = range.clone();
        move || {
            let site = selected();
            let range = range.clone();
            let path = format!("/api/v1/sites/{site}/paths?range={range}");
            async move { get_json::<PathReport>(&path).await }
        }
    });

    rsx! {
        SiteScopeSelect {
            sites: sites.clone(),
            selected: selected(),
            on_select: move |v| selected.set(v),
        }
        Card { title: "Top paths",
            {match &*data.read() {
                None => rsx! {
                    Skeleton { lines: 4 }
                },
                Some(Err(e)) => rsx! {
                    EmptyState { message: format!("Failed to load paths ({e})") }
                },
                Some(Ok(report)) => {
                    if report.rows.is_empty() {
                        rsx! {
                            EmptyState { message: "No multi-step sessions yet" }
                        }
                    } else {
                        rsx! {
                            table { class: "w-full border-collapse tabular-nums",
                                thead {
                                    tr {
                                        th { class: "text-muted-1 font-semibold text-left px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                            "Path"
                                        }
                                        th { class: "text-muted-1 font-semibold text-right px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                            "Sessions"
                                        }
                                        th { class: "text-muted-1 font-semibold text-right px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                            "%"
                                        }
                                    }
                                }
                                tbody {
                                    for (i , row) in report.rows.iter().enumerate() {
                                        tr { key: "{i}",
                                            td { class: "px-2.5 py-[11px] border-t border-border-1 text-text-2",
                                                div { class: "flex flex-wrap items-center gap-1.5",
                                                    for (j , step) in row.steps.iter().enumerate() {
                                                        span { key: "{j}", class: "inline-flex items-center gap-1.5",
                                                            if j > 0 {
                                                                span { class: "text-muted-2", "→" }
                                                            }
                                                            span { class: "max-w-[220px] overflow-hidden text-ellipsis whitespace-nowrap inline-block align-bottom",
                                                                "{step}"
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            td { class: "px-2.5 py-[11px] border-t border-border-1 text-text-2 text-right", "{row.sessions}" }
                                            td { class: "px-2.5 py-[11px] border-t border-border-1 text-muted-1 text-right", "{row.pct:.1}%" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }}
        }
    }
}

/// Global navigation-paths page.
#[component]
pub fn Paths(q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let range = q.range.clone().unwrap_or_else(|| "30d".to_string());
    let sites = use_resource(move || async move { get_json::<SitesList>("/api/v1/sites").await });

    rsx! {
        PageHead { title: "Paths", subtitle: "Common navigation sequences.",
            RangeTabs { active: range.clone() }
        }
        {active_filters(&route, &q)}
        {match &*sites.read() {
            None => rsx! {
                Skeleton { lines: 3 }
            },
            Some(Err(e)) => rsx! {
                Card { EmptyState { message: format!("Failed to load sites ({e})") } }
            },
            Some(Ok(list)) => {
                if list.sites.is_empty() {
                    rsx! {
                        Card { EmptyState { message: "No sites yet" } }
                    }
                } else {
                    rsx! {
                        PathsInner { sites: list.sites.clone(), range: range.clone() }
                    }
                }
            }
        }}
    }
}
