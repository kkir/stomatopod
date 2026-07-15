use dioxus::prelude::*;

use crate::ui::api::{delete, get_json, post_json};
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::pages::{BTN_GHOST, BTN_PRIMARY, CTRL_INPUT};
use crate::ui::types::{ApiKey, CreateKeyBody, CreatedApiKey, KeysList};

/// Account password rotation for the single self-hosted owner.
#[component]
fn PasswordCard() -> Element {
    let mut current = use_signal(String::new);
    let mut new_pw = use_signal(String::new);
    let mut confirm = use_signal(String::new);
    let mut status = use_signal(String::new);

    rsx! {
        Card { title: "Change password",
            p { class: "text-muted-1 text-[12.5px] mb-3",
                "Rotate the owner password for this instance. New password must be at least 12 characters."
            }
            form {
                class: "flex flex-col gap-2 max-w-md",
                onsubmit: move |evt: FormEvent| {
                    evt.prevent_default();
                    let cur = current().trim().to_string();
                    let next = new_pw().trim().to_string();
                    let conf = confirm().trim().to_string();
                    if next.len() < 12 {
                        status.set("New password must be at least 12 characters".into());
                        return;
                    }
                    if next != conf {
                        status.set("New password and confirmation do not match".into());
                        return;
                    }
                    let mut status = status;
                    let mut current = current;
                    let mut new_pw = new_pw;
                    let mut confirm = confirm;
                    spawn(async move {
                        let body = serde_json::json!({
                            "current_password": cur,
                            "new_password": next,
                        });
                        match post_json::<_, serde_json::Value>("/api/v1/me/password", &body).await {
                            Ok(_) => {
                                status.set("Password updated".into());
                                current.set(String::new());
                                new_pw.set(String::new());
                                confirm.set(String::new());
                            }
                            Err(e) => status.set(format!("Failed: {e}")),
                        }
                    });
                },
                label { class: "flex flex-col gap-1.5",
                    span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "Current password" }
                    input {
                        class: CTRL_INPUT,
                        r#type: "password",
                        autocomplete: "current-password",
                        placeholder: "Current password",
                        value: "{current}",
                        oninput: move |e| current.set(e.value()),
                    }
                }
                label { class: "flex flex-col gap-1.5",
                    span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "New password" }
                    input {
                        class: CTRL_INPUT,
                        r#type: "password",
                        autocomplete: "new-password",
                        placeholder: "New password",
                        value: "{new_pw}",
                        oninput: move |e| new_pw.set(e.value()),
                    }
                }
                label { class: "flex flex-col gap-1.5",
                    span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "Confirm new password" }
                    input {
                        class: CTRL_INPUT,
                        r#type: "password",
                        autocomplete: "new-password",
                        placeholder: "Confirm new password",
                        value: "{confirm}",
                        oninput: move |e| confirm.set(e.value()),
                    }
                }
                button { r#type: "submit", class: BTN_PRIMARY, "Update password" }
                if !status().is_empty() {
                    p {
                        class: "text-muted-1 text-[12px]",
                        role: "status",
                        "aria-live": "polite",
                        "{status}"
                    }
                }
            }
        }
    }
}

/// A one-time reveal banner for a freshly minted key's plaintext secret.
#[component]
pub fn SecretBanner(created: CreatedApiKey) -> Element {
    rsx! {
        div {
            class: "bg-teal-soft border border-teal/50 rounded-lg p-3 mb-4",
            role: "status",
            "aria-live": "polite",
            div { class: "text-teal-hi text-[13px] font-semibold mb-1",
                "Key \"{created.name}\" created - copy it now, it won't be shown again:"
            }
            code { class: "block bg-black/40 rounded-md px-3 py-2 text-text-1 text-[13px] break-all select-all",
                "{created.secret}"
            }
        }
    }
}

/// Renders one key row with a delete button. `delete_path` is the full
/// DELETE endpoint for this key.
#[component]
pub fn KeyRow(api_key: ApiKey, delete_path: String, on_delete: EventHandler<()>) -> Element {
    rsx! {
        div { class: "flex items-center justify-between gap-3 py-2 border-t border-border-1",
            div {
                div { class: "text-text-1 text-[13px] font-medium",
                    "{api_key.name}"
                    span { class: "ml-2 text-muted-1 text-[11px] uppercase tracking-[0.06em]", "{api_key.scope}" }
                }
                div { class: "text-muted-1 text-xs font-mono", "{api_key.display_prefix}…" }
            }
            button {
                r#type: "button",
                class: BTN_GHOST,
                "aria-label": "Revoke key {api_key.name}",
                onclick: move |_| {
                    let delete_path = delete_path.clone();
                    spawn(async move {
                        if delete(&delete_path).await.is_ok() {
                            on_delete.call(());
                        }
                    });
                },
                "Revoke"
            }
        }
    }
}

/// Global API keys page: keys not scoped to a single site (org-wide read
/// keys, plus any site-bound keys the org owns).
#[component]
pub fn Keys() -> Element {
    let refresh = use_signal(|| 0u32);
    let keys = use_resource(move || {
        let _ = refresh();
        async move { get_json::<KeysList>("/api/v1/keys").await }
    });

    let mut name = use_signal(String::new);
    let mut scope = use_signal(|| "read".to_string());
    let created = use_signal(|| None::<CreatedApiKey>);

    rsx! {
        PageHead { title: "API Keys", subtitle: "Keys not scoped to a single site." }

        div { class: "mb-4",
            PasswordCard {}
        }

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
                "aria-label": "Create API key",
                onsubmit: move |evt: FormEvent| {
                    evt.prevent_default();
                    let n = name().trim().to_string();
                    if n.is_empty() {
                        return;
                    }
                    let body = CreateKeyBody {
                        name: n,
                        scope: scope(),
                        site_id: None,
                    };
                    let mut created = created;
                    let mut refresh = refresh;
                    let mut name = name;
                    spawn(async move {
                        if let Ok(c) = post_json::<_, CreatedApiKey>("/api/v1/keys", &body).await {
                            created.set(Some(c));
                            name.set(String::new());
                            refresh += 1;
                        }
                    });
                },
                label { class: "flex flex-col gap-1",
                    span { class: "sr-only", "Key name" }
                    input {
                        class: CTRL_INPUT,
                        r#type: "text",
                        value: "{name}",
                        placeholder: "Key name",
                        required: true,
                        oninput: move |e| name.set(e.value()),
                    }
                }
                label { class: "flex flex-col gap-1",
                    span { class: "sr-only", "Key scope" }
                    select {
                        class: CTRL_INPUT,
                        value: "{scope}",
                        onchange: move |e| scope.set(e.value()),
                        option { value: "read", "Read (org-wide)" }
                    }
                }
                button { r#type: "submit", class: BTN_PRIMARY, "Create key" }
            }
            p { class: "text-muted-1 text-xs mt-2",
                "Ingest keys are created per-site from a site's API Keys tab."
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
                            rsx! {
                                for key in list.keys.clone() {
                                    KeyRow {
                                        key: "{key.id}",
                                        delete_path: format!("/api/v1/keys/{}", key.id),
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
