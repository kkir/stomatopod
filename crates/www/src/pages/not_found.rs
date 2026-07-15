use dioxus::prelude::*;

use crate::routes::Route;

#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let path = if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    };

    rsx! {
        div { class: "mx-auto max-w-5xl px-4 sm:px-6 py-20 sm:py-28 text-center",
            p { class: "text-teal-hi text-[12px] font-semibold uppercase tracking-[0.16em] mb-3",
                "404"
            }
            h1 { class: "font-display font-bold text-3xl sm:text-4xl tracking-tight text-text-1",
                "Page not found"
            }
            p { class: "mt-4 text-muted-1 text-[14px]",
                "No page at "
                code { class: "bg-black/40 rounded px-1.5 py-0.5 font-mono text-[12.5px] text-text-2",
                    "{path}"
                }
                "."
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
