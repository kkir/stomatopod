use dioxus::prelude::*;

use crate::ui::api;
use crate::ui::components::button::{Button, ButtonVariant};
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::chart::Sparkline;
use crate::ui::components::form::Field;
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::stat::{DeltaDir, DeltaInfo};
use crate::ui::pages::BTN_PRIMARY;
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
use crate::ui::types::{CreateSiteBody, CreatedSite, PageviewsResult, SiteSummary, SitesList};

/// Grouped thousands for card metrics (e.g. 12345 → "12,345").
fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i.is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

fn compute_delta(cur: u64, prev: u64) -> Option<DeltaInfo> {
    if prev == 0 {
        return if cur > 0 {
            Some(DeltaInfo {
                dir: DeltaDir::New,
                pct: 0.0,
            })
        } else {
            None
        };
    }
    let pct = (cur as f64 - prev as f64) / prev as f64 * 100.0;
    let dir = if pct > 0.0 {
        DeltaDir::Up
    } else if pct < 0.0 {
        DeltaDir::Down
    } else {
        DeltaDir::Flat
    };
    Some(DeltaInfo {
        dir,
        pct: pct.abs(),
    })
}

#[component]
fn DeltaBadge(delta: DeltaInfo) -> Element {
    rsx! {
        span {
            class: match delta.dir {
                DeltaDir::Up => "text-[11px] font-semibold text-green",
                DeltaDir::Down => "text-[11px] font-semibold text-red",
                DeltaDir::New | DeltaDir::Flat => "text-[11px] font-semibold text-muted-2",
            },
            {match delta.dir {
                DeltaDir::Up => rsx! { "▲ {delta.pct:.0}%" },
                DeltaDir::Down => rsx! { "▼ {delta.pct:.0}%" },
                DeltaDir::New => rsx! { "New" },
                DeltaDir::Flat => rsx! { "-" },
            }}
        }
    }
}

/// One site as a summary card: name/domain, last-7d pageviews + sessions,
/// period delta, and a tiny traffic sparkline. Fetches its own
/// `GET /api/v1/sites/:id/pageviews?range=7d&compare=1`.
#[component]
fn SiteCard(site: SiteSummary) -> Element {
    let site_id = site.id.clone();
    let stats = use_resource(move || {
        let site_id = site_id.clone();
        async move {
            api::get_json::<PageviewsResult>(&format!(
                "/api/v1/sites/{site_id}/pageviews?range=7d&compare=1"
            ))
            .await
        }
    });

    rsx! {
        Link {
            to: Route::SiteOverview {
                site_id: site.id.clone(),
                q: DashQuery {
                    range: Some("7d".into()),
                    ..Default::default()
                },
            },
            class: "group flex flex-col gap-4 bg-surface-1 border border-border-1 rounded-lg p-f3 shadow-sm shadow-inner-hi no-underline hover:border-border-3 transition-colors min-h-[168px]",
            div { class: "flex items-start justify-between gap-3",
                div { class: "min-w-0",
                    div { class: "text-[15px] font-semibold text-text-1 truncate", "{site.name}" }
                    div { class: "text-muted-1 text-[12.5px] mt-0.5 truncate", "{site.domain}" }
                }
                span {
                    class: "text-muted-2 group-hover:text-teal-hi transition-colors shrink-0 mt-0.5",
                    "aria-hidden": "true",
                    "\u{2192}"
                }
            }

            {match &*stats.read() {
                None => rsx! {
                    div { class: "mt-auto",
                        Skeleton { lines: 2 }
                    }
                },
                Some(Err(_)) => rsx! {
                    div { class: "mt-auto text-muted-2 text-[12.5px]",
                        "Could not load stats"
                    }
                },
                Some(Ok(pv)) => {
                    let spark: Vec<f64> = pv
                        .buckets
                        .iter()
                        .map(|b| b.pageviews as f64)
                        .collect();
                    let pv_delta = pv
                        .comparison
                        .as_ref()
                        .and_then(|c| compute_delta(pv.total_pageviews, c.total_pageviews));
                    let sess_delta = pv
                        .comparison
                        .as_ref()
                        .and_then(|c| compute_delta(pv.total_sessions, c.total_sessions));
                    let pageviews = fmt_count(pv.total_pageviews);
                    let sessions = fmt_count(pv.total_sessions);
                    let empty = pv.total_pageviews == 0 && pv.total_sessions == 0;
                    rsx! {
                        if empty {
                            div { class: "mt-auto",
                                p { class: "text-muted-1 text-[12.5px]",
                                    "No traffic in the last 7 days"
                                }
                                p { class: "text-muted-2 text-[11.5px] mt-1",
                                    "Install the tracker to start collecting."
                                }
                            }
                        } else {
                            div { class: "mt-auto flex flex-col gap-3",
                                div { class: "grid grid-cols-2 gap-3",
                                    div {
                                        div { class: "text-muted-1 text-[10.5px] uppercase tracking-[0.1em] font-semibold",
                                            "Pageviews"
                                        }
                                        div { class: "flex items-baseline gap-2 mt-1",
                                            span { class: "text-grad-value font-display text-[22px] font-bold tracking-tight tabular-nums",
                                                "{pageviews}"
                                            }
                                            if let Some(d) = pv_delta {
                                                DeltaBadge { delta: d }
                                            }
                                        }
                                    }
                                    div {
                                        div { class: "text-muted-1 text-[10.5px] uppercase tracking-[0.1em] font-semibold",
                                            "Sessions"
                                        }
                                        div { class: "flex items-baseline gap-2 mt-1",
                                            span { class: "text-grad-value font-display text-[22px] font-bold tracking-tight tabular-nums",
                                                "{sessions}"
                                            }
                                            if let Some(d) = sess_delta {
                                                DeltaBadge { delta: d }
                                            }
                                        }
                                    }
                                }
                                div { class: "flex items-center justify-between gap-2",
                                    span { class: "text-muted-2 text-[11px]", "Last 7 days" }
                                    if spark.len() >= 2 {
                                        Sparkline { points: spark, width: 96.0, height: 24.0 }
                                    }
                                }
                            }
                        }
                    }
                }
            }}
        }
    }
}

