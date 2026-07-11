use dioxus::prelude::*;

use crate::ui::api::get_json;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::types::{DocsHtml, DocsTocItem};

/// Descendant-styling for the server-rendered markdown injected below.
/// `scroll-mt-8` keeps an anchored heading clear of the top edge when jumped to.
const PROSE: &str = "docs-prose text-text-2 text-[14px] leading-relaxed \
    [&_h1]:text-text-1 [&_h1]:font-display [&_h1]:text-2xl [&_h1]:font-bold [&_h1]:mb-4 [&_h1]:mt-2 \
    [&_h2]:text-text-1 [&_h2]:font-semibold [&_h2]:text-lg [&_h2]:mt-6 [&_h2]:mb-2 [&_h2]:scroll-mt-8 \
    [&_h3]:text-text-1 [&_h3]:font-semibold [&_h3]:mt-4 [&_h3]:mb-2 [&_h3]:scroll-mt-8 \
    [&_p]:mb-3 [&_ul]:list-disc [&_ul]:pl-5 [&_ul]:mb-3 [&_ol]:list-decimal [&_ol]:pl-5 [&_ol]:mb-3 \
    [&_li]:mb-1 [&_a]:text-teal-hi [&_a]:underline \
    [&_code]:bg-black/40 [&_code]:rounded [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-[13px] [&_code]:font-mono \
    [&_pre]:bg-black/40 [&_pre]:rounded-lg [&_pre]:p-3 [&_pre]:overflow-x-auto [&_pre]:mb-3 \
    [&_pre_code]:bg-transparent [&_pre_code]:p-0";

const TOC_IDLE: &str = "block py-1 pr-2 text-[12.5px] no-underline border-l-2 -ml-px transition-colors text-muted-1 border-transparent hover:text-text-1 hover:border-teal/50";
const TOC_ACTIVE: &str = "block py-1 pr-2 text-[12.5px] no-underline border-l-2 -ml-px transition-colors text-text-1 border-teal bg-teal-soft/50 font-medium";

fn scroll_id_into_view(id: &str) {
    if id.is_empty() {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            if let Some(el) = document.get_element_by_id(id) {
                el.scroll_into_view_with_bool(true);
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = id;
    }
}

fn location_hash() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.location().hash().ok())
            .map(|h| h.trim_start_matches('#').to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        String::new()
    }
}

