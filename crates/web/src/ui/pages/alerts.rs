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

fn alert_kind_hint(kind: &str) -> &'static str {
    match kind {
        "traffic_spike" => "pageviews above the previous window",
        "traffic_drop" => "pageviews below the previous window",
        "new_referrer_spike" => "share of traffic from one referrer",
        _ => "threshold",
    }
}

fn threshold_suffix(kind: &str) -> &'static str {
    match kind {
        "new_referrer_spike" => "% of traffic",
        _ => "% change",
    }
}

/// Analytics alerts for one site. Notification destinations live under
/// Settings (Telegram, Slack, webhooks); each fire goes to every channel.
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

    let channels_loaded = (*channels.read()).is_some();
    let has_channels = match &*channels.read() {
        Some(Ok(list)) => !list.channels.is_empty(),
        _ => false,
    };
    let settings_href = Route::SiteSettings {
        site_id: site_id.clone(),
    };

    rsx! {
        Card { title: "Alerts",
            if channels_loaded && !has_channels {
                div {
                    class: "mb-4 rounded-lg border border-amber-500/25 bg-amber-500/5 px-3.5 py-3 text-[13px] leading-relaxed text-text-1",
                    p { class: "font-medium",
                        "No notification destinations set up"
                    }
                    p { class: "text-muted-1 mt-1",
                        "Alerts are evaluating in the background, but nothing will be delivered until you add a destination under "
                        Link {
                            to: settings_href.clone(),
                            class: "text-teal-hi hover:underline font-medium",
                            "Settings"
                        }
                        " (Telegram, Slack, or webhook)."
                    }
                }
            } else if has_channels {
                div { class: "text-xs text-muted-1 mb-3",
                    "When an alert fires it is sent to every notification destination under "
                    Link {
                        to: settings_href,
                        class: "text-teal-hi hover:underline",
                        "Settings"
                    }
                    "."
                }
            }
            form {
                class: "flex flex-wrap items-end gap-2 mb-4",
                onsubmit: {
                    let site_id = site_id.clone();
                    move |evt: FormEvent| {
                        evt.prevent_default();
                        if !has_channels {
                            return;
                        }
                        let body = CreateAlertBody {
                            alert_type: kind(),
                            threshold: threshold().trim().parse().unwrap_or(0.0),
                            window_minutes: window().trim().parse().unwrap_or(0),
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
                label { class: "flex flex-col gap-1.5",
                    span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "Type" }
                    select {
                        class: CTRL_INPUT,
                        value: "{kind}",
                        onchange: move |e| {
                            let v = e.value();
                            // Keep thresholds in a sensible range when switching kinds.
                            match v.as_str() {
                                "traffic_drop" => threshold.set("50".into()),
                                "new_referrer_spike" => threshold.set("35".into()),
                                _ => threshold.set("100".into()),
                            }
                            kind.set(v);
                        },
                        for (val , label) in ALERT_KINDS {
                            option { key: "{val}", value: "{val}", "{label}" }
                        }
                    }
                }
                label { class: "flex flex-col gap-1.5",
                    span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold",
                        "Threshold ({threshold_suffix(&kind())})"
                    }
                    input {
                        class: CTRL_INPUT,
                        r#type: "number",
                        value: "{threshold}",
                        placeholder: "100",
                        oninput: move |e| threshold.set(e.value()),
                    }
                }
                label { class: "flex flex-col gap-1.5",
                    span { class: "text-[0.72rem] tracking-[0.06em] uppercase text-muted-1 font-semibold", "Window (min)" }
                    input {
                        class: CTRL_INPUT,
                        r#type: "number",
                        value: "{window}",
                        placeholder: "60",
                        oninput: move |e| window.set(e.value()),
                    }
                }
                button {
                    r#type: "submit",
                    class: BTN_PRIMARY,
                    disabled: !has_channels,
                    title: if !has_channels {
                        "Add a notification destination under Settings first"
                    } else {
                        ""
                    },
                    "aria-disabled": if !has_channels { "true" },
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
                            EmptyState {
                                title: "No alerts yet",
                                message: "Starter traffic spike, drop, and referrer rules are stored when a site is created (delete any you do not need). Add more above once a notification destination is configured.",
                            }
                        }
                    } else {
                        rsx! {
                            div { class: "flex flex-col gap-2",
                                for alert in list.alerts.clone() {
                                    {
                                        let kind_label = alert_kind_label(&alert.kind);
                                        let toggle_label = if alert.enabled {
                                            format!("Disable {kind_label}")
                                        } else {
                                            format!("Enable {kind_label}")
                                        };
                                        let delete_label = format!("Delete {kind_label}");
                                        rsx! {
                                            div {
                                                key: "{alert.id}",
                                                class: "flex items-center justify-between gap-3 py-2 border-t border-border-1",
                                                div {
                                                    div { class: "text-text-1 text-[13px] font-medium",
                                                        "{kind_label}"
                                                        if !alert.enabled {
                                                            span { class: "ml-2 text-[11px] font-semibold uppercase tracking-wide text-muted-1",
                                                                "Off"
                                                            }
                                                        }
                                                    }
                                                    div { class: "text-muted-1 text-xs",
                                                        "{alert.config.threshold}% {alert_kind_hint(&alert.kind)} · {alert.config.window_minutes}m window"
                                                    }
                                                }
                                                div { class: "flex items-center gap-2",
                                                    button {
                                                        r#type: "button",
                                                        class: BTN_GHOST,
                                                        "aria-label": "{toggle_label}",
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
                                                        "aria-label": "{delete_label}",
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
                    }
                }
            }}
        }
    }
}
