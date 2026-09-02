use dioxus::prelude::*;
use stomatopod_ui::card::Card;

use crate::components::benchmarks::BENCHMARKS_URL;
use crate::components::page_head::PageHead;
use crate::routes::Route;
use crate::seo;

const GITHUB_URL: &str = "https://github.com/kkir/stomatopod";
const DEPLOY_URL: &str = "https://github.com/kkir/stomatopod/blob/main/DEPLOY.md";

struct CompareRow {
    label: &'static str,
    stomatopod: &'static str,
    plausible: &'static str,
    umami: &'static str,
    goatcounter: &'static str,
}

/// Same rows as README "Compared to common options". Keep them in lockstep.
const ROWS: &[CompareRow] = &[
    CompareRow {
        label: "Cookieless tracking",
        stomatopod: "Yes",
        plausible: "Yes",
        umami: "Yes",
        goatcounter: "Yes",
    },
    CompareRow {
        label: "External DB required",
        stomatopod: "No (embedded)",
        plausible: "Yes (Postgres/ClickHouse)",
        umami: "Yes (SQL)",
        goatcounter: "SQLite/Postgres",
    },
    CompareRow {
        label: "Process model",
        stomatopod: "Single binary",
        plausible: "App + DB",
        umami: "App + DB",
        goatcounter: "Single binary",
    },
    CompareRow {
        label: "Footprint class",
        stomatopod: "~40 MiB idle (sample)",
        plausible: "Heavier stack",
        umami: "App + DB",
        goatcounter: "Very light",
    },
    CompareRow {
        label: "Multi-site",
        stomatopod: "Yes (one owner)",
        plausible: "Yes",
        umami: "Yes",
        goatcounter: "Yes",
    },
    CompareRow {
        label: "License",
        stomatopod: "MIT",
        plausible: "AGPL",
        umami: "MIT",
        goatcounter: "EUPL / fair use",
    },
    CompareRow {
        label: "API + CLI",
        stomatopod: "REST, OpenAPI, stoma",
        plausible: "API",
        umami: "API",
        goatcounter: "API",
    },
];

