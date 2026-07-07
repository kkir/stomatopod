use dioxus::prelude::*;

use crate::ui::api::{delete, get_json, patch_json, post_json};
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::tabs::{SiteTab, SiteTabs};
use crate::ui::pages::{use_site_name, BTN_GHOST, BTN_PRIMARY, CTRL_INPUT};
use crate::ui::types::{
    AlertsList, ChannelTestResult, ChannelsList, CreateAlertBody, CreateChannelBody, PatchAlertBody,
};

const ALERT_KINDS: [(&str, &str); 4] = [
    ("traffic_spike", "Traffic spike"),
    ("traffic_drop", "Traffic drop"),
    ("goal_threshold", "Goal threshold"),
    ("new_referrer_spike", "New referrer spike"),
];

const CHANNEL_KINDS: [(&str, &str); 3] = [
    ("webhook", "Webhook"),
    ("slack", "Slack"),
    ("telegram", "Telegram"),
];

fn alert_kind_label(kind: &str) -> &'static str {
    ALERT_KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, l)| *l)
        .unwrap_or("Alert")
}

/// The alert-channels + analytics-alerts manager for one site, shared by
/// the per-site [`Alerts`] page and the [`GlobalAlerts`](super::GlobalAlerts)
/// page. Ports the forms from the legacy alerts.jinja.
#[component]
pub fn AlertsManager(site_id: String) -> Element {
    let refresh = use_signal(|| 0u32);
    let channels = use_resource({
        let site_id = site_id.clone();
        move || {
            let _ = refresh();
            let path = format!("/api/v1/sites/{site_id}/alert-channels");
            async move { get_json::<ChannelsList>(&path).await }
        }
    });
    let alerts = use_resource({
        let site_id = site_id.clone();
        move || {
            let _ = refresh();
            let path = format!("/api/v1/sites/{site_id}/analytics-alerts");
            async move { get_json::<AlertsList>(&path).await }
        }
    });

    rsx! {
        div { class: "grid grid-cols-1 lg:grid-cols-2 gap-4",
            ChannelsCard { site_id: site_id.clone(), channels: channels, refresh }
            AlertsCard { site_id: site_id.clone(), channels: channels, alerts, refresh }
        }
    }
}

type ChannelsRes = Resource<Result<ChannelsList, crate::ui::api::ApiError>>;
type AlertsRes = Resource<Result<AlertsList, crate::ui::api::ApiError>>;

