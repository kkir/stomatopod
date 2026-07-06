use dioxus::prelude::*;

use super::alerts::AlertsManager;
use crate::ui::api::get_json;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::pages::SiteScopeSelect;
use crate::ui::types::{SiteSummary, SitesList};

/// Alerts manager scoped to a site chosen from an internal selector. Split
/// from [`GlobalAlerts`] so `selected` initializes after the site list
/// loads. `GlobalAlerts` has no `site` route param (unlike realtime/goals),
/// so selection is local state rather than a navigation.
#[component]
fn GlobalAlertsInner(sites: Vec<SiteSummary>) -> Element {
    let first = sites.first().map(|s| s.id.clone()).unwrap_or_default();
    let mut selected = use_signal(|| first.clone());

    rsx! {
        // Keyed wrapper: changing the site remounts the subtree so the
        // manager re-fetches (props alone wouldn't re-run its resources).
        div { key: "{selected}",
            SiteScopeSelect {
                sites: sites.clone(),
                selected: selected(),
                on_select: move |v| selected.set(v),
            }
            AlertsManager { site_id: selected() }
        }
    }
}

/// Global analytics-alerts page.
#[component]
pub fn GlobalAlerts() -> Element {
    let sites = use_resource(move || async move { get_json::<SitesList>("/api/v1/sites").await });

    rsx! {
        PageHead { title: "Alerts", subtitle: "Analytics alerts across all sites." }
        {match &*sites.read() {
            None => rsx! {
                Skeleton { lines: 4 }
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
                        GlobalAlertsInner { sites: list.sites.clone() }
                    }
                }
            }
        }}
    }
}