#[component]
pub fn Compare() -> Element {
    rsx! {
        PageHead { meta: seo::COMPARE }

        div { class: "mx-auto max-w-5xl px-4 sm:px-6 py-12 sm:py-16",
            p { class: "text-teal-hi text-[12px] font-semibold uppercase tracking-[0.16em] mb-3",
                "Compare"
            }
            h1 { class: "font-display font-bold text-3xl sm:text-4xl tracking-tight text-text-1 max-w-3xl leading-[1.15]",
                "Open source Plausible/Umami alternative on a small VPS"
            }
            p { class: "mt-4 text-text-2 text-[15px] leading-relaxed max-w-2xl",
                "Stomatopod is MIT open source self-hosted analytics. "
                a {
                    class: "text-teal-hi hover:underline",
                    href: "{GITHUB_URL}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    "Source on GitHub"
                }
                ". Short orientation for self-hosters evaluating privacy analytics tools. The goal is to decide whether Stomatopod fits your box, not to rank products."
            }

            // Positioning
            Card {
                title: "Appliance ops, familiar product class".to_string(),
                p { class: "text-[14px] text-text-2 leading-relaxed",
                    "Stomatopod sits in the Plausible/Umami-class category with an appliance ops shape: one process, mount a volume, co-host on a small box. One org, many sites, you own the box. MIT licensed."
                }
                p { class: "mt-3 text-[14px] text-text-2 leading-relaxed",
                    "Cookieless browser tracker and server-side ingest. Dashboard, REST API, OpenAPI, and the "
                    code { class: "bg-black/40 rounded px-1 py-0.5 font-mono text-[12px]", "stoma" }
                    " CLI in a single binary. Storage is embedded (SQLite metadata, WAL, Parquet partitions) so you do not stand up Postgres or Redis for analytics."
                }
                p { class: "mt-3 text-[13px] text-muted-1 leading-relaxed",
                    "Sample idle RSS is about 40 MiB on the checked-in run. See "
                    a {
                        class: "text-teal-hi hover:underline",
                        href: "{BENCHMARKS_URL}",
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "BENCHMARKS.md"
                    }
                    " for the host, methodology, and load ladder. Re-run "
                    code { class: "text-text-2 text-[12px] font-mono", "mise run bench:memory" }
                    " on your machine."
                }
            }

            // Table
            div { class: "mt-8",
                h2 { class: "font-display font-bold text-xl sm:text-2xl tracking-tight text-text-1 mb-2",
                    "Side by side"
                }
                p { class: "text-muted-1 text-[13.5px] leading-relaxed mb-4 max-w-2xl",
                    "Numbers and feature sets move. Verify current docs for peers. Stomatopod figures come from "
                    a {
                        class: "text-teal-hi hover:underline",
                        href: "{BENCHMARKS_URL}",
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "BENCHMARKS.md"
                    }
                    " and the project README."
                }
                div { class: "overflow-x-auto rounded-xl border border-border-1 bg-surface-1 shadow-sm shadow-inner-hi",
                    table { class: "w-full min-w-[40rem] text-left text-[13px] border-collapse",
                        caption { class: "sr-only",
                            "Comparison of Stomatopod, Plausible CE, Umami, and GoatCounter"
                        }
                        thead {
                            tr { class: "border-b border-border-1 text-muted-1 text-[11.5px] uppercase tracking-[0.08em]",
                                th { class: "px-3.5 py-3 sm:px-4 font-semibold", scope: "col", "Criterion" }
                                th { class: "px-3.5 py-3 sm:px-4 font-semibold text-text-1 bg-teal-soft", scope: "col",
                                    "Stomatopod"
                                }
                                th { class: "px-3.5 py-3 sm:px-4 font-semibold", scope: "col", "Plausible CE" }
                                th { class: "px-3.5 py-3 sm:px-4 font-semibold", scope: "col", "Umami" }
                                th { class: "px-3.5 py-3 sm:px-4 font-semibold", scope: "col", "GoatCounter" }
                            }
                        }
                        tbody {
                            for (i, row) in ROWS.iter().enumerate() {
                                tr {
                                    key: "{row.label}",
                                    class: if i + 1 == ROWS.len() {
                                        ""
                                    } else {
                                        "border-b border-border-1"
                                    },
                                    th { class: "px-3.5 py-3 sm:px-4 font-semibold text-text-1 whitespace-nowrap", scope: "row",
                                        "{row.label}"
                                    }
                                    td { class: "px-3.5 py-3 sm:px-4 text-text-1 font-medium bg-teal-soft",
                                        "{row.stomatopod}"
                                    }
                                    td { class: "px-3.5 py-3 sm:px-4 text-text-2", "{row.plausible}" }
                                    td { class: "px-3.5 py-3 sm:px-4 text-text-2", "{row.umami}" }
                                    td { class: "px-3.5 py-3 sm:px-4 text-text-2", "{row.goatcounter}" }
                                }
                            }
                        }
                    }
                }
            }

            // Fit / not-fit
            div { class: "mt-10 grid gap-4 md:grid-cols-2",
                Card {
                    title: "A good fit when".to_string(),
                    ul { class: "list-disc pl-5 space-y-2 text-[13.5px] text-text-2 leading-relaxed",
                        li { "You want cookieless analytics you run yourself, on a small VPS or the same machine as the product." }
                        li { "You would rather not operate Postgres, ClickHouse, or Redis just to count pageviews." }
                        li { "One organization and one admin is enough, with many sites under that owner." }
                        li { "You want a dashboard plus a JSON API, OpenAPI, and a CLI for humans and agents." }
                    }
                }
                Card {
                    title: "Another option may fit better when".to_string(),
                    ul { class: "list-disc pl-5 space-y-2 text-[13.5px] text-text-2 leading-relaxed",
                        li { "You already run Plausible CE or Umami and are happy with an app-plus-database stack." }
                        li { "You want the lightest possible process and a smaller feature surface (GoatCounter is in that class)." }
                        li { "You need a multi-tenant or hosted SaaS rather than a single-owner appliance." }
                        li { "A specific license (AGPL, EUPL) or an existing ecosystem matters more than the ops shape." }
                    }
                }
            }

            // First boot
            div { class: "mt-10",
                Card {
                    title: "First boot".to_string(),
                    p { class: "text-[13.5px] text-text-2 leading-relaxed mb-3",
                        "Published image: "
                        code { class: "bg-black/40 rounded px-1 py-0.5 font-mono text-[12px]", "ghcr.io/kkir/stomatopod:latest" }
                        ". Mount a volume at "
                        code { class: "bg-black/40 rounded px-1 py-0.5 font-mono text-[12px]", "/app/data" }
                        " (or set "
                        code { class: "bg-black/40 rounded px-1 py-0.5 font-mono text-[12px]", "STOMATOPOD_STORAGE__DATA_DIR" }
                        "). Then:"
                    }
                    pre { class: "bg-black/40 rounded-lg p-3 sm:p-4 overflow-x-auto text-[12.5px] font-mono text-text-2 leading-relaxed",
                        "export STOMATOPOD_AUTH__SECRET_KEY=\"$(openssl rand -hex 32)\"\n\
                         export STOMATOPOD_ADMIN_PASSWORD=\"$(openssl rand -base64 24)\"\n\
                         export STOMATOPOD_ADMIN_EMAIL=you@example.com\n\
                         \n\
                         docker compose up -d"
                    }
                    p { class: "mt-3 text-muted-1 text-[13px] leading-relaxed",
                        "Open http://localhost:8080, sign in with the admin password you set, create a site, and paste the tracker snippet. Platform notes (volumes, health probes, Fly, Railway, Kubernetes) are in "
                        a {
                            class: "text-teal-hi hover:underline",
                            href: "{DEPLOY_URL}",
                            target: "_blank",
                            rel: "noopener noreferrer",
                            "DEPLOY.md"
                        }
                        "."
                    }
                }
            }

            // Links
            div { class: "mt-8 flex flex-wrap gap-3",
                Link {
                    class: "inline-flex items-center px-[18px] py-2.5 rounded-[10px] text-[14px] font-semibold bg-grad-btn text-[#032621] shadow-glow no-underline",
                    to: Route::GetStarted {},
                    "Get started"
                }
                a {
                    class: "inline-flex items-center px-[18px] py-2.5 rounded-[10px] text-[14px] font-semibold border border-border-2 text-text-1 no-underline",
                    href: "{GITHUB_URL}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    "GitHub repository"
                }
                a {
                    class: "inline-flex items-center px-[18px] py-2.5 rounded-[10px] text-[14px] font-semibold border border-border-2 text-text-1 no-underline",
                    href: "{DEPLOY_URL}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    "DEPLOY.md"
                }
                a {
                    class: "inline-flex items-center px-[18px] py-2.5 rounded-[10px] text-[14px] font-semibold border border-border-2 text-text-1 no-underline",
                    href: "{BENCHMARKS_URL}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    "BENCHMARKS.md"
                }
            }
        }
    }
}
