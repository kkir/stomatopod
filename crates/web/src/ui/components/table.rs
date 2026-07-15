use dioxus::prelude::*;

use super::card::{Card, EmptyState, SectionHeader};
use super::chart::Sparkline;

/// Rows shown before "Show more" is clicked. API still returns the full top-N.
const DEFAULT_VISIBLE_ROWS: usize = 8;

/// A single row rendered by [`BreakdownTable`]. Replaces the seven
/// `top_*.jinja` partials plus the `toptable` macro in the legacy
/// site.jinja.
#[derive(Clone, PartialEq, Debug)]
pub struct BreakdownRow {
    pub value: String,
    pub count: u64,
    pub pct: f64,
    /// Optional trend series; when any row carries one the table grows a
    /// Trend column drawing a [`Sparkline`], mirroring the legacy
    /// toptable macro's `sparks` argument.
    pub spark: Option<Vec<f64>>,
}

#[component]
pub fn BreakdownTable(
    title: String,
    value_header: String,
    count_header: String,
    rows: Vec<BreakdownRow>,
    csv_href: Option<String>,
    on_filter: Option<EventHandler<String>>,
    /// When false, omit the outer [`Card`] and section header so the table
    /// can sit inside a [`super::tabs::TabbedCard`].
    #[props(default = true)]
    framed: bool,
    empty_title: Option<String>,
    empty_message: Option<String>,
) -> Element {
    let has_spark = rows.iter().any(|r| r.spark.is_some());
    let mut expanded = use_signal(|| false);
    let total = rows.len();
    let visible: Vec<BreakdownRow> = if expanded() || total <= DEFAULT_VISIBLE_ROWS {
        rows.clone()
    } else {
        rows.iter().take(DEFAULT_VISIBLE_ROWS).cloned().collect()
    };
    let empty_title = empty_title.unwrap_or_else(|| format!("No {title} yet"));
    let empty_message = empty_message.unwrap_or_else(|| {
        "Nothing matched this range. Try a wider window or clear filters.".to_string()
    });

    let body = rsx! {
        if total == 0 {
            EmptyState {
                title: empty_title,
                message: empty_message,
                compact: true,
            }
        } else {
            div { class: "overflow-x-auto -mx-1 px-1",
                table { class: "w-full min-w-[16rem] border-collapse tabular-nums",
                    caption { class: "sr-only", "{title}" }
                    thead {
                        tr {
                            th {
                                scope: "col",
                                class: "text-muted-1 font-semibold text-left px-2 sm:px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                "{value_header}"
                            }
                            th {
                                scope: "col",
                                class: "text-muted-1 font-semibold text-left px-2 sm:px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                "{count_header}"
                            }
                            if has_spark {
                                th {
                                    scope: "col",
                                    class: "text-muted-1 font-semibold text-left px-2 sm:px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                    "Trend"
                                }
                            }
                            th {
                                scope: "col",
                                class: "text-muted-1 font-semibold text-left px-2 sm:px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                "Share"
                            }
                        }
                    }
                    tbody {
                        for row in visible {
                            {
                                let filter_label = format!("Filter by {}", row.value);
                                rsx! {
                                    tr { key: "{row.value}",
                                        td { class: "max-w-[12rem] sm:max-w-[220px] overflow-hidden text-ellipsis whitespace-nowrap px-2 sm:px-2.5 py-[11px] border-t border-border-1 text-text-2",
                                            if let Some(handler) = &on_filter {
                                                button {
                                                    class: "text-text-2 hover:text-teal-hi hover:underline underline-offset-2 text-left cursor-pointer bg-transparent border-0 p-0",
                                                    r#type: "button",
                                                    "aria-label": "{filter_label}",
                                                    onclick: {
                                                        let handler = *handler;
                                                        let value = row.value.clone();
                                                        move |_| handler.call(value.clone())
                                                    },
                                                    "{row.value}"
                                                }
                                            } else {
                                                "{row.value}"
                                            }
                                        }
                                        td { class: "px-2 sm:px-2.5 py-[11px] border-t border-border-1 text-text-2 whitespace-nowrap", "{row.count}" }
                                        if has_spark {
                                            td { class: "px-2 sm:px-2.5 py-[11px] border-t border-border-1 text-text-2",
                                                if let Some(points) = row.spark.clone() {
                                                    Sparkline { points }
                                                }
                                            }
                                        }
                                        td { class: "px-2 sm:px-2.5 py-[11px] border-t border-border-1 text-text-2",
                                            div { class: "flex items-center gap-1.5 sm:gap-2.5 min-w-[4.5rem]",
                                                div {
                                                    class: "h-1.5 rounded-full min-w-[2px] bg-grad-bar",
                                                    style: "width: {row.pct.min(100.0)}%",
                                                    "aria-hidden": "true",
                                                }
                                                span { class: "text-muted-1 text-xs whitespace-nowrap", "{row.pct:.1}%" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if total > DEFAULT_VISIBLE_ROWS {
                {
                    let label = if expanded() {
                        "Show less".to_string()
                    } else {
                        format!("Show all ({total})")
                    };
                    rsx! {
                        button {
                            class: "mt-3 w-full text-center text-[12px] font-semibold text-muted-1 hover:text-teal-hi cursor-pointer bg-transparent border-0 py-1.5",
                            r#type: "button",
                            "aria-expanded": if expanded() { "true" } else { "false" },
                            onclick: move |_| expanded.set(!expanded()),
                            "{label}"
                        }
                    }
                }
            }
        }
    };

    if framed {
        rsx! {
            Card {
                SectionHeader { title, csv_href }
                {body}
            }
        }
    } else {
        body
    }
}

/// A row for [`EntryExitTable`]: same value/count/pct shape as
/// [`BreakdownRow`] plus a bounce/exit rate column.
#[derive(Clone, PartialEq, Debug)]
pub struct EntryExitRow {
    pub value: String,
    pub count: u64,
    pub pct: f64,
    pub rate: f64,
}

/// Entry/exit pages table, port of the Entry Pages / Exit Pages cards
/// (site.jinja lines ~243-329): same shape as [`BreakdownTable`] with an
/// extra bounce-rate / exit-rate column.
#[component]
pub fn EntryExitTable(
    title: String,
    rate_header: String,
    rows: Vec<EntryExitRow>,
    csv_href: Option<String>,
    #[props(default = true)] framed: bool,
    empty_title: Option<String>,
    empty_message: Option<String>,
) -> Element {
    let mut expanded = use_signal(|| false);
    let total = rows.len();
    let visible: Vec<EntryExitRow> = if expanded() || total <= DEFAULT_VISIBLE_ROWS {
        rows.clone()
    } else {
        rows.iter().take(DEFAULT_VISIBLE_ROWS).cloned().collect()
    };
    let empty_title = empty_title.unwrap_or_else(|| format!("No {title} yet"));
    let empty_message = empty_message.unwrap_or_else(|| {
        "Nothing matched this range. Try a wider window or clear filters.".to_string()
    });

    let body = rsx! {
        if total == 0 {
            EmptyState {
                title: empty_title,
                message: empty_message,
                compact: true,
            }
        } else {
            div { class: "overflow-x-auto -mx-1 px-1",
                table { class: "w-full min-w-[16rem] border-collapse tabular-nums",
                    caption { class: "sr-only", "{title}" }
                    thead {
                        tr {
                            th {
                                scope: "col",
                                class: "text-muted-1 font-semibold text-left px-2 sm:px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                "Page"
                            }
                            th {
                                scope: "col",
                                class: "text-muted-1 font-semibold text-left px-2 sm:px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                "Visits"
                            }
                            th {
                                scope: "col",
                                class: "text-muted-1 font-semibold text-left px-2 sm:px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                "{rate_header}"
                            }
                        }
                    }
                    tbody {
                        for row in visible {
                            tr { key: "{row.value}",
                                td { class: "max-w-[12rem] sm:max-w-[220px] overflow-hidden text-ellipsis whitespace-nowrap px-2 sm:px-2.5 py-[11px] border-t border-border-1 text-text-2",
                                    "{row.value}"
                                }
                                td { class: "px-2 sm:px-2.5 py-[11px] border-t border-border-1 text-text-2 whitespace-nowrap", "{row.count}" }
                                td { class: "px-2 sm:px-2.5 py-[11px] border-t border-border-1 text-text-2 whitespace-nowrap", "{row.rate:.1}%" }
                            }
                        }
                    }
                }
            }
            if total > DEFAULT_VISIBLE_ROWS {
                {
                    let label = if expanded() {
                        "Show less".to_string()
                    } else {
                        format!("Show all ({total})")
                    };
                    rsx! {
                        button {
                            class: "mt-3 w-full text-center text-[12px] font-semibold text-muted-1 hover:text-teal-hi cursor-pointer bg-transparent border-0 py-1.5",
                            r#type: "button",
                            "aria-expanded": if expanded() { "true" } else { "false" },
                            onclick: move |_| expanded.set(!expanded()),
                            "{label}"
                        }
                    }
                }
            }
        }
    };

    if framed {
        rsx! {
            Card {
                SectionHeader { title, csv_href }
                {body}
            }
        }
    } else {
        body
    }
}
