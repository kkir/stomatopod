use dioxus::prelude::*;

use crate::ui::query::DashQuery;
use crate::ui::routes::Route;

/// App grid: fixed sidebar + scrolling main. Used as `#[layout(Shell)]`.
#[component]
pub fn Shell() -> Element {
    rsx! {
        div { class: "grid grid-cols-[232px_minmax(0,1fr)] min-h-screen bg-bg font-ui text-text-1",
            Sidebar {}
            main { class: "main min-w-0 overflow-y-auto",
                div { class: "container mx-auto max-w-[1200px] px-f4 pt-8 pb-16",
                    Outlet::<Route> {}
                }
            }
        }
    }
}

/// Sidebar nav, ported from crates/web/templates/base.jinja lines 20-44.
/// Active-link highlighting is longest-prefix match on the current
/// route's path, replacing the inline JS in base.jinja.
#[component]
pub fn Sidebar() -> Element {
    let current_route = use_route::<Route>();
    let current_path = current_route.to_string();

    let nav_items: [(Route, &str); 10] = [
        (Route::SitesIndex {}, "Sites"),
        (Route::GlobalRealtime { site: None }, "Real-time"),
        (Route::GlobalGoals { site: None }, "Goals"),
        (
            Route::Campaigns {
                q: DashQuery::default(),
            },
            "Campaigns",
        ),
        (
            Route::Retention {
                q: DashQuery::default(),
            },
            "Retention",
        ),
        (
            Route::Paths {
                q: DashQuery::default(),
            },
            "Paths",
        ),
        (
            Route::Compare {
                q: DashQuery::default(),
            },
            "Compare",
        ),
        (Route::GlobalAlerts {}, "Alerts"),
        (Route::Keys {}, "API Keys"),
        (Route::Docs {}, "Docs"),
    ];

    let active_href = nav_items
        .iter()
        .map(|(route, _)| route.to_string())
        .filter(|href| current_path.starts_with(href.as_str()))
        .max_by_key(|href| href.len());

    rsx! {
        aside { class: "side sticky top-0 h-dvh flex flex-col px-3.5 pt-5.5 pb-4 bg-bg-2/65 backdrop-blur-lg border-r border-border-1 z-30",
            Link {
                class: "logo inline-flex items-center gap-2.5 px-2.5 font-display font-bold text-base tracking-tight text-text-1",
                to: Route::SitesIndex {},
                span { class: "logo-mark bg-iri inline-block h-3.25 w-3.25 rounded-sm", "aria-hidden": "true" }
                "Stomatopod"
            }
            nav { class: "side-nav flex flex-col gap-0.5 mt-7",
                span { class: "side-label text-[10.5px] font-semibold uppercase tracking-wider text-muted-2 mx-3 mt-4 mb-1",
                    "Analytics"
                }
                for (route , label) in nav_items {
                    {
                    let href = route.to_string();
                    let is_active = active_href.as_deref() == Some(href.as_str());
                    rsx! {
                        Link {
                            key: "{label}",
                            "aria-current": if is_active { "page" },
                            class: if is_active {
                                "relative px-3 py-2 rounded-[10px] text-[13.5px] font-medium text-text-1 bg-teal-soft before:content-[''] before:absolute before:left-0 before:top-2 before:bottom-2 before:w-[3px] before:rounded-full before:bg-iri before:shadow-[0_0_8px_rgba(45,212,191,0.5)]"
                            } else {
                                "px-3 py-2 rounded-[10px] text-[13.5px] font-medium text-muted-1 hover:text-text-1 hover:bg-text-1/4"
                            },
                            to: route,
                            "{label}"
                        }
                    }
                    }
                }
            }
            form { class: "side-foot mt-auto grid", method: "post", action: "/logout",
                button {
                    class: "btn btn-ghost justify-center inline-flex items-center gap-1.5 px-4 py-2 rounded-[10px] border border-border-2 bg-text-1/3 text-text-2 shadow-inner-hi",
                    r#type: "submit",
                    "Logout"
                }
            }
        }
    }
}

/// Page title + optional subtitle, with a right-hand slot for
/// `RangeTabs`/`SiteTabs`/a site selector. Port of `.page-head`.
#[component]
pub fn PageHead(title: String, subtitle: Option<String>, children: Element) -> Element {
    rsx! {
        div { class: "flex justify-between items-center gap-4 mb-7 flex-wrap",
            div {
                h1 { class: "text-[22px] font-bold tracking-tight text-text-1", "{title}" }
                if let Some(sub) = subtitle {
                    p { class: "text-muted-1 text-[12.5px] mt-0.5", "{sub}" }
                }
            }
            div { class: "flex items-center gap-2.5", {children} }
        }
    }
}
