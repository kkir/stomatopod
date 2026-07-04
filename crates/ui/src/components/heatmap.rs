use dioxus::prelude::*;

use super::card::EmptyState;
use crate::types::RetentionGrid;

/// Weekly-cohort retention heatmap, port of the `.retention-grid` table in
/// the legacy retention.jinja (lines ~33-62). Each cell keeps the literal
/// `ret-cell` class and sets `--p` inline so the `color-mix` rule in
/// tailwind.css tints it by return rate.
#[component]
pub fn RetentionHeatmap(grid: RetentionGrid) -> Element {
    if grid.cohorts.is_empty() {
        return rsx! {
            EmptyState { message: "No cohort data yet" }
        };
    }
    let offsets: Vec<usize> = (0..=grid.max_offset).collect();
    rsx! {
        div { class: "overflow-x-auto",
            table { class: "w-full border-collapse tabular-nums text-[13px]",
                thead {
                    tr {
                        th { class: "text-muted-1 font-semibold text-left px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                            "Cohort"
                        }
                        th { class: "text-muted-1 font-semibold text-left px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                            "Sessions"
                        }
                        for k in offsets.iter().copied() {
                            th {
                                key: "h{k}",
                                class: "text-muted-1 font-semibold text-center px-2.5 py-2 text-[10.5px] uppercase tracking-[0.1em] border-b border-border-2",
                                "+{k}w"
                            }
                        }
                    }
                }
                tbody {
                    for c in grid.cohorts.iter() {
                        tr { key: "{c.week}",
                            td { class: "px-2.5 py-[9px] border-t border-border-1 text-text-2 whitespace-nowrap",
                                "{c.week}"
                            }
                            td { class: "px-2.5 py-[9px] border-t border-border-1 text-text-2", "{c.size}" }
                            for (k , cell) in c.cells.iter().enumerate() {
                                td {
                                    key: "{c.week}-{k}",
                                    class: "ret-cell px-2.5 py-[9px] border-t border-border-1 text-text-1",
                                    style: "--p:{cell.pct.min(100.0)}%",
                                    title: "{cell.returning} sessions",
                                    "{cell.pct:.0}%"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
