//! Scaled-down feature previews that mirror real dashboard UI patterns
//! (`StatTile`, `Card` / iridescent section marks, funnel bars, install snippet,
//! range pills, breakdown bars, form controls).

use dioxus::prelude::*;
use stomatopod_ui::card::Card;
use stomatopod_ui::stat::{DeltaDir, DeltaInfo, StatTile};

use super::browser_mock::WindowFrame;

// Dashboard control classes, with cursor-default so mocks never look live.
const BTN_PRIMARY: &str = "inline-flex items-center gap-1.5 px-[15px] py-2 rounded-[10px] text-[13px] font-semibold tracking-tight cursor-default bg-grad-btn text-[#032621] shadow-glow";
const BTN_GHOST: &str = "inline-flex items-center gap-1.5 px-[13px] py-1.5 rounded-[10px] text-[12px] font-semibold tracking-tight cursor-default bg-text-1/3 text-text-2 border border-border-2 shadow-inner-hi";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FeaturePreview {
    Tracker,
    Dashboard,
    Funnels,
    Alerts,
    Api,
    Cli,
}

#[component]
pub fn FeaturePreviewPane(kind: FeaturePreview) -> Element {
    // Keep previews inside a single rsx! tree (do not build Element outside
    // and interpolate) - intermediate Elements re-enter the runtime and can
    // panic with "RefCell already borrowed" during hydration.
    rsx! {
        WindowFrame {
            div {
                class: "origin-top-left scale-[0.62] w-[161%] h-[161%] min-w-[42rem] pointer-events-none cursor-default",
                match kind {
                    FeaturePreview::Tracker => rsx! { TrackerPreview {} },
                    FeaturePreview::Dashboard => rsx! { DashboardPreview {} },
                    FeaturePreview::Funnels => rsx! { FunnelsPreview {} },
                    FeaturePreview::Alerts => rsx! { AlertsPreview {} },
                    FeaturePreview::Api => rsx! { ApiPreview {} },
                    FeaturePreview::Cli => rsx! { CliPreview {} },
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Previews
// ---------------------------------------------------------------------------

/// Install-tracker card (mirrors `InstallCard` structure).
#[component]
fn TrackerPreview() -> Element {
    rsx! {
        div { class: "p-3 bg-bg h-full",
            Card { title: "Install the tracker".to_string(),
                p { class: "text-muted-1 text-[12.5px] leading-relaxed mb-3",
                    "Paste this snippet into the "
                    code { class: "bg-black/40 rounded px-1 py-0.5 text-[12px] font-mono text-text-2", "<head>" }
                    " of every page on "
                    span { class: "text-text-2 font-medium", "example.com" }
                    "."
                }
                pre {
                    class: "bg-black/40 border border-border-1 rounded-lg p-3 overflow-hidden mb-3",
                    code { class: "text-[12px] font-mono text-teal-hi break-all whitespace-pre-wrap",
                        "<script defer src=\"https://app.local/tracker.js\" data-site=\"pk_demo…\"></script>"
                    }
                }
                div { class: "flex flex-wrap items-center gap-2",
                    span { class: "{BTN_GHOST}", "Copy" }
                    span { class: "text-muted-1 text-[12px]", "Docs →" }
                }
            }
        }
    }
}

/// Site overview: sidebar shell + stats + chart (dashboard layout).
#[component]
fn DashboardPreview() -> Element {
    rsx! {
        div { class: "h-full grid grid-cols-[132px_minmax(0,1fr)] bg-bg font-ui text-text-1",
            // Sidebar (layout::Sidebar)
            aside { class: "flex flex-col gap-0 pt-3 pb-2 px-2.5 bg-bg-2/65 border-r border-border-1",
                div { class: "inline-flex items-center gap-2 px-1.5 font-display font-bold text-[14px] tracking-tight",
                    span { class: "logo-mark bg-iri inline-block h-3 w-3 rounded-sm" }
                    span { "Stomatopod" }
                }
                span { class: "text-[10.5px] font-semibold uppercase tracking-wider text-muted-2 mx-1.5 mt-4 mb-1",
                    "Analytics"
                }
                nav { class: "flex flex-col gap-0.5",
                    span { class: "relative px-2.5 py-1.5 rounded-[10px] text-[13px] font-medium text-text-1 bg-teal-soft before:content-[''] before:absolute before:left-0 before:top-1.5 before:bottom-1.5 before:w-[3px] before:rounded-full before:bg-iri",
                        "Sites"
                    }
                    span { class: "px-2.5 py-1.5 rounded-[10px] text-[13px] font-medium text-muted-1", "API Keys" }
                    span { class: "px-2.5 py-1.5 rounded-[10px] text-[13px] font-medium text-muted-1", "Docs" }
                }
            }
            main { class: "min-w-0 p-3 overflow-hidden",
                // PageHead + RangeTabs
                div { class: "flex justify-between items-center gap-3 mb-3",
                    h1 { class: "text-[18px] font-bold tracking-tight text-text-1 leading-tight", "example.com" }
                    nav {
                        class: "inline-flex items-center h-8 gap-0.5 p-[3px] rounded-[11px] bg-surface-2/80 border border-border-1 shadow-inner-hi",
                        span { class: "inline-flex items-center justify-center h-full px-2.5 rounded-[8px] text-[11px] font-semibold text-muted-1", "7d" }
                        span { class: "inline-flex items-center justify-center h-full px-2.5 rounded-[8px] text-[11px] font-semibold bg-grad-btn text-[#032621]", "30d" }
                        span { class: "inline-flex items-center justify-center h-full px-2.5 rounded-[8px] text-[11px] font-semibold text-muted-1", "90d" }
                    }
                }
                // Hero stats (StatTile)
                div { class: "grid grid-cols-3 gap-3 mb-3",
                    StatTile {
                        label: "Pageviews".to_string(),
                        value: "12.4k".to_string(),
                        delta: Some(DeltaInfo { dir: DeltaDir::Up, pct: 12.4 }),
                        prev: None,
                    }
                    StatTile {
                        label: "Visitors".to_string(),
                        value: "3.1k".to_string(),
                        delta: Some(DeltaInfo { dir: DeltaDir::Up, pct: 4.2 }),
                        prev: None,
                    }
                    StatTile {
                        label: "Bounce rate".to_string(),
                        value: "41%".to_string(),
                        delta: Some(DeltaInfo { dir: DeltaDir::Down, pct: 2.1 }),
                        prev: None,
                    }
                }
                // Timeseries-ish SVG (same teal/cyan series colors as chart)
                Card {
                    svg {
                        class: "w-full h-[96px] block",
                        view_box: "0 0 400 96",
                        preserve_aspect_ratio: "none",
                        polyline {
                            fill: "none",
                            stroke: "#2dd4bf",
                            stroke_width: "2",
                            points: "0,70 40,64 80,52 120,58 160,36 200,42 240,24 280,30 320,18 360,26 400,14",
                        }
                        polyline {
                            fill: "none",
                            stroke: "#22d3ee",
                            stroke_width: "1.5",
                            stroke_dasharray: "4 3",
                            opacity: "0.7",
                            points: "0,78 40,74 80,66 120,70 160,54 200,58 240,46 280,50 320,40 360,44 400,36",
                        }
                    }
                }
            }
        }
    }
}

/// Funnel detail bars (mirrors `FunnelBars` layout).
#[component]
fn FunnelsPreview() -> Element {
    let steps: [(&str, f64, u64); 3] = [
        ("Land", 1.0, 4200),
        ("Signup", 0.62, 2604),
        ("Activate", 0.28, 1176),
    ];
    rsx! {
        div { class: "p-3 bg-bg h-full",
            div { class: "flex justify-between items-center mb-3",
                h1 { class: "text-[18px] font-bold tracking-tight text-text-1", "Funnel" }
                nav {
                    class: "inline-flex items-center h-8 gap-0.5 p-[3px] rounded-[11px] bg-surface-2/80 border border-border-1 shadow-inner-hi",
                    span { class: "inline-flex items-center justify-center h-full px-2.5 rounded-[8px] text-[11px] font-semibold bg-grad-btn text-[#032621]", "30d" }
                    span { class: "inline-flex items-center justify-center h-full px-2.5 rounded-[8px] text-[11px] font-semibold text-muted-1", "90d" }
                }
            }
            Card {
                div { class: "flex items-end gap-4 min-h-[180px] pt-2",
                    for (name, rate, sessions) in steps {
                        div { class: "flex-1 min-w-0 flex flex-col items-center justify-end gap-1.5",
                            div { class: "text-teal-hi text-[13px] font-semibold tabular-nums",
                                "{(rate * 100.0) as i32}%"
                            }
                            div {
                                class: "w-full max-w-[88px] rounded-t-md bg-grad-bar min-h-[3px]",
                                style: "height: {(rate * 160.0) as i64}px",
                            }
                            div { class: "text-text-1 text-[13px] font-medium text-center", "{name}" }
                            div { class: "text-muted-1 text-xs tabular-nums", "{sessions}" }
                        }
                    }
                }
            }
        }
    }
}

/// Alerts list rows (mirrors alerts page list rows).
#[component]
fn AlertsPreview() -> Element {
    rsx! {
        div { class: "p-3 bg-bg h-full",
            div { class: "flex justify-between items-center mb-3",
                h1 { class: "text-[18px] font-bold tracking-tight text-text-1", "Alerts" }
                span { class: "{BTN_PRIMARY}", "+ New alert" }
            }
            Card {
                div { class: "flex flex-col",
                    AlertListRow {
                        name: "Traffic drop",
                        detail: "pageviews < 500 · 60m",
                        badge: "Firing",
                        badge_class: "text-red",
                    }
                    AlertListRow {
                        name: "Error spike",
                        detail: "events error > 50 · 15m",
                        badge: "OK",
                        badge_class: "text-muted-1",
                    }
                    AlertListRow {
                        name: "Signups",
                        detail: "event signup · daily digest",
                        badge: "OK",
                        badge_class: "text-muted-1",
                    }
                }
            }
        }
    }
}

#[component]
fn AlertListRow(
    name: &'static str,
    detail: &'static str,
    badge: &'static str,
    badge_class: &'static str,
) -> Element {
    rsx! {
        div { class: "flex items-center justify-between gap-3 py-2.5 border-t border-border-1 first:border-t-0",
            div { class: "min-w-0",
                div { class: "text-text-1 text-[13px] font-medium",
                    "{name}"
                    span { class: "ml-2 text-[11px] font-semibold uppercase tracking-wide {badge_class}",
                        "{badge}"
                    }
                }
                div { class: "text-muted-1 text-xs", "{detail}" }
            }
            div { class: "flex items-center gap-2 shrink-0",
                span { class: "{BTN_GHOST}", "Edit" }
                span { class: "{BTN_GHOST}", "Delete" }
            }
        }
    }
}

/// API / OpenAPI-style JSON with dashboard card chrome.
#[component]
fn ApiPreview() -> Element {
    rsx! {
        div { class: "p-3 bg-bg h-full",
            Card { title: "GET /api/v1/sites/…/pageviews".to_string(),
                pre {
                    class: "bg-black/40 border border-border-1 rounded-lg p-3 overflow-hidden m-0",
                    code { class: "text-[12px] font-mono text-text-2 whitespace-pre",
                        "{{\n  \"total\": 12402,\n  \"visitors\": 3104,\n  \"range\": \"30d\",\n  \"series\": […]\n}}"
                    }
                }
                div { class: "mt-3 flex flex-wrap gap-2",
                    span { class: "{BTN_GHOST}", "/openapi.json" }
                    span { class: "{BTN_GHOST}", "Copy curl" }
                }
            }
        }
    }
}

/// CLI output in a card (mono, same as install/docs code blocks).
#[component]
fn CliPreview() -> Element {
    rsx! {
        div { class: "p-3 bg-bg h-full",
            Card { title: "stoma CLI".to_string(),
                pre {
                    class: "bg-black/40 border border-border-1 rounded-lg p-3 overflow-hidden m-0 font-mono text-[12px] leading-relaxed text-text-2",
                    span { class: "text-teal-hi", "$ " }
                    "stoma query pageviews --days 7\n"
                    "\n"
                    "date        views   visitors\n"
                    "──────────  ──────  ────────\n"
                    "2026-07-14   1,842     512\n"
                    "2026-07-13   1,701     488\n"
                    "2026-07-12   1,956     540"
                }
            }
        }
    }
}