#[component]
fn ChannelsCard(site_id: String, channels: ChannelsRes, refresh: Signal<u32>) -> Element {
    let mut kind = use_signal(|| "webhook".to_string());
    let mut url = use_signal(String::new);
    let mut secret = use_signal(String::new);
    let mut status = use_signal(String::new);

    rsx! {
        Card { title: "Channels",
            form {
                class: "flex flex-wrap items-end gap-2 mb-4",
                onsubmit: {
                    let site_id = site_id.clone();
                    move |evt: FormEvent| {
                        evt.prevent_default();
                        let dest = url().trim().to_string();
                        if dest.is_empty() {
                            return;
                        }
                        let sec = secret().trim().to_string();
                        let body = CreateChannelBody {
                            kind: kind(),
                            url: dest,
                            secret: if sec.is_empty() { None } else { Some(sec) },
                        };
                        let site_id = site_id.clone();
                        spawn(async move {
                            let path = format!("/api/v1/sites/{site_id}/alert-channels");
                            if post_json::<_, serde_json::Value>(&path, &body).await.is_ok() {
                                url.set(String::new());
                                secret.set(String::new());
                                let mut r = refresh;
                                r += 1;
                            }
                        });
                    }
                },
                select {
                    class: CTRL_INPUT,
                    value: "{kind}",
                    onchange: move |e| kind.set(e.value()),
                    for (val , label) in CHANNEL_KINDS {
                        option { key: "{val}", value: "{val}", "{label}" }
                    }
                }
                input {
                    class: CTRL_INPUT,
                    r#type: "text",
                    value: "{url}",
                    placeholder: "Webhook URL / chat id",
                    oninput: move |e| url.set(e.value()),
                }
                input {
                    class: CTRL_INPUT,
                    r#type: "text",
                    value: "{secret}",
                    placeholder: "Secret / bot token (optional)",
                    oninput: move |e| secret.set(e.value()),
                }
                button { r#type: "submit", class: BTN_PRIMARY, "Add channel" }
            }
            if !status().is_empty() {
                div { class: "text-xs text-muted-1 mb-3", "{status}" }
            }
            {match &*channels.read() {
                None => rsx! {
                    Skeleton { lines: 2 }
                },
                Some(Err(e)) => rsx! {
                    EmptyState { message: format!("Failed to load channels ({e})") }
                },
                Some(Ok(list)) => {
                    if list.channels.is_empty() {
                        rsx! {
                            EmptyState { message: "No channels yet" }
                        }
                    } else {
                        rsx! {
                            div { class: "flex flex-col gap-2",
                                for ch in list.channels.clone() {
                                    div {
                                        key: "{ch.id}",
                                        class: "flex items-center justify-between gap-3 py-2 border-t border-border-1",
                                        div {
                                            div { class: "text-text-1 text-[13px] font-medium", "{ch.kind}" }
                                            div { class: "text-muted-1 text-xs max-w-[260px] overflow-hidden text-ellipsis whitespace-nowrap", "{ch.url}" }
                                        }
                                        div { class: "flex items-center gap-2",
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
                                                                "/api/v1/sites/{site_id}/alert-channels/{id}/test",
                                                            );
                                                            match post_json::<(), ChannelTestResult>(&path, &()).await {
                                                                Ok(r) if r.result == "ok" => status.set("Test delivered".to_string()),
                                                                Ok(r) => {
                                                                    status
                                                                        .set(
                                                                            format!("Test failed: {}", r.error.unwrap_or_default()),
                                                                        )
                                                                }
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
}

#[component]
fn AlertsCard(
    site_id: String,
    channels: ChannelsRes,
    alerts: AlertsRes,
    refresh: Signal<u32>,
) -> Element {
    let mut kind = use_signal(|| "traffic_spike".to_string());
    let mut threshold = use_signal(|| "100".to_string());
    let mut window = use_signal(|| "60".to_string());
    let mut goal_event = use_signal(String::new);
    let mut channel_id = use_signal(String::new);

    // Channel <select> options, plus a default selection once channels load.
    let channel_options = match &*channels.read() {
        Some(Ok(list)) => list
            .channels
            .iter()
            .map(|c| (c.id.clone(), format!("{} · {}", c.kind, c.url)))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    // Effective selection: the user's pick, or the first channel until they
    // choose. Derived (not written to the signal during render) to avoid a
    // re-render loop.
    let effective_channel = if channel_id().is_empty() {
        channel_options
            .first()
            .map(|(v, _)| v.clone())
            .unwrap_or_default()
    } else {
        channel_id()
    };
    let has_channels = !channel_options.is_empty();
    let is_goal = kind() == "goal_threshold";

    rsx! {
        Card { title: "Alerts",
            if !has_channels {
                div { class: "text-xs text-muted-1 mb-3", "Add a channel first to create an alert." }
            }
            form {
                class: "flex flex-wrap items-end gap-2 mb-4",
                onsubmit: {
                    let site_id = site_id.clone();
                    let effective_channel = effective_channel.clone();
                    move |evt: FormEvent| {
                        evt.prevent_default();
                        let chan = effective_channel.clone();
                        if chan.is_empty() {
                            return;
                        }
                        let goal = goal_event().trim().to_string();
                        let body = CreateAlertBody {
                            alert_type: kind(),
                            threshold: threshold().trim().parse().unwrap_or(0.0),
                            window_minutes: window().trim().parse().unwrap_or(0),
                            goal_event_name: if is_goal && !goal.is_empty() {
                                Some(goal)
                            } else {
                                None
                            },
                            channel_id: chan,
                        };
                        let site_id = site_id.clone();
                        spawn(async move {
                            let path = format!("/api/v1/sites/{site_id}/analytics-alerts");
                            if post_json::<_, serde_json::Value>(&path, &body).await.is_ok() {
                                let mut r = refresh;
                                r += 1;
                            }
                        });
                    }
                },
                select {
                    class: CTRL_INPUT,
                    value: "{kind}",
                    onchange: move |e| kind.set(e.value()),
                    for (val , label) in ALERT_KINDS {
                        option { key: "{val}", value: "{val}", "{label}" }
                    }
                }
                input {
                    class: CTRL_INPUT,
                    r#type: "number",
                    value: "{threshold}",
                    placeholder: "threshold",
                    oninput: move |e| threshold.set(e.value()),
                }
                input {
                    class: CTRL_INPUT,
                    r#type: "number",
                    value: "{window}",
                    placeholder: "window (min)",
                    oninput: move |e| window.set(e.value()),
                }
                if is_goal {
                    input {
                        class: CTRL_INPUT,
                        r#type: "text",
                        value: "{goal_event}",
                        placeholder: "goal event name",
                        oninput: move |e| goal_event.set(e.value()),
                    }
                }
                select {
                    class: CTRL_INPUT,
                    value: "{effective_channel}",
                    onchange: move |e| channel_id.set(e.value()),
                    for (val , label) in channel_options.clone() {
                        option { key: "{val}", value: "{val}", "{label}" }
                    }
                }
                button {
                    r#type: "submit",
                    class: BTN_PRIMARY,
                    disabled: !has_channels,
                    "Add alert"
                }
            }
            {match &*alerts.read() {
                None => rsx! {
                    Skeleton { lines: 2 }
                },
                Some(Err(e)) => rsx! {
                    EmptyState { message: format!("Failed to load alerts ({e})") }
                },
                Some(Ok(list)) => {
                    if list.alerts.is_empty() {
                        rsx! {
                            EmptyState { message: "No alerts yet" }
                        }
                    } else {
                        rsx! {
                            div { class: "flex flex-col gap-2",
                                for alert in list.alerts.clone() {
                                    div {
                                        key: "{alert.id}",
                                        class: "flex items-center justify-between gap-3 py-2 border-t border-border-1",
                                        div {
                                            div { class: "text-text-1 text-[13px] font-medium",
                                                "{alert_kind_label(&alert.kind)}"
                                            }
                                            div { class: "text-muted-1 text-xs",
                                                "threshold {alert.config.threshold} · {alert.config.window_minutes}m"
                                            }
                                        }
                                        div { class: "flex items-center gap-2",
                                            button {
                                                r#type: "button",
                                                class: BTN_GHOST,
                                                onclick: {
                                                    let site_id = site_id.clone();
                                                    let id = alert.id.clone();
                                                    let enabled = alert.enabled;
                                                    move |_| {
                                                        let site_id = site_id.clone();
                                                        let id = id.clone();
                                                        spawn(async move {
                                                            let path = format!(
                                                                "/api/v1/sites/{site_id}/analytics-alerts/{id}",
                                                            );
                                                            let body = PatchAlertBody { enabled: !enabled };
                                                            if patch_json::<_, serde_json::Value>(&path, &body).await.is_ok() {
                                                                let mut r = refresh;
                                                                r += 1;
                                                            }
                                                        });
                                                    }
                                                },
                                                if alert.enabled { "Disable" } else { "Enable" }
                                            }
                                            button {
                                                r#type: "button",
                                                class: BTN_GHOST,
                                                onclick: {
                                                    let site_id = site_id.clone();
                                                    let id = alert.id.clone();
                                                    move |_| {
                                                        let site_id = site_id.clone();
                                                        let id = id.clone();
                                                        spawn(async move {
                                                            let path = format!(
                                                                "/api/v1/sites/{site_id}/analytics-alerts/{id}",
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
}

/// Per-site analytics-alerts + channels page.
#[component]
pub fn Alerts(site_id: String) -> Element {
    let site_name = use_site_name(site_id.clone());
    rsx! {
        PageHead { title: "Alerts", subtitle: "{site_name}" }
        SiteTabs { site_id: site_id.clone(), range: "30d", active: SiteTab::Alerts }
        AlertsManager { site_id }
    }
}
