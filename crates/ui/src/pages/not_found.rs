use dioxus::prelude::*;

use crate::components::card::{Card, EmptyState};
use crate::components::layout::PageHead;
use crate::routes::Route;

/// Catch-all page for unknown routes (e.g. stale `/app/sites` links from the
/// legacy dashboard). Renders inside the shell with a way back home.
#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let path = segments.join("/");
    rsx! {
        PageHead { title: "Not found", subtitle: "/{path}" }
        Card {
            EmptyState { message: "This page doesn't exist." }
            div { class: "text-center",
                Link {
                    to: Route::SitesIndex {},
                    class: "text-teal-hi underline",
                    "Back to sites"
                }
            }
        }
    }
}
