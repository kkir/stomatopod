use dioxus::prelude::*;

use crate::api::{get_json, ApiError};
use crate::components::card::{Card, EmptyState};
use crate::components::layout::PageHead;
use crate::components::skeleton::Skeleton;
use crate::components::tabs::RangeTabs;
use crate::pages::active_filters;
use crate::query::DashQuery;
use crate::routes::Route;
use crate::types::{PageviewsResult, SitesList};

#[derive(Clone, PartialEq)]
struct CompareRow {
    name: String,
    domain: String,
    pageviews: u64,
    sessions: u64,
    bounce_rate: f64,
}

/// Fetch each site's totals for `range`, then sort by pageviews desc,
/// mirroring the legacy `compare_global` handler.
async fn load_compare(range: String) -> Result<Vec<CompareRow>, ApiError> {
    let list = get_json::<SitesList>("/api/v1/sites").await?;
    let mut rows = Vec::with_capacity(list.sites.len());
    for s in list.sites {
        let path = format!("/api/v1/sites/{}/pageviews?range={range}", s.id);
        // A per-site failure shouldn't sink the whole table; treat it as zero.
        let pv = get_json::<PageviewsResult>(&path).await.unwrap_or_default();
        rows.push(CompareRow {
            name: s.name,
            domain: s.domain,
            pageviews: pv.total_pageviews,
            sessions: pv.total_sessions,
            bounce_rate: pv.bounce_rate,
        });
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.pageviews));
    Ok(rows)
}

/// Global cross-site comparison page.
#[component]
pub fn Compare(q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let range = q.range.clone().unwrap_or_else(|| "30d".to_string());
    let data = use_resource({
        let range = range.clone();
        move || {
            let range = range.clone();
            async move { load_compare(range).await }
        }
    });

    rsx! {
        PageHead { title: "Compare", subtitle: "Compare metrics across sites.",
            RangeTabs { active: range.clone() }
        }
        {active_filters(&route, &q)}
        Card { title: "Sites",
            {match &*data.read() {
                None => rsx! {
                    Skeleton { lines: 4 }
                },
                Some(Err(e)) => rsx! {
                    EmptyState { message: format!("Failed to load comparison ({e})") }
                },
                Some(Ok(rows)) => {
                    if rows.is_empty() {
                        rsx! {
                            EmptyState { message: "No sites yet" }
                        }
                    } else {
                        rsx! {
                            table { class: "w-full border-collapse tabular-nums",
                                thead {
                                    tr {
                                        th { class: "text-muted-1 font-semibold text-left px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                            "Site"
                                        }
                                        th { class: "text-muted-1 font-semibold text-right px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                            "Pageviews"
                                        }
                                        th { class: "text-muted-1 font-semibold text-right px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                            "Sessions"
                                        }
                                        th { class: "text-muted-1 font-semibold text-right px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                            "Bounce"
                                        }
                                    }
                                }
                                tbody {
                                    for row in rows.clone() {
                                        tr { key: "{row.domain}",
                                            td { class: "px-2.5 py-[11px] border-t border-border-1",
                                                div { class: "text-text-1 font-medium", "{row.name}" }
                                                div { class: "text-muted-1 text-xs", "{row.domain}" }
                                            }
                                            td { class: "px-2.5 py-[11px] border-t border-border-1 text-text-2 text-right", "{row.pageviews}" }
                                            td { class: "px-2.5 py-[11px] border-t border-border-1 text-text-2 text-right", "{row.sessions}" }
                                            td { class: "px-2.5 py-[11px] border-t border-border-1 text-muted-1 text-right", "{row.bounce_rate:.1}%" }
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
