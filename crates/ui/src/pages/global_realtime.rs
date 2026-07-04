use dioxus::prelude::*;

use super::realtime::RealtimePanel;
use crate::api::get_json;
use crate::components::card::{Card, EmptyState};
use crate::components::layout::PageHead;
use crate::components::skeleton::Skeleton;
use crate::pages::SiteScopeSelect;
use crate::routes::Route;
use crate::types::SitesList;

/// Global real-time page: a site selector (navigates by swapping the `site`
/// query param) plus the same live [`RealtimePanel`] as the per-site page.
#[component]
pub fn GlobalRealtime(site: Option<String>) -> Element {
    let sites = use_resource(move || async move { get_json::<SitesList>("/api/v1/sites").await });

    rsx! {
        PageHead { title: "Real-time", subtitle: "Live activity across your sites." }
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
                    let selected = site
                        .clone()
                        .filter(|s| list.sites.iter().any(|x| &x.id == s))
                        .unwrap_or_else(|| list.sites[0].id.clone());
                    rsx! {
                        // Keyed wrapper: changing the site remounts the subtree
                        // so the panel re-fetches (props alone wouldn't re-run it).
                        div { key: "{selected}",
                            SiteScopeSelect {
                                sites: list.sites.clone(),
                                selected: selected.clone(),
                                on_select: move |v| {
                                    navigator().push(Route::GlobalRealtime { site: Some(v) });
                                },
                            }
                            RealtimePanel { site_id: selected }
                        }
                    }
                }
            }
        }}
    }
}
