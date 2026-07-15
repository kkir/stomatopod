use dioxus::prelude::*;

use crate::ui::api::{delete, get_json, post_json};
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::funnel::FunnelBuilder;
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::tabs::{RangeTabs, SiteTab, SiteTabs};
use crate::ui::pages::{active_filters, confirm_delete, use_site_name, BTN_GHOST};
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
use crate::ui::types::{CreateFunnelBody, FunnelsList};

/// Per-site funnels list + builder. Ports funnels.jinja's list and its
/// vanilla-JS create form (now [`FunnelBuilder`]).
#[component]
pub fn Funnels(site_id: String, q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let site_name = use_site_name(site_id.clone());
    let range = q.range.clone().unwrap_or_else(|| "30d".to_string());

    let refresh = use_signal(|| 0u32);
    let funnels = use_resource({
        let site_id = site_id.clone();
        move || {
            let _ = refresh();
            let path = format!("/api/v1/sites/{site_id}/funnels");
            async move { get_json::<FunnelsList>(&path).await }
        }
    });

    rsx! {
        PageHead { title: "Funnels", subtitle: "{site_name}",
            RangeTabs { active: range.clone() }
        }
        SiteTabs { site_id: site_id.clone(), range: range.clone(), active: SiteTab::Funnels }
        {active_filters(&route, &q)}

        div { class: "mb-4",
            Card { title: "Funnels",
                {match &*funnels.read() {
                    None => rsx! {
                        Skeleton { lines: 2 }
                    },
                    Some(Err(e)) => rsx! {
                        EmptyState { message: format!("Failed to load funnels ({e})") }
                    },
                    Some(Ok(list)) => {
                        if list.funnels.is_empty() {
                            rsx! {
                                EmptyState {
                                    title: "Map your conversion funnel",
                                    message: "A funnel tracks how visitors move through a sequence of steps - for example landing \u{2192} signup \u{2192} purchase - so you can see exactly where they drop off. Define your first one below.",
                                }
                            }
                        } else {
                            let site_id = site_id.clone();
                            let range = range.clone();
                            rsx! {
                                div { class: "flex flex-col gap-2",
                                    for f in list.funnels.clone() {
                                        div {
                                            key: "{f.id}",
                                            class: "flex items-center justify-between gap-3 py-2.5 px-1 border-t border-border-1",
                                            Link {
                                                class: "flex-1 min-w-0 text-text-1 no-underline hover:text-teal-hi",
                                                to: Route::FunnelDetail {
                                                    site_id: site_id.clone(),
                                                    funnel_id: f.id.clone(),
                                                    q: DashQuery {
                                                        range: Some(range.clone()),
                                                        ..Default::default()
                                                    },
                                                },
                                                span { class: "text-[13px] font-medium", "{f.name}" }
                                                span { class: "ml-2 text-muted-1 text-xs", "View →" }
                                            }
                                            button {
                                                r#type: "button",
                                                class: BTN_GHOST,
                                                "aria-label": "Delete funnel {f.name}",
                                                onclick: {
                                                    let site_id = site_id.clone();
                                                    let funnel_id = f.id.clone();
                                                    let mut refresh = refresh;
                                                    move |_| {
                                                        if !confirm_delete("Delete this funnel?") {
                                                            return;
                                                        }
                                                        let site_id = site_id.clone();
                                                        let funnel_id = funnel_id.clone();
                                                        spawn(async move {
                                                            let path = format!(
                                                                "/api/v1/sites/{site_id}/funnels/{funnel_id}"
                                                            );
                                                            if delete(&path).await.is_ok() {
                                                                refresh += 1;
                                                            }
                                                        });
                                                    }
                                                },
                                                "Delete"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }}
            }
        }

        Card { title: "Create funnel",
            FunnelBuilder {
                on_submit: {
                    let site_id = site_id.clone();
                    move |(name, steps)| {
                        let site_id = site_id.clone();
                        let mut refresh = refresh;
                        spawn(async move {
                            let path = format!("/api/v1/sites/{site_id}/funnels");
                            let body = CreateFunnelBody { name, steps };
                            if post_json::<_, serde_json::Value>(&path, &body).await.is_ok() {
                                refresh += 1;
                            }
                        });
                    }
                },
            }
        }
    }
}
