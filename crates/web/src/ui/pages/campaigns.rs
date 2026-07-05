use dioxus::prelude::*;

use crate::ui::api::get_json;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::table::{BreakdownRow, BreakdownTable};
use crate::ui::components::tabs::RangeTabs;
use crate::ui::pages::{active_filters, SiteScopeSelect};
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
use crate::ui::types::{Campaigns as CampaignsData, SiteSummary, SitesList, TopList};

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

/// The five UTM breakdown tables for one site + range. Split out from
/// [`Campaigns`] so `selected` initializes to the first site only after the
/// site list has loaded.
#[component]
fn CampaignsInner(sites: Vec<SiteSummary>, range: String) -> Element {
    let site_list = sites;
    let first = site_list.first().map(|s| s.id.clone()).unwrap_or_default();
    let mut selected = use_signal(|| first.clone());

    let data = use_resource({
        let range = range.clone();
        move || {
            let site = selected();
            let range = range.clone();
            let path = format!("/api/v1/sites/{site}/campaigns?range={range}");
            async move { get_json::<CampaignsData>(&path).await }
        }
    });

    rsx! {
        SiteScopeSelect {
            sites: site_list.clone(),
            selected: selected(),
            on_select: move |v| selected.set(v),
        }
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

/// Global campaigns (UTM breakdown) page.
#[component]
pub fn Campaigns(q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let range = q.range.clone().unwrap_or_else(|| "30d".to_string());
    let sites = use_resource(move || async move { get_json::<SitesList>("/api/v1/sites").await });

    rsx! {
        PageHead { title: "Campaigns", subtitle: "UTM source, medium, and campaign breakdowns.",
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
                        CampaignsInner { sites: list.sites.clone(), range: range.clone() }
                    }
                }
            }
        }}
    }
}
