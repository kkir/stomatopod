use dioxus::prelude::*;

use crate::ui::api;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::refresh::AutoRefresh;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::table::{BreakdownRow, BreakdownTable};
use crate::ui::components::tabs::{RangeTabs, SiteTab, SiteTabs};
use crate::ui::pages::{active_filters, site_api_url, site_csv_url, use_site_name, BTN_PRIMARY};
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
use crate::ui::types::TopList;

/// Port of the per-site custom-events page (events.jinja). The handler
/// (`analytics::events`, `GET /api/v1/sites/:site/events`) returns the
/// same `TopList` shape as the top-N breakdowns, so this reuses
/// `BreakdownTable`. events.jinja shows both a pageview total and a
/// session count column; `BreakdownTable` only has one count column, so
/// the pageview total (its primary "Total" column) is shown and the
/// per-row session count is dropped.
#[component]
pub fn Events(site_id: String, q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let site_name = use_site_name(site_id.clone());
    let range = q.range.clone().unwrap_or_else(|| "30d".to_string());
    let qs = q.to_string();
    let refresh_tick = use_signal(|| 0u32);
    let tick = refresh_tick();

    let csv_href = Some(site_csv_url(&site_id, "events", &qs));

    let events = use_resource(use_reactive!(|site_id, qs, tick| async move {
        let _ = tick;
        let path = site_api_url(&site_id, "events", &qs);
        api::get_json::<TopList>(&path).await
    }));

    let body = match &*events.read() {
        None => rsx! { Skeleton { lines: 4 } },
        Some(Err(e)) => rsx! {
            Card { EmptyState { message: e.to_string() } }
        },
        Some(Ok(list)) if list.rows.is_empty() => rsx! {
            Card {
                EmptyState {
                    title: "No custom events yet",
                    message: "Custom events track the actions that matter - signups, purchases, clicks. Fire them from the browser with stomatopod(\"event\", \"signup\") or POST to the ingest API, and they'll show up here.",
                    Link { class: "{BTN_PRIMARY} mt-6", to: Route::Docs {}, "Learn how to send events" }
                }
            }
        },
        Some(Ok(list)) => {
            let rows = list
                .rows
                .iter()
                .map(|row| BreakdownRow {
                    value: row.value.clone(),
                    count: row.pageviews,
                    pct: row.pct,
                    spark: None,
                })
                .collect::<Vec<_>>();
            rsx! {
                BreakdownTable {
                    title: "Events",
                    value_header: "Event",
                    count_header: "Count",
                    rows,
                    csv_href,
                    on_filter: None,
                }
            }
        }
    };

    rsx! {
        PageHead { title: "Events", subtitle: "{site_name}",
            div { class: "flex items-center gap-2 flex-wrap",
                RangeTabs { active: range.clone() }
                AutoRefresh { tick: refresh_tick }
            }
        }
        SiteTabs { site_id: site_id.clone(), range, active: SiteTab::Events }
        {active_filters(&route, &q)}
        {body}
    }
}
