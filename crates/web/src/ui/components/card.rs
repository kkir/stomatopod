use dioxus::prelude::*;

/// Base panel container, port of `.card` (dashboard.css Cards section).
/// The iridescent hairline ring is dropped (a `::before` border trick,
/// not worth reproducing in utilities); the border + shadow read is kept.
#[component]
pub fn Card(title: Option<String>, actions: Option<Element>, children: Element) -> Element {
    rsx! {
        div { class: "relative bg-surface-1 border border-border-1 rounded-lg p-f3 shadow-sm shadow-inner-hi",
            if title.is_some() || actions.is_some() {
                div { class: "flex justify-between items-center mb-4 gap-4",
                    if let Some(title) = title {
                        h2 { class: "inline-flex items-center gap-2.5 text-[15px] font-semibold tracking-tight text-text-1",
                            span {
                                class: "inline-block w-[3px] h-3.5 rounded-sm bg-iri shadow-[0_0_8px_rgba(45,212,191,0.4)]",
                                "aria-hidden": "true",
                            }
                            "{title}"
                        }
                    }
                    if let Some(actions) = actions {
                        div { class: "flex items-center gap-2", {actions} }
                    }
                }
            }
            {children}
        }
    }
}

/// Section title + optional CSV export link, port of `.section-header`
/// and the `.csv-btn` ghost link. Kept as its own component (rather than
/// folded into `Card`) since `BreakdownTable`/`EntryExitTable` need it
/// without `Card`'s optional-title branching.
#[component]
pub fn SectionHeader(title: String, csv_href: Option<String>) -> Element {
    rsx! {
        div { class: "flex justify-between items-center mb-4 gap-4",
            h2 { class: "inline-flex items-center gap-2.5 text-[15px] font-semibold tracking-tight text-text-1",
                span {
                    class: "inline-block w-[3px] h-3.5 rounded-sm bg-iri shadow-[0_0_8px_rgba(45,212,191,0.4)]",
                    "aria-hidden": "true",
                }
                "{title}"
            }
            if let Some(href) = csv_href {
                a {
                    class: "csv-btn inline-flex items-center px-[9px] py-0.5 rounded-md border border-border-2 text-[11px] font-semibold tracking-[0.04em] text-muted-1 no-underline hover:text-text-1 hover:border-border-3",
                    href: "{href}",
                    "Export CSV"
                }
            }
        }
    }
}

/// Centered placeholder inside a card, port of `.empty`. In its simplest form
/// (`message` only) it renders a single muted line for errors and terse
/// "nothing here" states. Passing `title` promotes it to a richer empty state
/// with a headline and roomier copy; anything passed as children (typically a
/// call-to-action button or link) renders below the message, so list pages can
/// teach and prompt instead of just saying "No data".
#[component]
pub fn EmptyState(message: String, title: Option<String>, children: Element) -> Element {
    rsx! {
        div { class: "flex flex-col items-center justify-center text-center py-14 px-6",
            if let Some(title) = title {
                h3 { class: "text-text-1 text-[15px] font-semibold tracking-tight", "{title}" }
                p { class: "text-muted-1 text-[13px] leading-relaxed max-w-sm mt-1.5", "{message}" }
            } else {
                p { class: "text-muted-1 text-sm", "{message}" }
            }
            // Empty children render nothing, so the CTA slot adds no stray
            // spacing when a caller omits it.
            {children}
        }
    }
}
