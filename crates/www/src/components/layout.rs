use dioxus::prelude::*;

use crate::routes::Route;

const GITHUB_URL: &str = "https://github.com/kkir/stomatopod";

/// Marketing shell: sticky top nav + main content + footer.
#[component]
pub fn MarketingShell() -> Element {
    rsx! {
        a {
            class: "skip-link focus:skip-link-focus",
            href: "#main-content",
            "Skip to main content"
        }
        div { class: "min-h-dvh flex flex-col bg-bg font-ui text-text-1",
            SiteNav {}
            main {
                id: "main-content",
                class: "flex-1 min-w-0",
                tabindex: "-1",
                Outlet::<Route> {}
            }
            SiteFooter {}
        }
    }
}

#[component]
fn SiteNav() -> Element {
    let current = use_route::<Route>();
    let path = current.to_string();

    // Precompute active flags so the rsx! tree stays flat (no nested rsx!
    // blocks inside the loop, which can re-enter the runtime during hydrate).
    let home_active = path == "/";
    let features_active = path == "/features" || path.starts_with("/features/");
    let get_started_active = path == "/get-started" || path.starts_with("/get-started/");

    let home_class = nav_class(home_active);
    let features_class = nav_class(features_active);
    let get_started_class = nav_class(get_started_active);

    rsx! {
        header {
            class: "sticky top-0 z-30 border-b border-border-1 bg-bg-2/75 backdrop-blur-lg",
            "aria-label": "Site",
            div { class: "mx-auto flex max-w-5xl items-center justify-between gap-4 px-4 py-3 sm:px-6",
                Link {
                    class: "inline-flex items-center gap-2.5 font-display font-bold text-[15px] sm:text-base tracking-tight text-text-1 no-underline",
                    to: Route::Home {},
                    "aria-label": "Stomatopod home",
                    span {
                        class: "bg-iri inline-block h-3.25 w-3.25 rounded-sm shrink-0",
                        "aria-hidden": "true",
                    }
                    span { "Stomatopod" }
                }
                nav {
                    class: "flex items-center gap-1 sm:gap-2",
                    "aria-label": "Primary",
                    Link {
                        class: "{home_class}",
                        to: Route::Home {},
                        "aria-current": if home_active { "page" },
                        "Home"
                    }
                    Link {
                        class: "{features_class}",
                        to: Route::Features {},
                        "aria-current": if features_active { "page" },
                        "Features"
                    }
                    Link {
                        class: "{get_started_class}",
                        to: Route::GetStarted {},
                        "aria-current": if get_started_active { "page" },
                        "Get started"
                    }
                    a {
                        class: "hidden sm:inline-flex ml-1 items-center px-[13px] py-1.5 rounded-[10px] text-[13px] font-semibold tracking-tight bg-grad-btn text-[#032621] shadow-glow no-underline hover:-translate-y-px transition-transform",
                        href: "{GITHUB_URL}",
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "GitHub"
                    }
                }
            }
        }
    }
}

fn nav_class(active: bool) -> &'static str {
    if active {
        "px-2.5 py-1.5 rounded-md text-[13px] font-semibold text-text-1 bg-teal-soft no-underline"
    } else {
        "px-2.5 py-1.5 rounded-md text-[13px] font-semibold text-muted-1 hover:text-text-1 no-underline"
    }
}

#[component]
fn SiteFooter() -> Element {
    rsx! {
        footer {
            class: "border-t border-border-1 bg-bg-2/40",
            div { class: "mx-auto max-w-5xl px-4 sm:px-6 py-8 flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 text-[13px] text-muted-1",
                p {
                    "Stomatopod - privacy-friendly web analytics. "
                    span { class: "text-muted-2", "MIT licensed." }
                }
                div { class: "flex flex-wrap gap-4",
                    Link {
                        class: "text-muted-1 hover:text-teal-hi no-underline",
                        to: Route::Features {},
                        "Features"
                    }
                    Link {
                        class: "text-muted-1 hover:text-teal-hi no-underline",
                        to: Route::GetStarted {},
                        "Get started"
                    }
                    a {
                        class: "text-muted-1 hover:text-teal-hi no-underline",
                        href: "{GITHUB_URL}",
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "GitHub"
                    }
                }
            }
        }
    }
}
