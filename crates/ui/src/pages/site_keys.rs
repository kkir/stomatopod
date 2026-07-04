use dioxus::prelude::*;

use super::keys::{KeyRow, SecretBanner};
use crate::api::{get_json, post_json};
use crate::components::card::{Card, EmptyState};
use crate::components::layout::PageHead;
use crate::components::skeleton::Skeleton;
use crate::components::tabs::{SiteTab, SiteTabs};
use crate::pages::{BTN_PRIMARY, CTRL_INPUT};
use crate::types::{CreateSiteKeyBody, CreatedApiKey, KeysList};

/// Per-site API keys page: keys bound to this site plus org-wide read keys.
#[component]
pub fn SiteKeys(site_id: String) -> Element {
    let refresh = use_signal(|| 0u32);
    let keys = use_resource({
        let site_id = site_id.clone();
        move || {
            let _ = refresh();
            let path = format!("/api/v1/sites/{site_id}/keys");
            async move { get_json::<KeysList>(&path).await }
        }
    });

    let mut name = use_signal(String::new);
    let mut scope = use_signal(|| "ingest".to_string());
    let created = use_signal(|| None::<CreatedApiKey>);

    rsx! {
        PageHead { title: "API Keys", subtitle: "Site {site_id}" }
        SiteTabs { site_id: site_id.clone(), range: "30d", active: SiteTab::Keys }

        Card { title: "New key",
            {
                let created_val = created();
                rsx! {
                    if let Some(c) = created_val {
                        SecretBanner { created: c }
                    }
                }
            }
            form {
                class: "flex flex-wrap items-end gap-2",
                onsubmit: {
                    let site_id = site_id.clone();
                    move |evt: FormEvent| {
                        evt.prevent_default();
                        let n = name().trim().to_string();
                        if n.is_empty() {
                            return;
                        }
                        let sc = scope();
                        let body = CreateSiteKeyBody {
                            name: n,
                            org_wide: sc == "read",
                            scope: sc,
                        };
                        let site_id = site_id.clone();
                        let mut created = created;
                        let mut refresh = refresh;
                        let mut name = name;
                        spawn(async move {
                            let path = format!("/api/v1/sites/{site_id}/keys");
                            if let Ok(c) = post_json::<_, CreatedApiKey>(&path, &body).await {
                                created.set(Some(c));
                                name.set(String::new());
                                refresh += 1;
                            }
                        });
                    }
                },
                input {
                    class: CTRL_INPUT,
                    r#type: "text",
                    value: "{name}",
                    placeholder: "Key name",
                    oninput: move |e| name.set(e.value()),
                }
                select {
                    class: CTRL_INPUT,
                    value: "{scope}",
                    onchange: move |e| scope.set(e.value()),
                    option { value: "ingest", "Ingest (this site)" }
                    option { value: "read", "Read (org-wide)" }
                }
                button { r#type: "submit", class: BTN_PRIMARY, "Create key" }
            }
        }

        div { class: "mt-4",
            Card { title: "Keys",
                {match &*keys.read() {
                    None => rsx! {
                        Skeleton { lines: 3 }
                    },
                    Some(Err(e)) => rsx! {
                        EmptyState { message: format!("Failed to load keys ({e})") }
                    },
                    Some(Ok(list)) => {
                        if list.keys.is_empty() {
                            rsx! {
                                EmptyState { message: "No keys yet" }
                            }
                        } else {
                            let site_id = site_id.clone();
                            rsx! {
                                for key in list.keys.clone() {
                                    KeyRow {
                                        key: "{key.id}",
                                        delete_path: format!("/api/v1/sites/{site_id}/keys/{}", key.id),
                                        api_key: key,
                                        on_delete: move |_| {
                                            let mut refresh = refresh;
                                            refresh += 1;
                                        },
                                    }
                                }
                            }
                        }
                    }
                }}
            }
        }
    }
}