/// Heading closest to the top of the scrollport (under a small top band).
#[cfg(target_arch = "wasm32")]
fn heading_in_view() -> String {
    use wasm_bindgen::JsCast;
    const TOP_PX: f64 = 120.0;
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return String::new();
    };
    let Ok(list) = document.query_selector_all(".docs-prose h2[id], .docs-prose h3[id]") else {
        return String::new();
    };
    let mut current = String::new();
    for i in 0..list.length() {
        let Some(node) = list.item(i) else { continue };
        let Ok(el) = node.dyn_into::<web_sys::Element>() else {
            continue;
        };
        if el.get_bounding_client_rect().top() <= TOP_PX {
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

/// Keeps the IntersectionObserver alive and disconnects it on drop.
#[cfg(target_arch = "wasm32")]
struct HeadingObserver {
    observer: web_sys::IntersectionObserver,
    _callback:
        wasm_bindgen::closure::Closure<dyn FnMut(js_sys::Array, web_sys::IntersectionObserver)>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for HeadingObserver {
    fn drop(&mut self) {
        self.observer.disconnect();
    }
}

/// Observe prose headings. IO callbacks re-enter Dioxus via `RuntimeGuard`
/// using a runtime captured while an effect was still on the stack.
#[cfg(target_arch = "wasm32")]
fn install_heading_observer(
    mut active: Signal<String>,
    runtime: std::rc::Rc<dioxus::core::Runtime>,
) -> Option<HeadingObserver> {
    use dioxus::core::RuntimeGuard;
    use wasm_bindgen::JsCast;
    use web_sys::{IntersectionObserver, IntersectionObserverInit};

    let window = web_sys::window()?;
    let document = window.document()?;
    let root = document.query_selector("main.main").ok().flatten();

    let callback = wasm_bindgen::closure::Closure::wrap(Box::new(
        move |_entries: js_sys::Array, _obs: IntersectionObserver| {
            let _guard = RuntimeGuard::new(runtime.clone());
            let id = heading_in_view();
            if id.is_empty() {
                return;
            }
            if *active.peek() != id {
                active.set(id);
            }
        },
    )
        as Box<dyn FnMut(js_sys::Array, IntersectionObserver)>);

    let options = IntersectionObserverInit::new();
    if let Some(root) = root.as_ref() {
        options.set_root(Some(root));
    }
    // Top band is the "active" zone; ignore most of the lower root area.
    options.set_root_margin("-16px 0px -70% 0px");
    options.set_threshold_f64(0.0);

    let observer =
        IntersectionObserver::new_with_options(callback.as_ref().unchecked_ref(), &options).ok()?;

    let Ok(list) = document.query_selector_all(".docs-prose h2[id], .docs-prose h3[id]") else {
        return None;
    };
    for i in 0..list.length() {
        if let Some(node) = list.item(i) {
            if let Ok(el) = node.dyn_into::<web_sys::Element>() {
                observer.observe(&el);
            }
        }
    }

    Some(HeadingObserver {
        observer,
        _callback: callback,
    })
}

/// Body + TOC once docs HTML is loaded.
#[component]
fn DocsBody(html: String, toc: Vec<DocsTocItem>) -> Element {
    let initial = {
        let h = location_hash();
        if !h.is_empty() {
            h
        } else {
            toc.first().map(|t| t.slug.clone()).unwrap_or_default()
        }
    };
    let mut active = use_signal(|| initial);

    // Deep link after prose paints.
    use_effect(move || {
        let hash = location_hash();
        if hash.is_empty() {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        {
            spawn(async move {
                gloo_timers::future::TimeoutFuture::new(0).await;
                active.set(hash.clone());
                scroll_id_into_view(&hash);
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = hash;
        }
    });

    // Intersection Observer scrollspy. Same hooks on all targets so SSR and
    // wasm keep identical hook order; install is a no-op off wasm.
    #[cfg(target_arch = "wasm32")]
    let mut observer_guard = use_signal(|| None::<HeadingObserver>);
    #[cfg(not(target_arch = "wasm32"))]
    let mut observer_guard = use_signal(|| false);

    use_effect(move || {
        #[cfg(target_arch = "wasm32")]
        {
            use dioxus::core::{Runtime, RuntimeGuard};
            // Capture while the effect still holds the runtime TLS stack.
            let runtime = Runtime::current();
            let active = active;
            let mut observer_guard = observer_guard;
            spawn(async move {
                // Wait for dangerous_inner_html headings to exist.
                gloo_timers::future::TimeoutFuture::new(0).await;
                let Some(obs) = install_heading_observer(active, runtime.clone()) else {
                    return;
                };
                // Store the observer under a runtime guard (spawn resumes
                // without TLS runtime after .await).
                let _g = RuntimeGuard::new(runtime);
                observer_guard.set(Some(obs));
            });
        }
    });
    use_drop(move || {
        #[cfg(target_arch = "wasm32")]
        {
            observer_guard.set(None);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            observer_guard.set(false);
        }
    });

    let current = active();

    rsx! {
        div { class: "grid grid-cols-1 lg:grid-cols-[minmax(0,1fr)_216px] gap-6 items-start",
            Card {
                div { class: PROSE, dangerous_inner_html: "{html}" }
            }
            if !toc.is_empty() {
                nav {
                    class: "hidden lg:block sticky top-8 self-start max-h-[calc(100dvh-4rem)] overflow-y-auto",
                    "aria-label": "On this page",
                    div { class: "text-[10.5px] font-semibold uppercase tracking-wider text-muted-2 mb-2.5 px-2",
                        "On this page"
                    }
                    ul { class: "flex flex-col gap-0.5 border-l border-border-1",
                        for item in toc.iter() {
                            {
                                let slug = item.slug.clone();
                                let is_active = current == slug;
                                let pad = if item.level >= 3 { "pl-6" } else { "pl-3" };
                                let class = if is_active {
                                    format!("{TOC_ACTIVE} {pad}")
                                } else {
                                    format!("{TOC_IDLE} {pad}")
                                };
                                let key = format!("{}-{}", item.slug, is_active);
                                rsx! {
                                    li { key: "{key}",
                                        a {
                                            href: "#{slug}",
                                            class: "{class}",
                                            "aria-current": if is_active { "true" },
                                            onclick: {
                                                let slug = slug.clone();
                                                move |evt| {
                                                    evt.prevent_default();
                                                    active.set(slug.clone());
                                                    scroll_id_into_view(&slug);
                                                }
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
    }
}

/// Docs page: injects markdown HTML from `GET /api/v1/docs` plus a TOC.
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
            Some(Ok(d)) => rsx! {
                DocsBody { html: d.html.clone(), toc: d.toc.clone() }
            },
        }}
    }
}
