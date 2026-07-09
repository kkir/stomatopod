use dioxus::prelude::*;

use crate::ui::api::get_json;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::types::{DocsHtml, DocsTocItem};

/// Descendant-styling for the server-rendered markdown injected below. Tailwind
/// v4 has no typography plugin here, so base element styles are applied via
/// arbitrary descendant variants.
/// `scroll-mt-8` keeps an anchored heading clear of the top edge when jumped
/// to from the anchor nav.
const PROSE: &str = "docs-prose text-text-2 text-[14px] leading-relaxed \
    [&_h1]:text-text-1 [&_h1]:font-display [&_h1]:text-2xl [&_h1]:font-bold [&_h1]:mb-4 [&_h1]:mt-2 \
    [&_h2]:text-text-1 [&_h2]:font-semibold [&_h2]:text-lg [&_h2]:mt-6 [&_h2]:mb-2 [&_h2]:scroll-mt-8 \
    [&_h3]:text-text-1 [&_h3]:font-semibold [&_h3]:mt-4 [&_h3]:mb-2 [&_h3]:scroll-mt-8 \
    [&_p]:mb-3 [&_ul]:list-disc [&_ul]:pl-5 [&_ul]:mb-3 [&_ol]:list-decimal [&_ol]:pl-5 [&_ol]:mb-3 \
    [&_li]:mb-1 [&_a]:text-teal-hi [&_a]:underline \
    [&_code]:bg-black/40 [&_code]:rounded [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-[13px] [&_code]:font-mono \
    [&_pre]:bg-black/40 [&_pre]:rounded-lg [&_pre]:p-3 [&_pre]:overflow-x-auto [&_pre]:mb-3 \
    [&_pre_code]:bg-transparent [&_pre_code]:p-0";

/// Resolve which heading is currently in view (last H2/H3 whose top is at or
/// above the scrollspy threshold).
#[cfg(target_arch = "wasm32")]
fn current_heading_id(document: &web_sys::Document) -> String {
    /// How far from the top of the viewport a heading must cross before it is
    /// considered the "current" section (matches scroll-mt-8 + a little room).
    const SCROLLSPY_TOP_PX: f64 = 96.0;
    let Ok(list) = document.query_selector_all(".docs-prose h2[id], .docs-prose h3[id]") else {
        return String::new();
    };
    let mut current = String::new();
    for i in 0..list.length() {
        let Some(node) = list.item(i) else { continue };
        let Ok(el) = node.dyn_into::<web_sys::Element>() else {
            continue;
        };
        if el.get_bounding_client_rect().top() <= SCROLLSPY_TOP_PX {
            if let Some(id) = el.get_attribute("id") {
                current = id;
            }
        }
    }
    if current.is_empty() {
        if let Some(node) = list.item(0) {
            if let Ok(el) = node.dyn_into::<web_sys::Element>() {
                current = el.get_attribute("id").unwrap_or_default();
            }
        }
    }
    current
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

/// Holds scroll/resize listeners and removes them on drop so SPA navigations
/// off Docs do not leak handlers.
#[cfg(target_arch = "wasm32")]
struct ScrollSpyGuard {
    scroll_target: web_sys::EventTarget,
    window: web_sys::Window,
    closure: wasm_bindgen::closure::Closure<dyn FnMut()>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for ScrollSpyGuard {
    fn drop(&mut self) {
        let cb = self.closure.as_ref().unchecked_ref();
        let _ = self
            .scroll_target
            .remove_event_listener_with_callback("scroll", cb);
        let _ = self.window.remove_event_listener_with_callback("resize", cb);
    }
}

/// Sticky "On this page" nav with scrollspy highlighting of the section in view.
#[component]
fn DocsToc(toc: Vec<DocsTocItem>) -> Element {
    let mut active = use_signal(|| {
        toc.first()
            .map(|t| t.slug.clone())
            .unwrap_or_default()
    });

    // Install scrollspy once the TOC (and sibling prose) is mounted. The guard
    // is dropped with the component via use_drop, detaching listeners.
    #[cfg(target_arch = "wasm32")]
    {
        let mut guard = use_signal(|| None::<ScrollSpyGuard>);
        use_effect(move || {
            let Some(window) = web_sys::window() else {
                return;
            };
            let Some(document) = window.document() else {
                return;
            };

            let scroll_target: web_sys::EventTarget = document
                .query_selector("main.main")
                .ok()
                .flatten()
                .map(|el| el.unchecked_into::<web_sys::EventTarget>())
                .unwrap_or_else(|| window.clone().unchecked_into());

            let mut active = active;
            let document_for_cb = document.clone();
            let closure = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
                let id = current_heading_id(&document_for_cb);
                if !id.is_empty() && active.peek().as_str() != id {
                    active.set(id);
                }
            }) as Box<dyn FnMut()>);

            let _ = scroll_target
                .add_event_listener_with_callback("scroll", closure.as_ref().unchecked_ref());
            let _ = window
                .add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref());

            // Initial pass after layout so dangerous_inner_html headings exist.
            let id = current_heading_id(&document);
            if !id.is_empty() {
                active.set(id);
            }

            guard.set(Some(ScrollSpyGuard {
                scroll_target,
                window,
                closure,
            }));
        });
        use_drop(move || {
            guard.set(None);
        });
    }

    rsx! {
        nav {
            // Stick inside the main scrollport. Cap height so a long TOC scrolls
            // independently without escaping the viewport.
            class: "hidden lg:block sticky top-8 self-start max-h-[calc(100dvh-4rem)] overflow-y-auto",
            "aria-label": "On this page",
            div { class: "text-[10.5px] font-semibold uppercase tracking-wider text-muted-2 mb-2.5 px-2",
                "On this page"
            }
            ul { class: "flex flex-col gap-0.5 border-l border-border-1",
                for item in toc {
                    {
                        let slug = item.slug.clone();
                        let is_active = active() == slug;
                        let base = if item.level >= 3 {
                            "block py-1 pr-2 pl-6 text-[12.5px] no-underline border-l-2 -ml-px transition-colors"
                        } else {
                            "block py-1 pr-2 pl-3 text-[12.5px] font-medium no-underline border-l-2 -ml-px transition-colors"
                        };
                        let class = if is_active {
                            format!(
                                "{base} text-text-1 border-teal bg-teal-soft/50 hover:text-text-1"
                            )
                        } else {
                            format!(
                                "{base} text-muted-1 border-transparent hover:text-text-1 hover:border-teal/50"
                            )
                        };
                        rsx! {
                            li { key: "{item.slug}",
                                a {
                                    href: "#{item.slug}",
                                    class: "{class}",
                                    "aria-current": if is_active { "location" },
                                    onclick: {
                                        let slug = slug.clone();
                                        move |_| active.set(slug.clone())
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
                            DocsToc { toc }
                        }
                    }
                }
            }
        }}
    }
}
