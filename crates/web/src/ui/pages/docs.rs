use dioxus::prelude::*;

use crate::ui::api::get_json;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::types::DocsHtml;

/// Descendant-styling for the server-rendered markdown injected below. Tailwind
/// v4 has no typography plugin here, so base element styles are applied via
/// arbitrary descendant variants.
/// `scroll-mt-8` keeps an anchored heading clear of the top edge when jumped
/// to from the anchor nav.
const PROSE: &str = "text-text-2 text-[14px] leading-relaxed \
    [&_h1]:text-text-1 [&_h1]:font-display [&_h1]:text-2xl [&_h1]:font-bold [&_h1]:mb-4 [&_h1]:mt-2 \
    [&_h2]:text-text-1 [&_h2]:font-semibold [&_h2]:text-lg [&_h2]:mt-6 [&_h2]:mb-2 [&_h2]:scroll-mt-8 \
    [&_h3]:text-text-1 [&_h3]:font-semibold [&_h3]:mt-4 [&_h3]:mb-2 [&_h3]:scroll-mt-8 \
    [&_p]:mb-3 [&_ul]:list-disc [&_ul]:pl-5 [&_ul]:mb-3 [&_ol]:list-decimal [&_ol]:pl-5 [&_ol]:mb-3 \
    [&_li]:mb-1 [&_a]:text-teal-hi [&_a]:underline \
    [&_code]:bg-black/40 [&_code]:rounded [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-[13px] [&_code]:font-mono \
    [&_pre]:bg-black/40 [&_pre]:rounded-lg [&_pre]:p-3 [&_pre]:overflow-x-auto [&_pre]:mb-3 \
    [&_pre_code]:bg-transparent [&_pre_code]:p-0";

/// Docs page: injects the server-rendered markdown HTML from
/// `GET /api/v1/docs` via `dangerous_inner_html`, alongside a sticky anchor
/// navigation built from the response's `toc` (H2/H3 headings).
#[component]
pub fn Docs() -> Element {
    let docs = use_resource(move || async move { get_json::<DocsHtml>("/api/v1/docs").await });

    rsx! {
        PageHead { title: "Docs", subtitle: "Tracker setup and API reference." }
        {match &*docs.read() {
            None => rsx! {
                Card {
                    Skeleton { lines: 6 }
                }
            },
            Some(Err(e)) => rsx! {
                Card {
                    EmptyState { message: format!("Failed to load docs ({e})") }
                }
            },
            Some(Ok(d)) => {
                let toc = d.toc.clone();
                rsx! {
                    div { class: "grid grid-cols-1 lg:grid-cols-[minmax(0,1fr)_216px] gap-6 items-start",
                        Card {
                            div { class: PROSE, dangerous_inner_html: "{d.html}" }
                        }
                        if !toc.is_empty() {
                            nav {
                                class: "hidden lg:block sticky top-8 self-start",
                                "aria-label": "On this page",
                                div { class: "text-[10.5px] font-semibold uppercase tracking-wider text-muted-2 mb-2.5 px-2",
                                    "On this page"
                                }
                                ul { class: "flex flex-col gap-0.5 border-l border-border-1",
                                    for item in toc {
                                        li { key: "{item.slug}",
                                            a {
                                                href: "#{item.slug}",
                                                class: if item.level >= 3 {
                                                    "block py-1 pr-2 pl-6 text-[12.5px] text-muted-1 hover:text-text-1 no-underline border-l-2 border-transparent hover:border-teal -ml-px"
                                                } else {
                                                    "block py-1 pr-2 pl-3 text-[12.5px] font-medium text-muted-1 hover:text-text-1 no-underline border-l-2 border-transparent hover:border-teal -ml-px"
                                                },
                                                "{item.text}"
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
