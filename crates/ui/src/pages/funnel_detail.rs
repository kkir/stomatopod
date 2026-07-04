use dioxus::prelude::*;

use crate::api::get_json;
use crate::components::card::{Card, EmptyState};
use crate::components::funnel::FunnelBars;
use crate::components::layout::PageHead;
use crate::components::skeleton::Skeleton;
use crate::components::tabs::{RangeTabs, SiteTab, SiteTabs};
use crate::pages::{active_filters, BTN_GHOST};
use crate::query::DashQuery;
use crate::routes::Route;
use crate::types::FunnelResult;

/// Funnel-detail page: runs the funnel for the current range and draws the
/// step-conversion bar chart ([`FunnelBars`]).
#[component]
pub fn FunnelDetail(site_id: String, funnel_id: String, q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let range = q.range.clone().unwrap_or_else(|| "30d".to_string());

    let result = use_resource({
        let site_id = site_id.clone();
        let funnel_id = funnel_id.clone();
        let range = range.clone();
        move || {
            let path = format!("/api/v1/sites/{site_id}/funnels/{funnel_id}?range={range}");
            async move { get_json::<FunnelResult>(&path).await }
        }
    });

    rsx! {
        PageHead { title: "Funnel", subtitle: "Site {site_id}",
            RangeTabs { active: range.clone() }
        }
        SiteTabs { site_id: site_id.clone(), range: range.clone(), active: SiteTab::Funnels }
        {active_filters(&route, &q)}

        Card {
            {match &*result.read() {
                None => rsx! {
                    Skeleton { lines: 4 }
                },
                Some(Err(e)) => rsx! {
                    EmptyState { message: format!("Failed to load funnel ({e})") }
                },
                Some(Ok(res)) => {
                    if res.steps.is_empty() {
                        rsx! {
                            EmptyState { message: "No data for this funnel" }
                        }
                    } else {
                        rsx! {
                            FunnelBars { steps: res.steps.clone() }
                        }
                    }
                }
            }}
        }

        div { class: "mt-4",
            Link {
                to: Route::Funnels {
                    site_id: site_id.clone(),
                    q: DashQuery {
                        range: Some(range.clone()),
                        ..Default::default()
                    },
                },
                class: BTN_GHOST,
                "← All funnels"
            }
        }
    }
}
