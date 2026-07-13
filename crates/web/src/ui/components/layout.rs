use dioxus::prelude::*;

use crate::ui::routes::Route;

/// App grid: fixed sidebar + scrolling main on desktop; collapsible top bar
/// on mobile. Used as `#[layout(Shell)]`.
#[component]
pub fn Shell() -> Element {
    // Client-only hydration marker. SSR renders the shell without it; the mount
    // effect below runs only after wasm hydrates (once event handlers are
    // attached), flipping `data-hydrated` on. Tests (and any readiness probe)
    // can wait for it instead of the SSR'd sidebar, which is present before the
    // page is actually interactive.
    let mut hydrated = use_signal(|| false);
    use_effect(move || hydrated.set(true));

    rsx! {
        // `h-dvh` + `min-h-0` on main make main the real scrollport. Without
        // a fixed-height ancestor, `overflow-y-auto` never clips and sticky
        // sub-navs (docs TOC) fail to stick because they sit inside a tall
        // overflow box that does not scroll.
        //
        // Mobile (< md): single column, auto-height top bar + main fills the
        // rest. Desktop: 232px sidebar + fluid main.
        div {
            class: "grid grid-cols-1 [grid-template-rows:auto_minmax(0,1fr)] md:grid-cols-[232px_minmax(0,1fr)] md:[grid-template-rows:minmax(0,1fr)] h-dvh bg-bg font-ui text-text-1",
            "data-hydrated": if hydrated() { "true" },
            Sidebar {}
            main { class: "main min-w-0 min-h-0 overflow-y-auto overflow-x-hidden",
                div { class: "container mx-auto max-w-[1200px] px-3 sm:px-f4 pt-5 sm:pt-8 pb-12 sm:pb-16",
                    Outlet::<Route> {}
                }
            }
        }
    }
}

/// Sidebar nav. Active-link highlighting is longest-prefix match on the
/// current route's path. On small screens this becomes a horizontal top bar.
#[component]
pub fn Sidebar() -> Element {
    let current_route = use_route::<Route>();
    let current_path = current_route.to_string();

    let nav_items: [(Route, &str); 3] = [
        (Route::SitesIndex {}, "Sites"),
        (Route::Keys {}, "API Keys"),
        (Route::Docs {}, "Docs"),
    ];

    let active_href = nav_items
        .iter()
        .map(|(route, _)| route.to_string())
        .filter(|href| current_path.starts_with(href.as_str()))
        .max_by_key(|href| href.len());

    rsx! {
        aside { class: "side sticky top-0 z-30 flex flex-row items-center gap-2 sm:gap-3 px-2.5 sm:px-3.5 py-2.5 md:flex-col md:items-stretch md:gap-0 md:h-dvh md:pt-5.5 md:pb-4 md:px-3.5 bg-bg-2/65 backdrop-blur-lg border-b border-border-1 md:border-b-0 md:border-r",
            Link {
                class: "logo inline-flex items-center gap-2 sm:gap-2.5 px-1.5 sm:px-2.5 font-display font-bold text-[15px] sm:text-base tracking-tight text-text-1 shrink-0",
                to: Route::SitesIndex {},
                span { class: "logo-mark bg-iri inline-block h-3.25 w-3.25 rounded-sm", "aria-hidden": "true" }
                span { class: "hidden min-[380px]:inline", "Stomatopod" }
            }
            nav { class: "side-nav flex flex-row items-center gap-0.5 flex-1 min-w-0 overflow-x-auto scrollbar-none md:flex-col md:overflow-visible md:mt-7 md:items-stretch",
                span { class: "side-label hidden md:block text-[10.5px] font-semibold uppercase tracking-wider text-muted-2 mx-3 mt-4 mb-1",
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
                                "relative shrink-0 px-2.5 sm:px-3 py-1.5 md:py-2 rounded-[10px] text-[12.5px] sm:text-[13.5px] font-medium text-text-1 bg-teal-soft before:content-[''] before:absolute before:left-2.5 before:right-2.5 before:bottom-0.5 before:h-0.5 before:rounded-full before:bg-iri before:shadow-[0_0_8px_rgba(45,212,191,0.5)] md:before:left-0 md:before:right-auto md:before:top-2 md:before:bottom-2 md:before:w-[3px] md:before:h-auto"
                            } else {
                                "shrink-0 px-2.5 sm:px-3 py-1.5 md:py-2 rounded-[10px] text-[12.5px] sm:text-[13.5px] font-medium text-muted-1 hover:text-text-1 hover:bg-text-1/4"
                            },
                            to: route,
                            "{label}"
                        }
                    }
                    }
                }
            }
            form { class: "side-foot shrink-0 md:mt-auto md:grid", method: "post", action: "/logout",
                button {
                    class: "btn btn-ghost justify-center inline-flex items-center gap-1.5 px-2.5 sm:px-4 py-1.5 sm:py-2 rounded-[10px] border border-border-2 bg-text-1/3 text-text-2 text-[12px] sm:text-[13px] shadow-inner-hi",
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
        div { class: "flex justify-between items-start sm:items-center gap-x-4 sm:gap-x-6 gap-y-3 sm:gap-y-4 mb-6 sm:mb-8 flex-wrap",
            div { class: "min-w-0",
                h1 { class: "text-[20px] sm:text-[24px] font-bold tracking-tight text-text-1 leading-tight", "{title}" }
                if let Some(sub) = subtitle {
                    p { class: "text-muted-1 text-[12.5px] sm:text-[13px] mt-1 break-all sm:break-normal", "{sub}" }
                }
            }
            div { class: "flex items-center gap-2 sm:gap-3 flex-wrap", {children} }
        }
    }
}
