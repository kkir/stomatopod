use dioxus::prelude::*;

use crate::ui::api::get_json;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::tabs::{RangeTabs, SiteTab, SiteTabs};
use crate::ui::pages::{active_filters, use_site_name};
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
use crate::ui::types::PathReport;

/// Per-site navigation-paths page.
#[component]
pub fn Paths(site_id: String, q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let range = q.range.clone().unwrap_or_else(|| "30d".to_string());
    let name = use_site_name(site_id.clone());

    let data = use_resource({
        let site_id = site_id.clone();
        let range = range.clone();
        move || {
            let path = format!("/api/v1/sites/{site_id}/paths?range={range}");
            async move { get_json::<PathReport>(&path).await }
        }
    });

    rsx! {
        PageHead {
            title: name,
            subtitle: "Common navigation sequences.".to_string(),
            RangeTabs { active: range.clone() }
        }
        SiteTabs { site_id: site_id.clone(), range: range.clone(), active: SiteTab::Paths }
        {active_filters(&route, &q)}
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
