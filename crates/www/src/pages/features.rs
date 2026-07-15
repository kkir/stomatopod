use dioxus::prelude::*;
use stomatopod_ui::card::Card;

use crate::routes::Route;

struct Feature {
    title: &'static str,
    body: &'static str,
}

const FEATURES: &[Feature] = &[
    Feature {
        title: "Browser tracker",
        body: "Drop-in /tracker.js for pageviews and custom events. Server-side ingest when you prefer not to run script in the browser.",
    },
    Feature {
        title: "Dashboard",
        body: "Dioxus fullstack UI for pageviews, funnels, campaigns, alerts, and digests - same design system as this site.",
    },
    Feature {
        title: "Funnels",
        body: "Multi-step conversion paths with step filters. Define and inspect funnels from the dashboard or API.",
    },
    Feature {
        title: "Alerts & digests",
        body: "Threshold alerts to webhooks or Slack, plus scheduled digests so you catch changes without living in the UI.",
    },
    Feature {
        title: "REST API + OpenAPI",
        body: "JSON at /api/v1 with OpenAPI at /openapi.json. Script workflows or wire agents against a stable contract.",
    },
    Feature {
        title: "stoma CLI",
        body: "Query pageviews, events, and funnels from the terminal. Built for humans and automation alike.",
    },
    Feature {
        title: "Embedded storage",
        body: "SQLite metadata, WAL, and Parquet partitions on a local volume. No Postgres or managed DB required.",
    },
    Feature {
        title: "Single-owner appliance",
        body: "One org, one admin user, many sites. First boot bootstraps ownership; cookie auth for the dashboard.",
    },
    Feature {
        title: "MIT license",
        body: "Open source. Fork it, self-host it, or embed the library crates in a larger product.",
    },
];

#[component]
pub fn Features() -> Element {
    rsx! {
        div { class: "mx-auto max-w-5xl px-4 sm:px-6 py-12 sm:py-16",
            p { class: "text-teal-hi text-[12px] font-semibold uppercase tracking-[0.16em] mb-3",
                "Product"
            }
            h1 { class: "font-display font-bold text-3xl sm:text-4xl tracking-tight text-text-1",
                "Everything you need to understand traffic"
            }
            p { class: "mt-4 text-text-2 text-[15px] leading-relaxed max-w-2xl",
                "Stomatopod is a privacy-friendly analytics appliance: cookieless collection, self-hosted storage, and tools for both dashboards and agents."
            }

            div { class: "mt-10 grid gap-4 sm:grid-cols-2 lg:grid-cols-3",
                for feature in FEATURES {
                    Card {
                        h2 { class: "text-text-1 text-[15px] font-semibold tracking-tight mb-2",
                            "{feature.title}"
                        }
                        p { class: "text-muted-1 text-[13px] leading-relaxed", "{feature.body}" }
                    }
                }
            }

            div { class: "mt-12 flex flex-wrap gap-3",
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
