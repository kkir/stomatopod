use dioxus::prelude::*;

use crate::api::get_json;
use crate::components::card::{Card, EmptyState};
use crate::components::layout::PageHead;
use crate::components::skeleton::Skeleton;
use crate::components::stat::StatTile;
use crate::components::tabs::{SiteTab, SiteTabs};

/// The live panel shared by the per-site [`Realtime`] page and the global
/// real-time page: active sessions + pageviews/min stat tiles, a top active
/// pages list, and a recent-events feed. Re-fetches every 10s via a
/// `gloo_timers` tick signal (replaces the legacy `hx-trigger="load, every
/// 10s"` on partials/realtime_panel.jinja).
#[component]
pub fn RealtimePanel(site_id: String) -> Element {
    let mut tick = use_signal(|| 0u32);
    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(10_000).await;
            tick += 1;
        }
    });

    let rt = use_resource({
        let site_id = site_id.clone();
        move || {
            let _ = tick();
            let path = format!("/api/v1/sites/{site_id}/realtime");
            async move { get_json::<crate::types::RealtimeSnapshot>(&path).await }
        }
    });

    rsx! {
        {match &*rt.read() {
            None => rsx! {
                Skeleton { lines: 4 }
            },
            Some(Err(e)) => rsx! {
                Card { EmptyState { message: format!("Failed to load real-time data ({e})") } }
            },
            Some(Ok(s)) => {
                rsx! {
                Card {
                    div { class: "grid grid-cols-2 gap-4",
                        StatTile {
                            label: "Active sessions",
                            value: format!("{}", s.active_sessions),
                        }
                        StatTile {
                            label: "Pageviews / min",
                            value: format!("{:.1}", s.pageviews_per_minute),
                        }
                    }
                }
                div { class: "grid grid-cols-1 md:grid-cols-2 gap-4 mt-4",
                    Card { title: "Active pages",
                        if s.top_pages.is_empty() {
                            EmptyState { message: "No active visitors" }
                        } else {
                            table { class: "w-full border-collapse tabular-nums",
                                tbody {
                                    for p in s.top_pages.clone() {
                                        tr { key: "{p.url}",
                                            td { class: "max-w-[220px] overflow-hidden text-ellipsis whitespace-nowrap px-2.5 py-[11px] border-t border-border-1 text-text-2",
                                                "{p.url}"
                                            }
                                            td { class: "px-2.5 py-[11px] border-t border-border-1 text-text-2 text-right w-16",
                                                "{p.active_sessions}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Card { title: "Recent events",
                        if s.recent_events.is_empty() {
                            EmptyState { message: "No recent events" }
                        } else {
                            table { class: "w-full border-collapse tabular-nums",
                                tbody {
                                    for (i , ev) in s.recent_events.iter().enumerate() {
                                        tr { key: "{i}-{ev.url}",
                                            td { class: "px-2.5 py-[11px] border-t border-border-1 text-text-1 font-medium whitespace-nowrap",
                                                "{ev.name}"
                                            }
                                            td { class: "max-w-[180px] overflow-hidden text-ellipsis whitespace-nowrap px-2.5 py-[11px] border-t border-border-1 text-text-2",
                                                "{ev.url}"
                                            }
                                            td { class: "px-2.5 py-[11px] border-t border-border-1 text-muted-1 text-right whitespace-nowrap",
                                                "{ev.seconds_ago}s ago"
                                            }
                                        }
                                    }
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

/// Per-site real-time page: tab row + live [`RealtimePanel`].
#[component]
pub fn Realtime(site_id: String) -> Element {
    rsx! {
        PageHead { title: "Real-time", subtitle: "Site {site_id}" }
        SiteTabs { site_id: site_id.clone(), range: "30d", active: SiteTab::Realtime }
        RealtimePanel { site_id }
    }
}
