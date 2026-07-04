use dioxus::prelude::*;

use crate::api::get_json;
use crate::components::card::{Card, EmptyState};
use crate::components::layout::PageHead;
use crate::components::skeleton::Skeleton;
use crate::types::DocsHtml;

/// Descendant-styling for the server-rendered markdown injected below. Tailwind
/// v4 has no typography plugin here, so base element styles are applied via
/// arbitrary descendant variants.
const PROSE: &str = "text-text-2 text-[14px] leading-relaxed \
    [&_h1]:text-text-1 [&_h1]:font-display [&_h1]:text-2xl [&_h1]:font-bold [&_h1]:mb-4 [&_h1]:mt-2 \
    [&_h2]:text-text-1 [&_h2]:font-semibold [&_h2]:text-lg [&_h2]:mt-6 [&_h2]:mb-2 \
    [&_h3]:text-text-1 [&_h3]:font-semibold [&_h3]:mt-4 [&_h3]:mb-2 \
    [&_p]:mb-3 [&_ul]:list-disc [&_ul]:pl-5 [&_ul]:mb-3 [&_ol]:list-decimal [&_ol]:pl-5 [&_ol]:mb-3 \
    [&_li]:mb-1 [&_a]:text-teal-hi [&_a]:underline \
    [&_code]:bg-black/40 [&_code]:rounded [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-[13px] [&_code]:font-mono \
    [&_pre]:bg-black/40 [&_pre]:rounded-lg [&_pre]:p-3 [&_pre]:overflow-x-auto [&_pre]:mb-3 \
    [&_pre_code]:bg-transparent [&_pre_code]:p-0";

/// Docs page: injects the server-rendered markdown HTML from
/// `GET /api/v1/docs` via `dangerous_inner_html`.
#[component]
pub fn Docs() -> Element {
    let docs = use_resource(move || async move { get_json::<DocsHtml>("/api/v1/docs").await });

    rsx! {
        PageHead { title: "Docs", subtitle: "Tracker setup and API reference." }
        Card {
            {match &*docs.read() {
                None => rsx! {
                    Skeleton { lines: 6 }
                },
                Some(Err(e)) => rsx! {
                    EmptyState { message: format!("Failed to load docs ({e})") }
                },
                Some(Ok(d)) => rsx! {
                    div { class: PROSE, dangerous_inner_html: "{d.html}" }
                },
            }}
        }
    }
}
