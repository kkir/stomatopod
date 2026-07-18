use dioxus::prelude::*;

use crate::components::benchmarks::{BenchmarkStrip, BENCHMARKS_URL};
use crate::components::feature_previews::{FeaturePreview, FeaturePreviewPane};
use crate::routes::Route;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Feature {
    title: &'static str,
    body: &'static str,
    /// `None` for text-only cards (no mock UI).
    preview: Option<FeaturePreview>,
}

/// Left column of the masonry stack (taller mocks + one short card).
const LEFT: &[Feature] = &[
    Feature {
        title: "Browser tracker",
        body: "Drop-in /tracker.js for pageviews and custom events. Server-side ingest when you prefer not to run script in the browser.",
        preview: Some(FeaturePreview::Tracker),
    },
    Feature {
        title: "Funnels",
        body: "Multi-step conversion paths with step filters. Define and inspect funnels from the dashboard or API.",
        preview: Some(FeaturePreview::Funnels),
    },
    Feature {
        title: "REST API + OpenAPI",
        body: "JSON at /api/v1 with OpenAPI at /openapi.json. Script workflows or wire agents against a stable contract.",
        preview: Some(FeaturePreview::Api),
    },
    Feature {
        title: "Embedded storage",
        body: "SQLite metadata, WAL, and Parquet partitions on a local volume. No Postgres or managed DB required.",
        preview: None,
    },
    Feature {
        title: "Lightweight footprint",
        body: "Sample ~40 MiB RSS idle and ~80-100 MiB under moderate load, with headroom around ~1000 pageview RPS in the same process. Co-host with your app or run on a small VPS.",
        preview: None,
    },
];

/// Right column (remaining mocks + short card so columns balance).
const RIGHT: &[Feature] = &[
    Feature {
        title: "Dashboard",
        body: "Dashboard UI for pageviews, funnels, campaigns, alerts, and digests - same design system as this site.",
        preview: Some(FeaturePreview::Dashboard),
    },
    Feature {
        title: "Alerts & digests",
        body: "Threshold alerts to webhooks or Slack, plus scheduled digests so you catch changes without living in the UI.",
        preview: Some(FeaturePreview::Alerts),
    },
    Feature {
        title: "stoma CLI",
        body: "Query pageviews, events, and funnels from the terminal. Built for humans and automation alike.",
        preview: Some(FeaturePreview::Cli),
    },
    Feature {
        title: "MIT license",
        body: "Open source. Fork it, self-host it, or embed the library crates in a larger product.",
        preview: None,
    },
];

#[component]
pub fn Features() -> Element {
    rsx! {
        // Intro
        div { class: "mx-auto max-w-6xl px-3 sm:px-5 py-10 sm:py-14 pb-8 sm:pb-10",
            p { class: "text-teal-hi text-[12px] font-semibold uppercase tracking-[0.16em] mb-3",
                "Product"
            }
            h1 { class: "font-display font-bold text-3xl sm:text-4xl tracking-tight text-text-1",
                "Everything you need to understand traffic"
            }
            p { class: "mt-3 text-text-2 text-[15px] leading-relaxed max-w-2xl",
                "Stomatopod is a privacy-friendly analytics appliance: cookieless collection, self-hosted storage, a small process footprint, and tools for both dashboards and agents."
            }
        }

        // Resource benchmarks - full-bleed strip
        BenchmarkStrip {}

        div { class: "mx-auto max-w-6xl px-3 sm:px-5 py-10 sm:py-14",
            // Two independent stacks side by side = reliable masonry-style
            // packing (CSS multi-column / grid-masonry were not applying).
            div { class: "grid grid-cols-1 md:grid-cols-2 gap-5 items-start",
                FeatureColumn { features: LEFT }
                FeatureColumn { features: RIGHT }
            }

            // Methodology callout
            aside {
                class: "mt-8 rounded-xl border border-border-1 bg-surface-1 px-4 py-4 sm:px-5 sm:py-5 shadow-sm shadow-inner-hi",
                h2 { class: "text-text-1 text-[15px] font-semibold tracking-tight",
                    "Resource numbers are measured, not guessed"
                }
                p { class: "mt-2 text-muted-1 text-[13.5px] leading-relaxed max-w-3xl",
                    "We run a release binary through an idle / warm / ~10 / ~100 / ~1000 pageview-RPS ladder and sample process RSS. Reproduce with "
                    code { class: "text-text-2 text-[12px] font-mono", "mise run bench:memory" }
                    " or read "
                    a {
                        class: "text-teal-hi hover:underline",
                        href: "{BENCHMARKS_URL}",
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "BENCHMARKS.md"
                    }
                    " for the sample table and methodology."
                }
            }

            div { class: "mt-10 flex flex-wrap gap-3",
                Link {
                    class: "inline-flex items-center px-[18px] py-2.5 rounded-[10px] text-[14px] font-semibold bg-grad-btn text-[#032621] shadow-glow no-underline",
                    to: Route::GetStarted {},
                    "Get started"
                }
                Link {
                    class: "inline-flex items-center px-[18px] py-2.5 rounded-[10px] text-[14px] font-semibold border border-border-2 text-text-1 no-underline",
                    to: Route::Home {},
                    "Back home"
                }
            }
        }
    }
}

#[component]
fn FeatureColumn(features: &'static [Feature]) -> Element {
    rsx! {
        div { class: "flex flex-col gap-5 min-w-0",
            for feature in features {
                FeatureCard {
                    key: "{feature.title}",
                    title: feature.title,
                    body: feature.body,
                    preview: feature.preview,
                }
            }
        }
    }
}

#[component]
fn FeatureCard(
    title: &'static str,
    body: &'static str,
    preview: Option<FeaturePreview>,
) -> Element {
    rsx! {
        article {
            class: "rounded-xl border border-border-1 bg-surface-1 shadow-sm shadow-inner-hi overflow-hidden",
            if let Some(kind) = preview {
                FeaturePreviewPane { kind }
                div { class: "px-4 py-3 sm:px-5 sm:py-3.5 border-t border-border-1",
                    h2 { class: "text-text-1 text-[16px] font-semibold tracking-tight",
                        "{title}"
                    }
                    p { class: "mt-1 text-muted-1 text-[13.5px] leading-relaxed",
                        "{body}"
                    }
                }
            } else {
                div { class: "px-4 py-4 sm:px-5 sm:py-5",
                    h2 { class: "inline-flex items-center gap-2.5 text-text-1 text-[16px] font-semibold tracking-tight",
                        span {
                            class: "inline-block w-[3px] h-3.5 rounded-sm bg-iri shadow-[0_0_8px_rgba(45,212,191,0.4)]",
                            "aria-hidden": "true",
                        }
                        "{title}"
                    }
                    p { class: "mt-2 text-muted-1 text-[13.5px] leading-relaxed",
                        "{body}"
                    }
                }
            }
        }
    }
}
