use dioxus::prelude::*;
use serde_json::json;

use crate::ui::api::{delete, get_json, patch_json, post_json, put_json};
use crate::ui::components::button::{Button, ButtonVariant};
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::form::Switch;
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::tabs::{SiteTab, SiteTabs};
use crate::ui::pages::{BTN_GHOST, BTN_PRIMARY, CTRL_INPUT};
use crate::ui::routes::Route;
use crate::ui::types::{
    CreateShareLinkBody, DigestSubscriptionResponse, PutSubscriptionBody, ShareLinksList,
    SiteSummary, SitesList,
};

/// General settings (name/domain/timezone) via PATCH /api/v1/sites/:site.
#[component]
fn GeneralCard(site: SiteSummary) -> Element {
    let mut name = use_signal(|| site.name.clone());
    let mut domain = use_signal(|| site.domain.clone());
    let mut timezone = use_signal(String::new);
    let mut saved = use_signal(|| false);
    let site_id = site.id.clone();

    rsx! {
        Card { title: "General",
            form {
                class: "flex flex-col gap-3",
                onsubmit: move |evt: FormEvent| {
                    evt.prevent_default();
                    let mut body = json!({ "name" : name(), "domain" : domain() });
                    let tz = timezone().trim().to_string();
                    if !tz.is_empty() {
                        body["timezone"] = json!(tz);
                    }
                    let site_id = site_id.clone();
                    let mut saved = saved;
                    spawn(async move {
                        let path = format!("/api/v1/sites/{site_id}");
                        if patch_json::<_, serde_json::Value>(&path, &body).await.is_ok() {
                            saved.set(true);
                        }
                    });
                },
                div { class: "grid grid-cols-1 md:grid-cols-2 gap-3",
                    label { class: "flex flex-col gap-1.5",
                        span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "Name" }
                        input {
                            class: "{CTRL_INPUT} w-full",
                            value: "{name}",
                            oninput: move |e| {
                                name.set(e.value());
                                saved.set(false);
                            },
                        }
                    }
                    label { class: "flex flex-col gap-1.5",
                        span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "Domain" }
                        input {
                            class: "{CTRL_INPUT} w-full",
                            value: "{domain}",
                            oninput: move |e| {
                                domain.set(e.value());
                                saved.set(false);
                            },
                        }
                    }
                    label { class: "flex flex-col gap-1.5",
                        span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "Timezone (leave blank to keep)" }
                        input {
                            class: "{CTRL_INPUT} w-full",
                            value: "{timezone}",
                            placeholder: "e.g. UTC",
                            oninput: move |e| {
                                timezone.set(e.value());
                                saved.set(false);
                            },
                        }
                    }
                }
                div { class: "flex items-center gap-3",
                    button { r#type: "submit", class: BTN_PRIMARY, "Save" }
                    if saved() {
                        span { class: "text-green text-xs", "Saved" }
                    }
                }
            }
        }
    }
}

/// Email digest subscription: GET current, toggle + frequency via PUT.
#[component]
fn DigestCard(site_id: String) -> Element {
    let refresh = use_signal(|| 0u32);
    let sub = use_resource({
        let site_id = site_id.clone();
        move || {
            let _ = refresh();
            let path = format!("/api/v1/sites/{site_id}/digest-subscription");
            async move { get_json::<DigestSubscriptionResponse>(&path).await }
        }
    });

    let put = {
        let site_id = site_id.clone();
        move |frequency: String, enabled: bool| {
            let site_id = site_id.clone();
            let mut refresh = refresh;
            spawn(async move {
                let path = format!("/api/v1/sites/{site_id}/digest-subscription");
                let body = PutSubscriptionBody { frequency, enabled };
                if put_json::<_, serde_json::Value>(&path, &body).await.is_ok() {
                    refresh += 1;
                }
            });
        }
    };

    rsx! {
        Card { title: "Email digest",
            {match &*sub.read() {
                None => rsx! {
                    Skeleton { lines: 2 }
                },
                Some(Err(e)) => rsx! {
                    EmptyState { message: format!("Failed to load subscription ({e})") }
                },
                Some(Ok(resp)) => {
                    let current = resp.subscription.clone();
                    let enabled = current.as_ref().map(|s| s.enabled).unwrap_or(false);
                    let frequency = current
                        .as_ref()
                        .map(|s| s.frequency.clone())
                        .unwrap_or_else(|| "weekly".to_string());
                    let put_toggle = put.clone();
                    let put_freq = put.clone();
                    let freq_for_toggle = frequency.clone();
                    rsx! {
                        div { class: "flex flex-wrap items-center gap-4",
                            Switch {
                                label: "Send me a digest",
                                checked: enabled,
                                onchange: move |on| put_toggle(freq_for_toggle.clone(), on),
                            }
                            select {
                                class: CTRL_INPUT,
                                value: "{frequency}",
                                onchange: move |e| put_freq(e.value(), enabled),
                                option { value: "weekly", "Weekly" }
                                option { value: "monthly", "Monthly" }
                                option { value: "both", "Weekly + monthly" }
                            }
                        }
                    }
                }
            }}
        }
    }
}

