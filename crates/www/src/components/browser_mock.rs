use dioxus::prelude::*;

/// Simple window frame around a UI preview (no traffic lights / URL bar).
/// Decorative only; the surrounding feature title stays the accessible name.
#[component]
pub fn WindowFrame(children: Element) -> Element {
    rsx! {
        div {
            class: "border-b border-border-1 bg-bg overflow-hidden select-none pointer-events-none cursor-default",
            "aria-hidden": "true",
            // Room for ~0.62-scaled dashboard chrome without feeling huge.
            div { class: "relative h-[16rem] sm:h-[18rem] overflow-hidden",
                {children}
            }
        }
    }
}
