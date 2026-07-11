use dioxus::prelude::*;

use crate::ui::api::get_json;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::refresh::AutoRefresh;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::table::{BreakdownRow, BreakdownTable};
use crate::ui::components::tabs::{RangeTabs, SiteTab, SiteTabs, TabbedCard};
use crate::ui::pages::{active_filters, site_api_url, use_site_name};
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

/// Per-site campaigns (UTM breakdown) page with progressive dimension tabs.
#[component]
pub fn Campaigns(site_id: String, q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let range = q.range.clone().unwrap_or_else(|| "30d".to_string());
    let name = use_site_name(site_id.clone());
    let mut tab = use_signal(|| 0usize);
    let refresh_tick = use_signal(|| 0u32);
    let tick = refresh_tick();

    let qs = q.to_string();
    let data = use_resource(use_reactive!(|site_id, qs, tick| async move {
        let _ = tick;
        let path = site_api_url(&site_id, "campaigns", &qs);
        get_json::<CampaignsData>(&path).await
    }));

    rsx! {
        PageHead {
            title: name,
            subtitle: "UTM source, medium, and campaign breakdowns.".to_string(),
            RangeTabs { active: range.clone() }
            AutoRefresh { tick: refresh_tick }
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
            Some(Ok(c)) => {
                let dims: [(&str, &str, &str, &TopList); 5] = [
                    ("Source", "utm_source", "Source", &c.utm_source),
                    ("Medium", "utm_medium", "Medium", &c.utm_medium),
                    ("Campaign", "utm_campaign", "Campaign", &c.utm_campaign),
                    ("Term", "utm_term", "Term", &c.utm_term),
                    ("Content", "utm_content", "Content", &c.utm_content),
                ];
                let active = tab().min(dims.len() - 1);
                let (label, field, value_header, list) = dims[active];
                let rows = rows_of(list);
                let route = route.clone();
                let q = q.clone();
                let field = field.to_string();
                rsx! {
                    TabbedCard {
                        title: "Campaigns",
                        tabs: dims.iter().map(|(l, ..)| (*l).to_string()).collect(),
                        active,
                        on_select: move |i| tab.set(i),
                        csv_href: None,
                        BreakdownTable {
                            title: format!("UTM {label}"),
                            value_header: value_header.to_string(),
                            count_header: "Visitors".to_string(),
                            rows,
                            csv_href: None,
                            framed: false,
                            empty_title: format!("No UTM {label} data yet"),
                            empty_message: "UTM parameters appear when visitors arrive with tracking tags on the URL.".to_string(),
                            on_filter: Some(EventHandler::new(move |value: String| {
                                let mut nq = q.clone();
                                nq.filters.push(format!("{field}:eq:{value}"));
                                navigator().push(route.with_query(nq));
                            })),
                        }
                    }
                }
            },
        }}
    }
}
