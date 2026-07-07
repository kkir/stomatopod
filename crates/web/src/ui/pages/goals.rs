use dioxus::prelude::*;

use crate::ui::api::{delete, get_json, post_json};
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::layout::PageHead;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::tabs::{RangeTabs, SiteTab, SiteTabs};
use crate::ui::pages::{active_filters, use_site_name, BTN_GHOST, BTN_PRIMARY, CTRL_INPUT};
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
use crate::ui::types::{CreateGoalBody, Goal, GoalStats, GoalsList};

/// One goal row: name + target event plus its own lazily-fetched
/// conversion stats for `range`, and a delete button.
#[component]
fn GoalCard(site_id: String, range: String, goal: Goal, on_change: EventHandler<()>) -> Element {
    let stats = use_resource({
        let site_id = site_id.clone();
        let goal_id = goal.id.clone();
        let range = range.clone();
        move || {
            let path = format!("/api/v1/sites/{site_id}/goals/{goal_id}/stats?range={range}");
            async move { get_json::<GoalStats>(&path).await }
        }
    });

    let (completions, unique, rate) = match &*stats.read() {
        Some(Ok(s)) => (
            format!("{}", s.completions),
            format!("{}", s.unique_completions),
            format!("{:.1}%", s.conversion_rate),
        ),
        _ => ("-".to_string(), "-".to_string(), "-".to_string()),
    };

    rsx! {
        Card {
            div { class: "flex justify-between items-start gap-4",
                div {
                    div { class: "text-[15px] font-semibold text-text-1", "{goal.name}" }
                    div { class: "text-muted-1 text-xs mt-1", "Event: {goal.event_name}" }
                }
                button {
                    r#type: "button",
                    class: BTN_GHOST,
                    onclick: {
                        let site_id = site_id.clone();
                        let goal_id = goal.id.clone();
                        move |_| {
                            let site_id = site_id.clone();
                            let goal_id = goal_id.clone();
                            spawn(async move {
                                let path = format!("/api/v1/sites/{site_id}/goals/{goal_id}");
                                if delete(&path).await.is_ok() {
                                    on_change.call(());
                                }
                            });
                        }
                    },
                    "Delete"
                }
            }
            div { class: "grid grid-cols-3 gap-4 mt-4",
                div {
                    div { class: "text-muted-1 text-[11px] uppercase tracking-[0.12em] font-semibold", "Completions" }
                    div { class: "text-text-1 font-display text-2xl font-bold tabular-nums mt-1", "{completions}" }
                }
                div {
                    div { class: "text-muted-1 text-[11px] uppercase tracking-[0.12em] font-semibold", "Unique" }
                    div { class: "text-text-1 font-display text-2xl font-bold tabular-nums mt-1", "{unique}" }
                }
                div {
                    div { class: "text-muted-1 text-[11px] uppercase tracking-[0.12em] font-semibold", "Conversion" }
                    div { class: "text-text-1 font-display text-2xl font-bold tabular-nums mt-1", "{rate}" }
                }
            }
        }
    }
}

/// Goals list + create form + per-goal conversion stats, shared by the
/// per-site [`Goals`] page and the global goals page.
#[component]
pub fn GoalsPanel(site_id: String, range: String) -> Element {
    let refresh = use_signal(|| 0u32);
    let goals = use_resource({
        let site_id = site_id.clone();
        move || {
            let _ = refresh();
            let path = format!("/api/v1/sites/{site_id}/goals");
            async move { get_json::<GoalsList>(&path).await }
        }
    });

    let mut g_name = use_signal(String::new);
    let mut g_event = use_signal(String::new);

    rsx! {
        Card { title: "New goal",
            form {
                class: "flex flex-wrap items-end gap-2",
                onsubmit: {
                    let site_id = site_id.clone();
                    move |evt: FormEvent| {
                        evt.prevent_default();
                        let name = g_name().trim().to_string();
                        let event_name = g_event().trim().to_string();
                        if name.is_empty() || event_name.is_empty() {
                            return;
                        }
                        let site_id = site_id.clone();
                        spawn(async move {
                            let path = format!("/api/v1/sites/{site_id}/goals");
                            let body = CreateGoalBody { name, event_name };
                            if post_json::<_, serde_json::Value>(&path, &body).await.is_ok() {
                                g_name.set(String::new());
                                g_event.set(String::new());
                                let mut r = refresh;
                                r += 1;
                            }
                        });
                    }
                },
                input {
                    class: CTRL_INPUT,
                    r#type: "text",
                    value: "{g_name}",
                    placeholder: "Goal name",
                    oninput: move |e| g_name.set(e.value()),
                }
                input {
                    class: CTRL_INPUT,
                    r#type: "text",
                    value: "{g_event}",
                    placeholder: "Event name (e.g. signup)",
                    oninput: move |e| g_event.set(e.value()),
                }
                button { r#type: "submit", class: BTN_PRIMARY, "Add goal" }
            }
        }

        div { class: "grid grid-cols-1 md:grid-cols-2 gap-4 mt-4",
            {match &*goals.read() {
                None => rsx! {
                    Skeleton { lines: 4 }
                },
                Some(Err(e)) => rsx! {
                    Card { EmptyState { message: format!("Failed to load goals ({e})") } }
                },
                Some(Ok(list)) => {
                    if list.goals.is_empty() {
                        rsx! {
                            Card {
                                EmptyState {
                                    title: "Set your first goal",
                                    message: "Goals measure how often visitors complete an action you care about - a signup, a purchase, a plan upgrade - and track its conversion rate over time. Create one with the form above.",
                                }
                            }
                        }
                    } else {
                        rsx! {
                            for goal in list.goals.clone() {
                                GoalCard {
                                    key: "{goal.id}",
                                    site_id: site_id.clone(),
                                    range: range.clone(),
                                    goal,
                                    on_change: move |_| {
                                        let mut r = refresh;
                                        r += 1;
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

/// Per-site goals page: tab row + range tabs + [`GoalsPanel`].
#[component]
pub fn Goals(site_id: String, q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let site_name = use_site_name(site_id.clone());
    let range = q.range.clone().unwrap_or_else(|| "30d".to_string());

    rsx! {
        PageHead { title: "Goals", subtitle: "{site_name}",
            RangeTabs { active: range.clone() }
        }
        SiteTabs { site_id: site_id.clone(), range: range.clone(), active: SiteTab::Goals }
        {active_filters(&route, &q)}
        GoalsPanel { site_id, range }
    }
}
