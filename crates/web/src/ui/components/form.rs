use dioxus::prelude::*;

/// A labelled control wrapper, port of `.field`.
#[component]
pub fn Field(label: String, children: Element) -> Element {
    rsx! {
        div { class: "mb-3.5",
            label { class: "block text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 mb-1.5 font-semibold",
                "{label}"
            }
            {children}
        }
    }
}

/// Port of `select` styling wrapped in [`Field`].
#[component]
pub fn SelectField(
    label: String,
    value: String,
    options: Vec<(String, String)>,
    onchange: EventHandler<String>,
) -> Element {
    rsx! {
        Field { label,
            select {
                class: "w-full bg-black/32 border border-border-2 text-text-1 rounded-[10px] px-3 py-2 text-[13px] shadow-inner-hi focus:outline-none focus:border-teal/55",
                value: "{value}",
                onchange: move |evt| onchange.call(evt.value()),
                for (val , option_label) in options {
                    option { key: "{val}", value: "{val}", selected: val == value, "{option_label}" }
                }
            }
        }
    }
}

/// Toggle switch, port of `.switch`/`.switch-track`.
#[component]
pub fn Switch(label: String, checked: bool, onchange: EventHandler<bool>) -> Element {
    rsx! {
        label { class: "inline-flex items-center gap-2.5 min-h-[38px] cursor-pointer",
            span { class: "relative w-[42px] h-6 inline-block",
                input {
                    class: "absolute opacity-0 pointer-events-none",
                    r#type: "checkbox",
                    checked,
                    onchange: move |evt| onchange.call(evt.checked()),
                }
                span {
                    class: if checked {
                        "block w-full h-full rounded-full bg-teal/60 border border-teal/70 relative transition-colors after:content-[''] after:absolute after:left-[20px] after:top-0.5 after:w-[18px] after:h-[18px] after:rounded-full after:bg-[#e8fffb] after:transition-transform"
                    } else {
                        "block w-full h-full rounded-full bg-black/40 border border-border-2 relative transition-colors after:content-[''] after:absolute after:left-0.5 after:top-0.5 after:w-[18px] after:h-[18px] after:rounded-full after:bg-[#dbe7ec] after:transition-transform"
                    },
                }
            }
            span { class: "text-text-2 text-[13px]", "{label}" }
        }
    }
}
