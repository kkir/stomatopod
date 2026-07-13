use dioxus::prelude::*;
use serde_json::json;

use crate::ui::api::{delete, get_json, patch_json, post_json, put_json};
use crate::ui::components::button::{Button, ButtonVariant};
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::form::Switch;
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::tabs::{SiteTab, SiteTabs, TabbedCard};
use crate::ui::pages::{
    invalidate_sites_cache, site_from_resource, use_sites_list, BTN_GHOST, BTN_PRIMARY, CTRL_INPUT,
};
use crate::ui::routes::Route;
use crate::ui::types::{
    AlertChannel, ChannelTestResult, ChannelsList, CreateChannelBody, DigestSubscriptionResponse,
    PutSubscriptionBody, SiteSummary,
};

/// General settings (name/domain) via PATCH /api/v1/sites/:site.
///
/// Site timezone is kept on the model/API for future digest scheduling, but
/// is not exposed here: digests still fire in UTC, and the dashboard shows
/// times in the browser's local zone.
#[component]
fn GeneralCard(site: SiteSummary) -> Element {
    let mut name = use_signal(|| site.name.clone());
    let mut domain = use_signal(|| site.domain.clone());
    let mut saved = use_signal(|| false);
    let error = use_signal(|| None::<String>);
    let site_id = site.id.clone();

    rsx! {
        Card { title: "General",
            form {
                class: "flex flex-col gap-3",
                onsubmit: move |evt: FormEvent| {
                    evt.prevent_default();
                    let body = json!({
                        "name": name(),
                        "domain": domain(),
                    });
                    let site_id = site_id.clone();
                    let mut saved = saved;
                    let mut error = error;
                    spawn(async move {
                        let path = format!("/api/v1/sites/{site_id}");
                        match patch_json::<_, serde_json::Value>(&path, &body).await {
                            Ok(_) => {
                                error.set(None);
                                saved.set(true);
                            }
                            Err(e) => {
                                saved.set(false);
                                error.set(Some(e.to_string()));
                            }
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
                }
                div { class: "flex items-center gap-3",
                    button { r#type: "submit", class: BTN_PRIMARY, "Save" }
                    if saved() {
                        span { class: "text-green text-xs", "Saved" }
                    }
                    if let Some(err) = error() {
                        span { class: "text-red-400 text-xs", "{err}" }
                    }
                }
            }
        }
    }
}

/// Analytics digest subscription: GET current, toggle + frequency via PUT.
/// Digests go to the site's notification destinations (below).
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
        Card { title: "Analytics digest",
            p { class: "text-muted-1 text-[12.5px] mb-3",
                "Weekly or monthly summary delivered via the notification destinations below (Slack, Telegram, or webhook)."
            }
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

type ChannelsRes = Resource<Result<ChannelsList, crate::ui::api::ApiError>>;

/// Input that resists browser credential autofill.
///
/// Chrome ignores `autocomplete=off` on fields it thinks are logins. The
/// reliable tricks: never use a `<form>` or `type=password`, and keep the
/// control `readonly` until the user focuses it (autofill skips readonly).
#[component]
fn NoAutofillInput(value: Signal<String>, placeholder: String, secret: bool) -> Element {
    let mut unlocked = use_signal(|| false);
    let class = if secret {
        format!("{CTRL_INPUT} w-full font-mono")
    } else {
        format!("{CTRL_INPUT} w-full")
    };
    rsx! {
        input {
            class: "{class}",
            // Never "password" or "email" - those trigger the credential UI.
            r#type: "text",
            inputmode: "text",
            // Non-standard token; "off" is ignored by Chrome.
            autocomplete: "one-time-code",
            autocorrect: "off",
            autocapitalize: "off",
            spellcheck: false,
            // Chrome will not autofill into a readonly field.
            readonly: !unlocked(),
            "data-1p-ignore": true,
            "data-lpignore": "true",
            "data-bwignore": "true",
            "data-form-type": "other",
            "data-protonpass-ignore": true,
            placeholder: "{placeholder}",
            value: "{value}",
            style: if secret {
                "-webkit-text-security: disc;"
            } else {
                ""
            },
            onfocus: move |_| unlocked.set(true),
            onblur: move |_| {
                if value().trim().is_empty() {
                    unlocked.set(false);
                }
            },
            oninput: move |e| value.set(e.value()),
        }
    }
}

/// Shared list rows for a channel kind: destination label, Test, Delete.
#[component]
fn ChannelList(
    site_id: String,
    kind: String,
    channels: ChannelsRes,
    refresh: Signal<u32>,
    status: Signal<String>,
    empty_message: String,
) -> Element {
    rsx! {
        {match &*channels.read() {
            None => rsx! {
                Skeleton { lines: 2 }
            },
            Some(Err(e)) => rsx! {
                EmptyState { message: format!("Failed to load ({e})") }
            },
            Some(Ok(list)) => {
                let rows: Vec<AlertChannel> = list
                    .channels
                    .iter()
                    .filter(|c| c.kind == kind)
                    .cloned()
                    .collect();
                if rows.is_empty() {
                    rsx! {
                        EmptyState { message: empty_message }
                    }
                } else {
                    rsx! {
                        div { class: "flex flex-col gap-2",
                            for ch in rows {
                                div {
                                    key: "{ch.id}",
                                    class: "flex items-center justify-between gap-3 py-2 border-t border-border-1",
                                    div { class: "min-w-0",
                                        div {
                                            class: "text-text-1 text-[13px] font-medium font-mono max-w-full overflow-hidden text-ellipsis whitespace-nowrap",
                                            "{ch.url}"
                                        }
                                        if ch.last_error_at.is_some() {
                                            div { class: "text-red-400 text-xs", "Last delivery failed" }
                                        }
                                    }
                                    div { class: "flex items-center gap-2 shrink-0",
                                        button {
                                            r#type: "button",
                                            class: BTN_GHOST,
                                            onclick: {
                                                let site_id = site_id.clone();
                                                let id = ch.id.clone();
                                                move |_| {
                                                    let site_id = site_id.clone();
                                                    let id = id.clone();
                                                    let mut status = status;
                                                    spawn(async move {
                                                        let path = format!(
                                                            "/api/v1/sites/{site_id}/alert-channels/{id}/test",
                                                        );
                                                        match post_json::<(), ChannelTestResult>(&path, &()).await {
                                                            Ok(r) if r.result == "ok" => {
                                                                status.set("Test delivered".to_string())
                                                            }
                                                            Ok(r) => status.set(format!(
                                                                "Test failed: {}",
                                                                r.error.unwrap_or_default()
                                                            )),
                                                            Err(e) => status.set(format!("Test error: {e}")),
                                                        }
                                                    });
                                                }
                                            },
                                            "Test"
                                        }
                                        button {
                                            r#type: "button",
                                            class: BTN_GHOST,
                                            onclick: {
                                                let site_id = site_id.clone();
                                                let id = ch.id.clone();
                                                move |_| {
                                                    let site_id = site_id.clone();
                                                    let id = id.clone();
                                                    spawn(async move {
                                                        let path = format!(
                                                            "/api/v1/sites/{site_id}/alert-channels/{id}",
                                                        );
                                                        if delete(&path).await.is_ok() {
                                                            let mut r = refresh;
                                                            r += 1;
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
            }
        }}
    }
}

/// POST a new alert channel; shared by all notification panels.
fn create_channel(
    site_id: String,
    body: CreateChannelBody,
    refresh: Signal<u32>,
    mut status: Signal<String>,
    mut error: Signal<Option<String>>,
    ok_msg: &'static str,
    clear: impl FnOnce() + 'static,
) {
    spawn(async move {
        let path = format!("/api/v1/sites/{site_id}/alert-channels");
        match post_json::<_, serde_json::Value>(&path, &body).await {
            Ok(_) => {
                clear();
                error.set(None);
                status.set(ok_msg.into());
                let mut r = refresh;
                r += 1;
            }
            Err(e) => error.set(Some(e.to_string())),
        }
    });
}

/// Telegram panel: chat id + bot token (default Notifications tab).
///
/// Intentionally not a `<form>` - Chrome credential autofill keys off forms
/// with two text fields.
#[component]
fn TelegramPanel(site_id: String, channels: ChannelsRes, refresh: Signal<u32>) -> Element {
    let mut chat_id = use_signal(String::new);
    let mut bot_token = use_signal(String::new);
    let status = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);

    rsx! {
        div {
            p { class: "text-muted-1 text-[12.5px] mb-3",
                "Create a bot with @BotFather, then paste the bot token and the chat id where alerts should land (personal chat or group)."
            }
            div { class: "flex flex-col gap-3 mb-4",
                div { class: "grid grid-cols-1 md:grid-cols-2 gap-3",
                    label { class: "flex flex-col gap-1.5",
                        span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "Chat id" }
                        NoAutofillInput {
                            value: chat_id,
                            placeholder: "-1001234567890".to_string(),
                            secret: false,
                        }
                    }
                    label { class: "flex flex-col gap-1.5",
                        span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "Bot token" }
                        NoAutofillInput {
                            value: bot_token,
                            placeholder: "123456:ABC-DEF...".to_string(),
                            secret: true,
                        }
                    }
                }
                div { class: "flex items-center gap-3",
                    button {
                        r#type: "button",
                        class: BTN_PRIMARY,
                        onclick: {
                            let site_id = site_id.clone();
                            move |_| {
                                let chat = chat_id().trim().to_string();
                                let token = bot_token().trim().to_string();
                                if chat.is_empty() || token.is_empty() {
                                    error.set(Some("Chat id and bot token are required".into()));
                                    return;
                                }
                                let body = CreateChannelBody {
                                    kind: "telegram".into(),
                                    url: chat,
                                    secret: Some(token),
                                };
                                create_channel(
                                    site_id.clone(),
                                    body,
                                    refresh,
                                    status,
                                    error,
                                    "Telegram destination added",
                                    move || {
                                        chat_id.set(String::new());
                                        bot_token.set(String::new());
                                    },
                                );
                            }
                        },
                        "Add Telegram"
                    }
                    if let Some(err) = error() {
                        span { class: "text-red-400 text-xs", "{err}" }
                    }
                }
            }
            if !status().is_empty() {
                div { class: "text-xs text-muted-1 mb-3", "{status}" }
            }
            ChannelList {
                site_id: site_id.clone(),
                kind: "telegram".to_string(),
                channels,
                refresh,
                status,
                empty_message: "No Telegram destinations yet".to_string(),
            }
        }
    }
}

/// Slack panel: incoming webhook URL.
#[component]
fn SlackPanel(site_id: String, channels: ChannelsRes, refresh: Signal<u32>) -> Element {
    let mut url = use_signal(String::new);
    let mut secret = use_signal(String::new);
    let status = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);

    rsx! {
        div {
            p { class: "text-muted-1 text-[12.5px] mb-3",
                "Paste an Incoming Webhook URL from a Slack app. Optional secret is used to sign payloads if you verify them."
            }
            div { class: "flex flex-col gap-3 mb-4",
                label { class: "flex flex-col gap-1.5",
                    span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "Webhook URL" }
                    NoAutofillInput {
                        value: url,
                        placeholder: "https://hooks.slack.com/services/...".to_string(),
                        secret: false,
                    }
                }
                label { class: "flex flex-col gap-1.5",
                    span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "Signing secret (optional)" }
                    NoAutofillInput {
                        value: secret,
                        placeholder: "Optional".to_string(),
                        secret: true,
                    }
                }
                div { class: "flex items-center gap-3",
                    button {
                        r#type: "button",
                        class: BTN_PRIMARY,
                        onclick: {
                            let site_id = site_id.clone();
                            move |_| {
                                let dest = url().trim().to_string();
                                if dest.is_empty() {
                                    error.set(Some("Webhook URL is required".into()));
                                    return;
                                }
                                let sec = secret().trim().to_string();
                                let body = CreateChannelBody {
                                    kind: "slack".into(),
                                    url: dest,
                                    secret: if sec.is_empty() { None } else { Some(sec) },
                                };
                                create_channel(
                                    site_id.clone(),
                                    body,
                                    refresh,
                                    status,
                                    error,
                                    "Slack destination added",
                                    move || {
                                        url.set(String::new());
                                        secret.set(String::new());
                                    },
                                );
                            }
                        },
                        "Add Slack"
                    }
                    if let Some(err) = error() {
                        span { class: "text-red-400 text-xs", "{err}" }
                    }
                }
            }
            if !status().is_empty() {
                div { class: "text-xs text-muted-1 mb-3", "{status}" }
            }
            ChannelList {
                site_id: site_id.clone(),
                kind: "slack".to_string(),
                channels,
                refresh,
                status,
                empty_message: "No Slack destinations yet".to_string(),
            }
        }
    }
}

/// Generic HTTPS webhook panel.
#[component]
fn WebhookPanel(site_id: String, channels: ChannelsRes, refresh: Signal<u32>) -> Element {
    let mut url = use_signal(String::new);
    let mut secret = use_signal(String::new);
    let status = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);

    rsx! {
        div {
            p { class: "text-muted-1 text-[12.5px] mb-3",
                "POST alert JSON to any public HTTPS endpoint. Optional secret signs the body for verification."
            }
            div { class: "flex flex-col gap-3 mb-4",
                label { class: "flex flex-col gap-1.5",
                    span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "Endpoint URL" }
                    NoAutofillInput {
                        value: url,
                        placeholder: "https://example.com/hooks/stomatopod".to_string(),
                        secret: false,
                    }
                }
                label { class: "flex flex-col gap-1.5",
                    span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "Signing secret (optional)" }
                    NoAutofillInput {
                        value: secret,
                        placeholder: "Optional".to_string(),
                        secret: true,
                    }
                }
                div { class: "flex items-center gap-3",
                    button {
                        r#type: "button",
                        class: BTN_PRIMARY,
                        onclick: {
                            let site_id = site_id.clone();
                            move |_| {
                                let dest = url().trim().to_string();
                                if dest.is_empty() {
                                    error.set(Some("Endpoint URL is required".into()));
                                    return;
                                }
                                let sec = secret().trim().to_string();
                                let body = CreateChannelBody {
                                    kind: "webhook".into(),
                                    url: dest,
                                    secret: if sec.is_empty() { None } else { Some(sec) },
                                };
                                create_channel(
                                    site_id.clone(),
                                    body,
                                    refresh,
                                    status,
                                    error,
                                    "Webhook added",
                                    move || {
                                        url.set(String::new());
                                        secret.set(String::new());
                                    },
                                );
                            }
                        },
                        "Add webhook"
                    }
                    if let Some(err) = error() {
                        span { class: "text-red-400 text-xs", "{err}" }
                    }
                }
            }
            if !status().is_empty() {
                div { class: "text-xs text-muted-1 mb-3", "{status}" }
            }
            ChannelList {
                site_id: site_id.clone(),
                kind: "webhook".to_string(),
                channels,
                refresh,
                status,
                empty_message: "No webhooks yet".to_string(),
            }
        }
    }
}

/// Single Notifications card with Telegram / Slack / Webhook tabs.
#[component]
fn NotificationDestinations(site_id: String) -> Element {
    let refresh = use_signal(|| 0u32);
    // 0 = Telegram (default), 1 = Slack, 2 = Webhook
    let mut tab = use_signal(|| 0usize);
    let channels = use_resource({
        let site_id = site_id.clone();
        move || {
            let _ = refresh();
            let path = format!("/api/v1/sites/{site_id}/alert-channels");
            async move { get_json::<ChannelsList>(&path).await }
        }
    });
    let active = tab();

    rsx! {
        TabbedCard {
            title: "Notifications".to_string(),
            tabs: vec![
                "Telegram".to_string(),
                "Slack".to_string(),
                "Webhook".to_string(),
            ],
            active,
            on_select: move |i| tab.set(i),
            csv_href: None,
            match active {
                1 => rsx! {
                    SlackPanel {
                        site_id: site_id.clone(),
                        channels,
                        refresh,
                    }
                },
                2 => rsx! {
                    WebhookPanel {
                        site_id: site_id.clone(),
                        channels,
                        refresh,
                    }
                },
                _ => rsx! {
                    TelegramPanel {
                        site_id: site_id.clone(),
                        channels,
                        refresh,
                    }
                },
            }
        }
    }
}

/// Per-site settings: general, digest, notification destinations, danger zone.
#[component]
pub fn SiteSettings(site_id: String) -> Element {
    let sites = use_sites_list();
    let site_name = site_from_resource(&sites, &site_id)
        .map(|s| s.name)
        .unwrap_or_else(|| site_id.clone());

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
                                NotificationDestinations { site_id: site_id.clone() }
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
                                                        invalidate_sites_cache();
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
