use dioxus::prelude::*;

use crate::ui::api;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::table::{BreakdownRow, BreakdownTable};
use crate::ui::components::tabs::{RangeTabs, SiteTab, SiteTabs};
use crate::ui::pages::{active_filters, site_api_url, site_csv_url};
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
    let range = q.range.clone().unwrap_or_else(|| "30d".to_string());
    let qs = q.to_string();

    let csv_href = Some(site_csv_url(&site_id, "events", &qs));

    let site_id_for_resource = site_id.clone();
    let events = use_resource(move || {
        let path = site_api_url(&site_id_for_resource, "events", &qs);
        async move { api::get_json::<TopList>(&path).await }
    });

    let body = match &*events.read() {
        None => rsx! { Skeleton { lines: 4 } },
        Some(Err(e)) => rsx! {
            Card { EmptyState { message: e.to_string() } }
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
        PageHead { title: "Events", subtitle: "Site {site_id}",
            RangeTabs { active: range.clone() }
        }
        SiteTabs { site_id: site_id.clone(), range, active: SiteTab::Events }
        {active_filters(&route, &q)}
        {body}
    }
}