/// Public share links: list, create (label), delete.
#[component]
fn ShareLinksCard(site_id: String) -> Element {
    let refresh = use_signal(|| 0u32);
    let links = use_resource({
        let site_id = site_id.clone();
        move || {
            let _ = refresh();
            let path = format!("/api/v1/sites/{site_id}/share-links");
            async move { get_json::<ShareLinksList>(&path).await }
        }
    });
    let mut label = use_signal(String::new);

    rsx! {
        Card { title: "Share links",
            form {
                class: "flex flex-wrap items-end gap-2 mb-4",
                onsubmit: {
                    let site_id = site_id.clone();
                    move |evt: FormEvent| {
                        evt.prevent_default();
                        let l = label().trim().to_string();
                        let body = CreateShareLinkBody {
                            label: if l.is_empty() { None } else { Some(l) },
                            expires_at: None,
                        };
                        let site_id = site_id.clone();
                        let mut label = label;
                        let mut refresh = refresh;
                        spawn(async move {
                            let path = format!("/api/v1/sites/{site_id}/share-links");
                            if post_json::<_, serde_json::Value>(&path, &body).await.is_ok() {
                                label.set(String::new());
                                refresh += 1;
                            }
                        });
                    }
                },
                input {
                    class: CTRL_INPUT,
                    r#type: "text",
                    value: "{label}",
                    placeholder: "Label (optional)",
                    oninput: move |e| label.set(e.value()),
                }
                button { r#type: "submit", class: BTN_PRIMARY, "Create link" }
            }
            {match &*links.read() {
                None => rsx! {
                    Skeleton { lines: 2 }
                },
                Some(Err(e)) => rsx! {
                    EmptyState { message: format!("Failed to load share links ({e})") }
                },
                Some(Ok(list)) => {
                    if list.share_links.is_empty() {
                        rsx! {
                            EmptyState { message: "No share links" }
                        }
                    } else {
                        let site_id = site_id.clone();
                        rsx! {
                            div { class: "flex flex-col gap-2",
                                for link in list.share_links.clone() {
                                    div {
                                        key: "{link.id}",
                                        class: "flex items-center justify-between gap-3 py-2 border-t border-border-1",
                                        div { class: "min-w-0",
                                            div { class: "text-text-1 text-[13px] font-medium",
                                                {link.label.clone().unwrap_or_else(|| "Untitled".to_string())}
                                            }
                                            a {
                                                class: "text-muted-1 text-xs font-mono block max-w-[320px] overflow-hidden text-ellipsis whitespace-nowrap hover:text-teal-hi",
                                                href: "{link.url}",
                                                target: "_blank",
                                                "{link.url}"
                                            }
                                        }
                                        button {
                                            r#type: "button",
                                            class: BTN_GHOST,
                                            onclick: {
                                                let site_id = site_id.clone();
                                                let id = link.id.clone();
                                                move |_| {
                                                    let site_id = site_id.clone();
                                                    let id = id.clone();
                                                    spawn(async move {
                                                        let path = format!("/api/v1/sites/{site_id}/share-links/{id}");
                                                        if delete(&path).await.is_ok() {
                                                            let mut refresh = refresh;
                                                            refresh += 1;
                                                        }
                                                    });
                                                }
                                            },
                                            "Delete"
                                        }
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

/// Per-site settings page: general, digest, share links, and a danger zone
/// that deactivates the site (there is no hard-delete endpoint; PATCH
/// `is_active=false` is the closest operation).
#[component]
pub fn SiteSettings(site_id: String) -> Element {
    let sites = use_resource(move || async move { get_json::<SitesList>("/api/v1/sites").await });
    let site_name = {
        let guard = sites.read();
        match guard.as_ref() {
            Some(Ok(list)) => list
                .sites
                .iter()
                .find(|s| s.id == site_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| site_id.clone()),
            _ => site_id.clone(),
        }
    };

    rsx! {
        PageHead { title: "Site Settings", subtitle: "{site_name}" }
        SiteTabs { site_id: site_id.clone(), range: "30d", active: SiteTab::Settings }

        {match &*sites.read() {
            None => rsx! {
                Skeleton { lines: 4 }
            },
            Some(Err(e)) => rsx! {
                Card { EmptyState { message: format!("Failed to load site ({e})") } }
            },
            Some(Ok(list)) => {
                match list.sites.iter().find(|s| s.id == site_id).cloned() {
                    None => rsx! {
                        Card { EmptyState { message: "Site not found" } }
                    },
                    Some(site) => {
                        rsx! {
                            div { class: "flex flex-col gap-4",
                                GeneralCard { site }
                                DigestCard { site_id: site_id.clone() }
                                ShareLinksCard { site_id: site_id.clone() }
                                Card { title: "Danger zone",
                                    p { class: "text-muted-1 text-[12.5px] mb-3",
                                        "Deactivating a site stops it from accepting new events and hides it from the dashboard."
                                    }
                                    Button {
                                        variant: ButtonVariant::Danger,
                                        onclick: {
                                            let site_id = site_id.clone();
                                            move |_| {
                                                let site_id = site_id.clone();
                                                spawn(async move {
                                                    let path = format!("/api/v1/sites/{site_id}");
                                                    let body = json!({ "is_active" : false });
                                                    if patch_json::<_, serde_json::Value>(&path, &body)
                                                        .await
                                                        .is_ok()
                                                    {
                                                        navigator().push(Route::SitesIndex {});
                                                    }
                                                });
                                            }
                                        },
                                        "Deactivate site"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }}
    }
}
