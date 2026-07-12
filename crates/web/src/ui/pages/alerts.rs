use dioxus::prelude::*;

use crate::ui::api::{delete, get_json, patch_json, post_json};
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::tabs::{SiteTab, SiteTabs};
use crate::ui::pages::{use_site_name, BTN_GHOST, BTN_PRIMARY, CTRL_INPUT};
use crate::ui::routes::Route;
use crate::ui::types::{AlertsList, ChannelsList, CreateAlertBody, PatchAlertBody};

const ALERT_KINDS: [(&str, &str); 3] = [
    ("traffic_spike", "Traffic spike"),
    ("traffic_drop", "Traffic drop"),
    ("new_referrer_spike", "New referrer spike"),
];

fn alert_kind_label(kind: &str) -> &'static str {
    ALERT_KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, l)| *l)
        .unwrap_or("Alert")
}

fn channel_label(kind: &str, url: &str) -> String {
    let kind_label = match kind {
        "telegram" => "Telegram",
        "slack" => "Slack",
        "webhook" => "Webhook",
        other => other,
    };
    format!("{kind_label} · {url}")
}

/// Analytics alerts for one site. Notification destinations live under
/// Settings (Telegram, Slack, webhooks).
#[component]
pub fn Alerts(site_id: String) -> Element {
    let site_name = use_site_name(site_id.clone());
    rsx! {
        PageHead { title: "Alerts", subtitle: "{site_name}" }
        SiteTabs { site_id: site_id.clone(), range: "30d", active: SiteTab::Alerts }
        AlertsCard { site_id }
    }
}

#[component]
fn AlertsCard(site_id: String) -> Element {
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

    let mut kind = use_signal(|| "traffic_spike".to_string());
    let mut threshold = use_signal(|| "100".to_string());
    let mut window = use_signal(|| "60".to_string());
    let mut channel_id = use_signal(String::new);

    let channel_options = match &*channels.read() {
        Some(Ok(list)) => list
            .channels
            .iter()
            .map(|c| (c.id.clone(), channel_label(&c.kind, &c.url)))
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
    let settings_href = Route::SiteSettings {
        site_id: site_id.clone(),
    };

    rsx! {
        Card { title: "Alerts",
            if !has_channels {
                div { class: "text-xs text-muted-1 mb-3",
                    "Add a notification destination under "
                    Link {
                        to: settings_href,
                        class: "text-teal-hi hover:underline",
                        "Settings"
                    }
                    " (Telegram, Slack, or webhook) before creating an alert."
                }
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
                        let body = CreateAlertBody {
                            alert_type: kind(),
                            threshold: threshold().trim().parse().unwrap_or(0.0),
                            window_minutes: window().trim().parse().unwrap_or(0),
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
