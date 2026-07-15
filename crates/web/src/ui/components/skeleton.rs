use dioxus::prelude::*;

/// Shimmering placeholder bars shown while a `use_resource` is `None`,
/// port of `.skeleton` (legacy dashboard stylesheet Skeleton section).
#[component]
pub fn Skeleton(lines: usize) -> Element {
    rsx! {
        div {
            class: "flex flex-col gap-2",
            role: "status",
            "aria-busy": "true",
            "aria-live": "polite",
            span { class: "sr-only", "Loading" }
            for i in 0..lines {
                div {
                    key: "{i}",
                    class: "skeleton-bar h-5 motion-reduce:animate-none",
                    "aria-hidden": "true",
                }
            }
        }
    }
}
