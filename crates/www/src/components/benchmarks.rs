//! Sample resource numbers for marketing. Source of truth: repo `BENCHMARKS.md`.
//! Figures are illustrative (one host, release binary); always re-run the harness.

use dioxus::prelude::*;

/// Methodology and full table in the open-source repo.
pub const BENCHMARKS_URL: &str = "https://github.com/kkir/stomatopod/blob/main/BENCHMARKS.md";

#[derive(Clone, Copy)]
struct Stat {
    value: &'static str,
    label: &'static str,
    hint: &'static str,
}

const STATS: &[Stat] = &[
    Stat {
        value: "~40 MiB",
        label: "RSS idle",
        hint: "After boot, no traffic",
    },
    Stat {
        value: "~80-100 MiB",
        label: "Under load",
        hint: "History + light dashboard use",
    },
    Stat {
        value: "~1000/s",
        label: "Ingest headroom",
        hint: "Pageviews sustained in sample",
    },
    Stat {
        value: "1 process",
        label: "No sidecar DB",
        hint: "SQLite + WAL + Parquet on disk",
    },
];

/// Compact metric strip for home and features. Links out to BENCHMARKS.md.
#[component]
pub fn BenchmarkStrip() -> Element {
    rsx! {
        section {
            class: "border-y border-border-1 bg-bg-2/40",
            div { class: "mx-auto max-w-5xl px-4 sm:px-6 py-10 sm:py-12",
                div { class: "flex flex-col sm:flex-row sm:items-end sm:justify-between gap-3 mb-6",
                    div {
                        p { class: "text-teal-hi text-[12px] font-semibold uppercase tracking-[0.16em] mb-2",
                            "Footprint"
                        }
                        h2 { class: "font-display font-bold text-xl sm:text-2xl tracking-tight text-text-1",
                            "Light enough to co-host"
                        }
                        p { class: "mt-2 text-muted-1 text-[14px] max-w-xl leading-relaxed",
                            "Run analytics on a small VPS or the same machine as the product you measure. One binary, embedded storage, no external database."
                        }
                    }
                    a {
                        class: "text-[13px] font-semibold text-teal-hi hover:underline no-underline shrink-0",
                        href: "{BENCHMARKS_URL}",
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "How we measure →"
                    }
                }
                div { class: "grid grid-cols-2 lg:grid-cols-4 gap-3 sm:gap-4",
                    for stat in STATS {
                        div {
                            class: "rounded-xl border border-border-1 bg-surface-1 px-3.5 py-3.5 sm:px-4 sm:py-4 shadow-sm shadow-inner-hi",
                            p { class: "font-display font-bold text-[1.35rem] sm:text-[1.5rem] tracking-tight text-text-1 tabular-nums leading-none",
                                "{stat.value}"
                            }
                            p { class: "mt-2 text-text-1 text-[13px] font-semibold tracking-tight",
                                "{stat.label}"
                            }
                            p { class: "mt-0.5 text-muted-1 text-[12px] leading-snug",
                                "{stat.hint}"
                            }
                        }
                    }
                }
                p { class: "mt-4 text-muted-1 text-[12px] leading-relaxed max-w-3xl",
                    "Sample on Apple Silicon (release build): ~40 MiB idle RSS; ~80-100 MiB with ~25k events and light queries; ~1000 pageview RPS with peak RSS around ~120 MiB. Host-dependent - re-run "
                    code { class: "text-text-2 text-[11.5px] font-mono", "mise run bench:memory" }
                    " for your box. Full table in "
                    a {
                        class: "text-teal-hi hover:underline",
                        href: "{BENCHMARKS_URL}",
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "BENCHMARKS.md"
                    }
                    "."
                }
            }
        }
    }
}