/// Sites index: responsive card grid with per-site 7-day traffic summaries.
#[component]
pub fn SitesIndex() -> Element {
    let mut refresh = use_signal(|| 0u32);
    let mut show_form = use_signal(|| false);
    let mut domain = use_signal(String::new);
    let mut name = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);

    let sites = use_resource(move || {
        refresh();
        async move { api::get_json::<SitesList>("/api/v1/sites").await }
    });

    let onsubmit = move |evt: FormEvent| {
        evt.prevent_default();
        let body = CreateSiteBody {
            domain: domain(),
            name: name(),
        };
        spawn(async move {
            match api::post_json::<CreateSiteBody, CreatedSite>("/api/v1/sites", &body).await {
                Ok(created) => {
                    domain.set(String::new());
                    name.set(String::new());
                    show_form.set(false);
                    error.set(None);
                    refresh.set(refresh() + 1);
                    navigator().push(Route::SiteOverview {
                        site_id: created.id,
                        q: DashQuery::default(),
                    });
                }
                Err(e) => error.set(Some(e.to_string())),
            }
        });
    };

    let body = match &*sites.read() {
        None => rsx! { Skeleton { lines: 4 } },
        Some(Err(e)) => rsx! {
            Card { EmptyState { message: e.to_string() } }
        },
        Some(Ok(list)) if list.sites.is_empty() => rsx! {
            Card {
                EmptyState {
                    title: "Track your first site",
                    message: "A site is any website or app you want to measure. Create one to get a lightweight, cookie-free tracking snippet and start seeing pageviews, referrers, devices, and conversions in real time.",
                    button {
                        r#type: "button",
                        class: "{BTN_PRIMARY} mt-6",
                        onclick: move |_| show_form.set(true),
                        "New Site"
                    }
                }
            }
        },
        Some(Ok(list)) => rsx! {
            div { class: "grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4 mt-2",
                for site in list.sites.clone() {
                    SiteCard { key: "{site.id}", site }
                }
            }
        },
    };

    rsx! {
        PageHead { title: "Sites", subtitle: "All sites in your organization.",
            Button {
                variant: ButtonVariant::Primary,
                onclick: move |_| show_form.set(!show_form()),
                "New Site"
            }
        }

        if show_form() {
            Card { title: "Add a new site".to_string(),
                form { class: "grid grid-cols-[1fr_1fr_auto] gap-3 items-end mb-4", onsubmit,
                    Field { label: "Domain",
                        input {
                            class: "w-full bg-black/32 border border-border-2 text-text-1 rounded-[10px] px-3 py-2 text-[13px] shadow-inner-hi focus:outline-none focus:border-teal/55",
                            placeholder: "example.com",
                            required: true,
                            value: "{domain}",
                            oninput: move |evt| domain.set(evt.value()),
                        }
                    }
                    Field { label: "Name",
                        input {
                            class: "w-full bg-black/32 border border-border-2 text-text-1 rounded-[10px] px-3 py-2 text-[13px] shadow-inner-hi focus:outline-none focus:border-teal/55",
                            placeholder: "My Site",
                            required: true,
                            value: "{name}",
                            oninput: move |evt| name.set(evt.value()),
                        }
                    }
                    button {
                        class: "inline-flex items-center gap-1.5 px-[15px] py-2 rounded-[10px] text-[13px] font-semibold tracking-tight cursor-pointer transition-all duration-150 bg-grad-btn text-[#032621] shadow-glow hover:-translate-y-px",
                        r#type: "submit",
                        "Create"
                    }
                }
                if let Some(msg) = error() {
                    p { class: "text-red text-[12.5px] mt-2", "{msg}" }
                }
            }
        }

        {body}
    }
}
