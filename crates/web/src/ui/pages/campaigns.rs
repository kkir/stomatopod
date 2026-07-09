use dioxus::prelude::*;

use crate::ui::api::get_json;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::table::{BreakdownRow, BreakdownTable};
use crate::ui::components::tabs::{RangeTabs, SiteTab, SiteTabs};
use crate::ui::pages::{active_filters, use_site_name};
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
use crate::ui::types::{Campaigns as CampaignsData, TopList};

fn rows_of(list: &TopList) -> Vec<BreakdownRow> {
    list.rows
        .iter()
        .map(|r| BreakdownRow {
            value: r.value.clone(),
            count: r.sessions,
            pct: r.pct,
            spark: None,
        })
        .collect()
}

/// Per-site campaigns (UTM breakdown) page.
#[component]
pub fn Campaigns(site_id: String, q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let range = q.range.clone().unwrap_or_else(|| "30d".to_string());
    let name = use_site_name(site_id.clone());

    let data = use_resource({
        let site_id = site_id.clone();
        let range = range.clone();
        move || {
            let path = format!("/api/v1/sites/{site_id}/campaigns?range={range}");
            async move { get_json::<CampaignsData>(&path).await }
        }
    });

    rsx! {
        PageHead {
            title: name,
            subtitle: "UTM source, medium, and campaign breakdowns.".to_string(),
            RangeTabs { active: range.clone() }
        }
        SiteTabs { site_id: site_id.clone(), range: range.clone(), active: SiteTab::Campaigns }
        {active_filters(&route, &q)}
        {match &*data.read() {
            None => rsx! {
                Skeleton { lines: 4 }
            },
            Some(Err(e)) => rsx! {
                Card { EmptyState { message: format!("Failed to load campaigns ({e})") } }
            },
            Some(Ok(c)) => rsx! {
                div { class: "grid grid-cols-1 md:grid-cols-2 gap-4",
                    BreakdownTable {
                        title: "UTM Source",
                        value_header: "Source",
                        count_header: "Visitors",
                        rows: rows_of(&c.utm_source),
                        csv_href: None,
                        on_filter: None,
                    }
                    BreakdownTable {
                        title: "UTM Medium",
                        value_header: "Medium",
                        count_header: "Visitors",
                        rows: rows_of(&c.utm_medium),
                        csv_href: None,
                        on_filter: None,
                    }
                    BreakdownTable {
                        title: "UTM Campaign",
                        value_header: "Campaign",
                        count_header: "Visitors",
                        rows: rows_of(&c.utm_campaign),
                        csv_href: None,
                        on_filter: None,
                    }
                    BreakdownTable {
                        title: "UTM Term",
                        value_header: "Term",
                        count_header: "Visitors",
                        rows: rows_of(&c.utm_term),
                        csv_href: None,
                        on_filter: None,
                    }
                    BreakdownTable {
                        title: "UTM Content",
                        value_header: "Content",
                        count_header: "Visitors",
                        rows: rows_of(&c.utm_content),
                        csv_href: None,
                        on_filter: None,
                    }
                }
            },
        }}
    }
}
