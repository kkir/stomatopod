use dioxus::prelude::*;

use crate::ui::types::{FilterDraft, FunnelStepDraft, FunnelStepResult};

const FB_INPUT: &str = "bg-black/32 border border-border-2 text-text-1 rounded-[10px] px-3 py-2 text-[13px] shadow-inner-hi focus:outline-none focus:border-teal/55";
const FB_BTN_PRIMARY: &str = "inline-flex items-center gap-1.5 px-[15px] py-2 rounded-[10px] text-[13px] font-semibold tracking-tight cursor-pointer bg-grad-btn text-[#032621] shadow-glow";
const FB_BTN_GHOST: &str = "inline-flex items-center gap-1.5 px-[13px] py-1.5 rounded-[10px] text-[12px] font-semibold tracking-tight cursor-pointer bg-text-1/3 text-text-2 border border-border-2 shadow-inner-hi hover:text-text-1";

/// Filter field options, matching the `FIELDS` list in the legacy
/// funnels.jinja builder script.
const FIELDS: [(&str, &str); 10] = [
    ("url", "URL"),
    ("referrer", "Referrer"),
    ("country", "Country"),
    ("browser", "Browser"),
    ("os", "OS"),
    ("device_type", "Device"),
    ("utm_source", "UTM Source"),
    ("utm_medium", "UTM Medium"),
    ("utm_campaign", "UTM Campaign"),
    ("event_name", "Event Name"),
];

const OPS: [(&str, &str); 4] = [
    ("eq", "is"),
    ("not_eq", "is not"),
    ("contains", "contains"),
    ("starts_with", "starts with"),
];

