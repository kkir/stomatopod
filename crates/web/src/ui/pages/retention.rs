use dioxus::prelude::*;

use crate::ui::api::get_json;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::heatmap::RetentionHeatmap;
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::tabs::RangeTabs;
use crate::ui::pages::{active_filters, SiteScopeSelect};
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
use crate::ui::types::{RetentionGrid, SiteSummary, SitesList};

/// The retention heatmap for one selected site + range. Split from
/// [`Retention`] so `selected` initializes after the site list loads.
#[component]
fn RetentionInner(sites: Vec<SiteSummary>, range: String) -> Element {
    let first = sites.first().map(|s| s.id.clone()).unwrap_or_default();
    let mut selected = use_signal(|| first.clone());

    let data = use_resource({
        let range = range.clone();
        move || {
            let site = selected();
            let range = range.clone();
            let path = format!("/api/v1/sites/{site}/retention?range={range}");
            async move { get_json::<RetentionGrid>(&path).await }
        }
    });

    rsx! {
        SiteScopeSelect {
            sites: sites.clone(),
            selected: selected(),
            on_select: move |v| selected.set(v),
        }
        Card { title: "Weekly cohorts",
            {match &*data.read() {
                None => rsx! {
                    Skeleton { lines: 4 }
                },
                Some(Err(e)) => rsx! {
                    EmptyState { message: format!("Failed to load retention ({e})") }
                },
                Some(Ok(grid)) => rsx! {
                    RetentionHeatmap { grid: grid.clone() }
                },
            }}
        }
    }
}

/// Global weekly-cohort retention page.
#[component]
pub fn Retention(q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let range = q.range.clone().unwrap_or_else(|| "90d".to_string());
    let sites = use_resource(move || async move { get_json::<SitesList>("/api/v1/sites").await });

    rsx! {
        PageHead { title: "Retention",
            subtitle: "Weekly cohort retention. Cookieless sessions rotate, so cross-week returns are naturally sparse.",
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
                        RetentionInner { sites: list.sites.clone(), range: range.clone() }
                    }
                }
            }
        }}
    }
}
