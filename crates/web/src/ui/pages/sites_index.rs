use dioxus::prelude::*;

use crate::ui::api;
use crate::ui::components::button::{Button, ButtonVariant};
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::form::Field;
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::pages::BTN_PRIMARY;
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
use crate::ui::types::{CreateSiteBody, CreatedSite, SitesList};

/// Port of index.jinja: the site-cards list. `GET /api/v1/sites` only
/// returns `id`/`domain`/`name` (no per-site stats or sparkline), which
/// matches index.jinja's own cards - they show just the name and domain
/// too, so no data is missing here.
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
            div { class: "flex flex-col gap-2.5 mt-2",
                for site in list.sites.clone() {
                    Link {
                        key: "{site.id}",
                        to: Route::SiteOverview { site_id: site.id.clone(), q: DashQuery::default() },
                        class: "flex items-center justify-between gap-4 bg-surface-1 border border-border-1 rounded-lg p-f3 shadow-sm shadow-inner-hi no-underline hover:border-border-3 transition-colors",
                        div {
                            div { class: "text-[14px] font-semibold text-text-1", "{site.name}" }
                            div { class: "text-muted-1 text-[12.5px] mt-0.5", "{site.domain}" }
                        }
                        span { class: "text-muted-1", "aria-hidden": "true", "\u{2192}" }
                    }
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
                form { class: "grid grid-cols-[1fr_1fr_auto] gap-3 items-end", onsubmit,
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