/// The funnel builder, a signals-based replacement for the ~75-line
/// vanilla-JS DOM builder in funnels.jinja. Emits `(name, steps)` on submit;
/// starts with two empty steps and supports add/remove of steps and
/// per-step filters.
#[component]
pub fn FunnelBuilder(on_submit: EventHandler<(String, Vec<FunnelStepDraft>)>) -> Element {
    let mut name = use_signal(String::new);
    let mut steps = use_signal(|| {
        vec![
            FunnelStepDraft {
                name: String::new(),
                event_name: "pageview".to_string(),
                filters: Vec::new(),
            },
            FunnelStepDraft {
                name: String::new(),
                event_name: String::new(),
                filters: Vec::new(),
            },
        ]
    });

    rsx! {
        form {
            onsubmit: move |evt: FormEvent| {
                evt.prevent_default();
                let n = name().trim().to_string();
                if n.is_empty() {
                    return;
                }
                let drafts = steps();
                if drafts.len() < 2 {
                    return;
                }
                on_submit.call((n, drafts));
            },
            div { class: "mb-3.5",
                label { class: "block",
                    span { class: "block text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 mb-1.5 font-semibold",
                        "Funnel name"
                    }
                    input {
                        class: "{FB_INPUT} w-full",
                        value: "{name}",
                        placeholder: "e.g. Signup flow",
                        required: true,
                        oninput: move |e| name.set(e.value()),
                    }
                }
            }
            div {
                role: "group",
                "aria-label": "Funnel steps",
                span { class: "block text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 mb-2 font-semibold",
                    "Steps"
                }
            div { class: "flex flex-col gap-3",
                for (i , step) in steps().into_iter().enumerate() {
                    div {
                        key: "{i}",
                        class: "bg-surface-2/60 border border-border-1 rounded-lg p-3",
                        role: "group",
                        "aria-label": "Step {i + 1}",
                        div { class: "flex items-center gap-2 mb-2",
                            span {
                                class: "text-muted-1 text-xs font-semibold w-14",
                                "aria-hidden": "true",
                                "Step {i + 1}"
                            }
                            input {
                                class: "{FB_INPUT} flex-1",
                                value: "{step.name}",
                                placeholder: "Step label",
                                "aria-label": "Step {i + 1} label",
                                oninput: move |e| {
                                    steps.write()[i].name = e.value();
                                },
                            }
                            input {
                                class: "{FB_INPUT} flex-1",
                                value: "{step.event_name}",
                                placeholder: "Event name",
                                "aria-label": "Step {i + 1} event name",
                                oninput: move |e| {
                                    steps.write()[i].event_name = e.value();
                                },
                            }
                            if steps().len() > 2 {
                                button {
                                    r#type: "button",
                                    class: FB_BTN_GHOST,
                                    "aria-label": "Remove step {i + 1}",
                                    onclick: move |_| {
                                        steps.write().remove(i);
                                    },
                                    "Remove"
                                }
                            }
                        }
                        div { class: "flex flex-col gap-2 pl-14",
                            for (j , filter) in step.filters.iter().enumerate() {
                                div {
                                    key: "{j}",
                                    class: "flex items-center gap-2",
                                    role: "group",
                                    "aria-label": "Step {i + 1} filter {j + 1}",
                                    select {
                                        class: FB_INPUT,
                                        value: "{filter.field}",
                                        "aria-label": "Filter field",
                                        onchange: move |e| {
                                            steps.write()[i].filters[j].field = e.value();
                                        },
                                        for (val , label) in FIELDS {
                                            option { key: "{val}", value: "{val}", "{label}" }
                                        }
                                    }
                                    select {
                                        class: FB_INPUT,
                                        value: "{filter.op}",
                                        "aria-label": "Filter operator",
                                        onchange: move |e| {
                                            steps.write()[i].filters[j].op = e.value();
                                        },
                                        for (val , label) in OPS {
                                            option { key: "{val}", value: "{val}", "{label}" }
                                        }
                                    }
                                    input {
                                        class: "{FB_INPUT} flex-1",
                                        value: "{filter.value}",
                                        placeholder: "value",
                                        "aria-label": "Filter value",
                                        oninput: move |e| {
                                            steps.write()[i].filters[j].value = e.value();
                                        },
                                    }
                                    button {
                                        r#type: "button",
                                        class: FB_BTN_GHOST,
                                        "aria-label": "Remove filter",
                                        onclick: move |_| {
                                            steps.write()[i].filters.remove(j);
                                        },
                                        span { "aria-hidden": "true", "×" }
                                    }
                                }
                            }
                            button {
                                r#type: "button",
                                class: "{FB_BTN_GHOST} self-start",
                                onclick: move |_| {
                                    steps
                                        .write()[i]
                                        .filters
                                        .push(FilterDraft {
                                            field: "url".to_string(),
                                            op: "eq".to_string(),
                                            value: String::new(),
                                        });
                                },
                                "+ Filter"
                            }
                        }
                    }
                }
            }
            }
            div { class: "flex items-center gap-2 mt-3",
                button {
                    r#type: "button",
                    class: FB_BTN_GHOST,
                    onclick: move |_| {
                        steps
                            .write()
                            .push(FunnelStepDraft {
                                name: String::new(),
                                event_name: String::new(),
                                filters: Vec::new(),
                            });
                    },
                    "+ Add step"
                }
                button { r#type: "submit", class: FB_BTN_PRIMARY, "Create funnel" }
            }
        }
    }
}

/// The funnel result bar chart, port of the `.funnel` block in
/// funnels.jinja (lines 26-38): one bar per step, height =
/// `conversion_rate * 180px` via an inline `--h` style.
#[component]
pub fn FunnelBars(steps: Vec<FunnelStepResult>) -> Element {
    rsx! {
        div {
            class: "overflow-x-auto -mx-1 px-1",
            role: "img",
            "aria-label": "Funnel conversion by step",
            // Accessible data; visual bars are decorative.
            table { class: "sr-only",
                caption { "Funnel steps" }
                thead {
                    tr {
                        th { scope: "col", "Step" }
                        th { scope: "col", "Sessions" }
                        th { scope: "col", "Conversion" }
                    }
                }
                tbody {
                    for step in steps.iter() {
                        tr {
                            th { scope: "row", "{step.name}" }
                            td { "{step.sessions}" }
                            td { "{(step.conversion_rate * 100.0):.0}%" }
                        }
                    }
                }
            }
            div {
                class: "flex items-end gap-3 sm:gap-4 min-h-[200px] sm:min-h-[220px] pt-4 min-w-[min(100%,18rem)]",
                "aria-hidden": "true",
                for (i , step) in steps.iter().enumerate() {
                    div { key: "{i}", class: "flex-1 min-w-[3.5rem] flex flex-col items-center justify-end gap-1.5",
                        div { class: "text-teal-hi text-[12px] sm:text-[13px] font-semibold tabular-nums",
                            "{(step.conversion_rate * 100.0):.0}%"
                        }
                        div {
                            class: "w-full max-w-[80px] rounded-t-md bg-grad-bar min-h-[3px]",
                            style: "height: {(step.conversion_rate * 180.0) as i64}px",
                        }
                        div { class: "text-text-1 text-[12px] sm:text-[13px] font-medium text-center break-words max-w-full", "{step.name}" }
                        div { class: "text-muted-1 text-xs tabular-nums", "{step.sessions}" }
                    }
                }
            }
        }
    }
}
