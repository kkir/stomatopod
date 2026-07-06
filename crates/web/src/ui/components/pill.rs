use dioxus::prelude::*;

/// Status pill, port of `.pill`.
#[component]
pub fn Pill(children: Element) -> Element {
    rsx! {
        span { class: "inline-flex items-center gap-1.5 px-[9px] py-[3px] rounded-full text-[11px] font-semibold uppercase tracking-[0.06em] bg-surface-2 border border-border-2 text-muted-1",
            {children}
        }
    }
}

/// An active-filter chip with a remove button.
#[component]
pub fn FilterPill(label: String, on_remove: EventHandler<()>) -> Element {
    rsx! {
        span { class: "inline-flex items-center gap-1.5 px-[9px] py-[3px] rounded-full text-[11px] font-semibold bg-teal-soft border border-border-2 text-teal-hi",
            "{label}"
            button {
                class: "text-teal-hi/70 hover:text-teal-hi cursor-pointer bg-transparent border-0 p-0 leading-none",
                r#type: "button",
                onclick: move |_| on_remove.call(()),
                "×"
            }
        }
    }
}
