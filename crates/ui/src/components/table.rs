use dioxus::prelude::*;

use super::card::{Card, EmptyState, SectionHeader};
use super::chart::Sparkline;

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
) -> Element {
    let has_spark = rows.iter().any(|r| r.spark.is_some());
    rsx! {
        Card {
            SectionHeader { title, csv_href }
            if rows.is_empty() {
                EmptyState { message: "No data" }
            } else {
                table { class: "w-full border-collapse tabular-nums",
                    thead {
                        tr {
                            th { class: "text-muted-1 font-semibold text-left px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                "{value_header}"
                            }
                            th { class: "text-muted-1 font-semibold text-left px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                "{count_header}"
                            }
                            if has_spark {
                                th { class: "text-muted-1 font-semibold text-left px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                    "Trend"
                                }
                            }
                            th { class: "text-muted-1 font-semibold text-left px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                "%"
                            }
                        }
                    }
                    tbody {
                        for row in rows {
                            tr { key: "{row.value}",
                                td { class: "max-w-[220px] overflow-hidden text-ellipsis whitespace-nowrap px-2.5 py-[11px] border-t border-border-1 text-text-2",
                                    if let Some(handler) = &on_filter {
                                        button {
                                            class: "text-text-2 hover:text-teal-hi hover:underline underline-offset-2 text-left cursor-pointer bg-transparent border-0 p-0",
                                            r#type: "button",
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
                                td { class: "px-2.5 py-[11px] border-t border-border-1 text-text-2", "{row.count}" }
                                if has_spark {
                                    td { class: "px-2.5 py-[11px] border-t border-border-1 text-text-2",
                                        if let Some(points) = row.spark.clone() {
                                            Sparkline { points }
                                        }
                                    }
                                }
                                td { class: "px-2.5 py-[11px] border-t border-border-1 text-text-2",
                                    div { class: "flex items-center gap-2.5",
                                        div {
                                            class: "h-1.5 rounded-full min-w-[2px] bg-grad-bar",
                                            style: "width: {row.pct.min(100.0)}%",
                                        }
                                        span { class: "text-muted-1 text-xs", "{row.pct:.1}%" }
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
) -> Element {
    rsx! {
        Card {
            SectionHeader { title, csv_href }
            if rows.is_empty() {
                EmptyState { message: "No data" }
            } else {
                table { class: "w-full border-collapse tabular-nums",
                    thead {
                        tr {
                            th { class: "text-muted-1 font-semibold text-left px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                "Page"
                            }
                            th { class: "text-muted-1 font-semibold text-left px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                "Visits"
                            }
                            th { class: "text-muted-1 font-semibold text-left px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                "{rate_header}"
                            }
                        }
                    }
                    tbody {
                        for row in rows {
                            tr { key: "{row.value}",
                                td { class: "max-w-[220px] overflow-hidden text-ellipsis whitespace-nowrap px-2.5 py-[11px] border-t border-border-1 text-text-2",
                                    "{row.value}"
                                }
                                td { class: "px-2.5 py-[11px] border-t border-border-1 text-text-2", "{row.count}" }
                                td { class: "px-2.5 py-[11px] border-t border-border-1 text-text-2", "{row.rate:.1}%" }
                            }
                        }
                    }
                }
            }
        }
    }
}
