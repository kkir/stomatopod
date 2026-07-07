use dioxus::prelude::*;

use crate::ui::api::get_json;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::funnel::FunnelBars;
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::tabs::{RangeTabs, SiteTab, SiteTabs};
use crate::ui::pages::{active_filters, use_site_name, BTN_GHOST};
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
use crate::ui::types::FunnelResult;

/// Funnel-detail page: runs the funnel for the current range and draws the
/// step-conversion bar chart ([`FunnelBars`]).
#[component]
pub fn FunnelDetail(site_id: String, funnel_id: String, q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let site_name = use_site_name(site_id.clone());
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
        PageHead { title: "Funnel", subtitle: "{site_name}",
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
