use dioxus::prelude::*;
use stomatopod_ui::card::Card;

use crate::components::benchmarks::BenchmarkStrip;
use crate::components::mascot::HeroMascot;
use crate::routes::Route;

const GITHUB_URL: &str = "https://github.com/kkir/stomatopod";

#[component]
pub fn Home() -> Element {
    rsx! {
        // Hero
        section {
            class: "relative overflow-hidden border-b border-border-1",
            // Soft brand glow behind the headline
            div {
                class: "pointer-events-none absolute inset-0 -z-0",
                "aria-hidden": "true",
                div {
                    class: "absolute -top-24 left-1/2 h-72 w-[36rem] -translate-x-1/2 rounded-full bg-teal/15 blur-3xl",
                }
                div {
                    class: "absolute bottom-0 right-0 h-56 w-56 rounded-full bg-violet/10 blur-3xl",
                }
            }
            div {
                class: "relative mx-auto max-w-5xl px-4 sm:px-6 pt-12 sm:pt-20 pb-14 sm:pb-20 grid gap-10 lg:gap-12 lg:grid-cols-[minmax(0,1fr)_minmax(14rem,20rem)] items-center",
                div {
                    p { class: "text-teal-hi text-[12px] font-semibold uppercase tracking-[0.16em] mb-4",
                        "Self-hosted analytics"
                    }
                    h1 { class: "font-display font-bold text-[2.25rem] sm:text-5xl lg:text-[3.25rem] tracking-tight text-text-1 max-w-2xl leading-[1.1]",
                        "Privacy-friendly web analytics you run yourself"
                    }
                    p { class: "mt-5 text-text-2 text-base sm:text-lg leading-relaxed max-w-xl",
                        "Cookieless tracking, a dashboard, REST API, and CLI - one light binary with embedded storage. Co-host on a small VPS or the same machine as your product. MIT licensed."
                    }
                    div { class: "mt-8 flex flex-wrap items-center gap-3",
                        Link {
                            class: "inline-flex items-center gap-1.5 px-[18px] py-2.5 rounded-[10px] text-[14px] font-semibold tracking-tight bg-grad-btn text-[#032621] shadow-glow no-underline hover:-translate-y-px transition-transform",
                            to: Route::GetStarted {},
                            "Get started"
                        }
                        Link {
                            class: "inline-flex items-center gap-1.5 px-[18px] py-2.5 rounded-[10px] text-[14px] font-semibold tracking-tight border border-border-2 text-text-1 no-underline hover:border-border-3 hover:bg-surface-1 transition-colors",
                            to: Route::Features {},
                            "See features"
                        }
                        a {
                            class: "inline-flex items-center gap-1.5 px-3 py-2.5 text-[14px] font-semibold text-muted-1 hover:text-teal-hi no-underline",
                            href: "{GITHUB_URL}",
                            target: "_blank",
                            rel: "noopener noreferrer",
                            "View on GitHub →"
                        }
                    }
                }
                div {
                    class: "flex justify-center lg:justify-end order-first lg:order-none",
                    HeroMascot {}
                }
            }
        }

        // Resource benchmarks (idle / load / RPS)
        BenchmarkStrip {}

        // Value props
        section {
            class: "mx-auto max-w-5xl px-4 sm:px-6 py-14 sm:py-16",
            h2 { class: "font-display font-bold text-xl sm:text-2xl tracking-tight text-text-1 mb-2",
                "Built for operators and agents"
            }
            p { class: "text-muted-1 text-[14px] mb-8 max-w-2xl",
                "A single-owner appliance: one org, one admin, many sites. No external database required."
            }
            div { class: "grid gap-4 sm:grid-cols-2 lg:grid-cols-4",
                ValueCard {
                    title: "Cookieless by design".to_string(),
                    body: "Browser tracker and server-side ingest without tracking cookies. Respect visitor privacy out of the box.".to_string(),
                }
                ValueCard {
                    title: "Lightweight".to_string(),
                    body: "Sample ~40 MiB RSS idle and ~80-100 MiB under moderate load - sized for a small box, not a dedicated analytics fleet.".to_string(),
                }
                ValueCard {
                    title: "One binary".to_string(),
                    body: "Dashboard, JSON API, tracker, and workers in a single process. Mount a volume and go.".to_string(),
                }
                ValueCard {
                    title: "API + CLI".to_string(),
                    body: "OpenAPI, the stoma CLI, and agent-friendly queries for funnels, pageviews, and digests.".to_string(),
                }
            }
        }

        // Operator orientation
        section {
            class: "border-t border-border-1 bg-bg-2/20",
            div { class: "mx-auto max-w-5xl px-4 sm:px-6 py-14 sm:py-16",
                p { class: "text-teal-hi text-[12px] font-semibold uppercase tracking-[0.16em] mb-3",
                    "Compare"
                }
                h2 { class: "font-display font-bold text-xl sm:text-2xl tracking-tight text-text-1",
                    "Plausible-class analytics, appliance ops"
                }
                p { class: "mt-3 text-text-2 text-[15px] leading-relaxed max-w-2xl",
                    "Short orientation for self-hosters weighing privacy analytics tools. Stomatopod is a single binary with embedded storage: one process, mount a volume, co-host on a small box."
                }
                div { class: "mt-6",
                    Link {
                        class: "inline-flex items-center px-[18px] py-2.5 rounded-[10px] text-[14px] font-semibold border border-border-2 text-text-1 no-underline hover:border-border-3 hover:bg-surface-1 transition-colors",
                        to: Route::Compare {},
                        "How it compares →"
                    }
                }
            }
        }

        // Bottom CTA
        section {
            class: "border-t border-border-1 bg-bg-2/30",
            div { class: "mx-auto max-w-5xl px-4 sm:px-6 py-14 text-center",
                h2 { class: "font-display font-bold text-xl sm:text-2xl tracking-tight text-text-1",
                    "Run it on your own hardware"
                }
                p { class: "mt-3 text-muted-1 text-[14px] max-w-lg mx-auto",
                    "Docker Compose or a release binary. Your data stays on your disk - without a heavy analytics stack."
                }
                div { class: "mt-6 flex flex-wrap justify-center gap-3",
                    Link {
                        class: "inline-flex items-center px-[18px] py-2.5 rounded-[10px] text-[14px] font-semibold bg-grad-btn text-[#032621] shadow-glow no-underline",
                        to: Route::GetStarted {},
                        "Install Stomatopod"
                    }
                    a {
                        class: "inline-flex items-center px-[18px] py-2.5 rounded-[10px] text-[14px] font-semibold border border-border-2 text-text-1 no-underline",
                        href: "{GITHUB_URL}",
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "Star on GitHub"
                    }
                }
            }
        }
    }
}

#[component]
fn ValueCard(title: String, body: String) -> Element {
    rsx! {
        Card {
            h3 { class: "text-text-1 text-[15px] font-semibold tracking-tight mb-2", "{title}" }
            p { class: "text-muted-1 text-[13px] leading-relaxed", "{body}" }
        }
    }
}
