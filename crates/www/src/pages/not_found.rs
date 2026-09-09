use dioxus::prelude::*;

use crate::components::page_head::PageHead;
use crate::routes::Route;
use crate::seo;

/// Static `/404` route. SSG pre-renders this into `404/index.html`, which the
/// Pages artifact step copies to root `404.html` and then deletes `404/` so
/// `/404/` is not a 200 URL. GitHub Pages serves root `404.html` as the
/// error document for unknown paths (HTTP 404).
#[component]
pub fn NotFoundPage() -> Element {
    rsx! { NotFoundView { path: None } }
}

#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let path = if segments.is_empty() {
        None
    } else {
        Some(format!("/{}", segments.join("/")))
    };
    rsx! { NotFoundView { path } }
}

#[component]
fn NotFoundView(path: Option<String>) -> Element {
    rsx! {
        PageHead { meta: seo::NOT_FOUND }

        div { class: "mx-auto max-w-5xl px-4 sm:px-6 py-20 sm:py-28 text-center",
            p { class: "text-teal-hi text-[12px] font-semibold uppercase tracking-[0.16em] mb-3",
                "404"
            }
            h1 { class: "font-display font-bold text-3xl sm:text-4xl tracking-tight text-text-1",
                "Page not found"
            }
            p { class: "mt-4 text-muted-1 text-[14px]",
                if let Some(path) = path.as_deref() {
                    "No page at "
                    code { class: "bg-black/40 rounded px-1.5 py-0.5 font-mono text-[12.5px] text-text-2",
                        "{path}"
                    }
                    "."
                } else {
                    "That URL is not a page on this site."
                }
            }
            div { class: "mt-8",
                Link {
                    class: "inline-flex items-center px-[18px] py-2.5 rounded-[10px] text-[14px] font-semibold bg-grad-btn text-[#032621] shadow-glow no-underline",
                    to: Route::Home {},
                    "Back home"
                }
            }
        }
    }
}
