//! Auto-refetch interval control for live dashboard pages.

use dioxus::prelude::*;

use crate::ui::pages::CTRL_TOOLBAR;

#[cfg(target_arch = "wasm32")]
const STORAGE_KEY: &str = "stomatopod.auto_refresh_secs";

/// Off / 1s / 5s / 30s / 1min.
const OPTIONS: [(u32, &str); 5] = [
    (0, "Manual"),
    (1, "Every 1s"),
    (5, "Every 5s"),
    (30, "Every 30s"),
    (60, "Every 1 min"),
];

fn load_interval() -> u32 {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(window) = web_sys::window() {
            if let Ok(Some(storage)) = window.local_storage() {
                if let Ok(Some(raw)) = storage.get_item(STORAGE_KEY) {
                    if let Ok(v) = raw.parse::<u32>() {
                        if OPTIONS.iter().any(|(s, _)| *s == v) {
                            return v;
                        }
                    }
                }
            }
        }
    }
    0
}

fn save_interval(secs: u32) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(window) = web_sys::window() {
            if let Ok(Some(storage)) = window.local_storage() {
                let _ = storage.set_item(STORAGE_KEY, &secs.to_string());
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = secs;
    }
}

/// Dropdown that periodically bumps `tick` so dependent `use_resource`s
/// re-fetch. Preference is stored in `localStorage`.
///
/// Styled as a toolbar control (same height as range tabs / compare). While
/// auto-refresh is on, a teal status dot sits inside the select and blips
/// on each tick.
#[component]
pub fn AutoRefresh(mut tick: Signal<u32>) -> Element {
    let mut interval_secs = use_signal(load_interval);
    let mut generation = use_signal(|| 0u32);
    let blip = use_signal(|| 0u32);

    use_effect(move || {
        let secs = interval_secs();
        let gen = generation();
        if secs == 0 {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let mut tick = tick;
            let mut blip = blip;
            spawn(async move {
                loop {
                    gloo_timers::future::TimeoutFuture::new(secs.saturating_mul(1000)).await;
                    if generation() != gen {
                        break;
                    }
                    tick.with_mut(|n| *n = n.wrapping_add(1));
                    blip.with_mut(|n| *n = n.wrapping_add(1));
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (secs, gen, tick, blip);
        }
    });

    let live = interval_secs() > 0;
    let blip_n = blip();
    let select_pad = if live { "pl-8" } else { "pl-3.5" };

    rsx! {
        span {
            class: "relative inline-flex items-center shrink-0",
            title: "Auto-refresh interval",
            if live {
                span {
                    class: "pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 z-[1] inline-flex items-center justify-center w-[9px] h-[9px]",
                    "aria-hidden": "true",
                    span { class: "refresh-dot refresh-dot-live" }
                    if blip_n > 0 {
                        span {
                            key: "{blip_n}",
                            class: "absolute inset-0 m-auto refresh-dot refresh-dot-blip",
                        }
                    }
                }
            }
            select {
                class: "{CTRL_TOOLBAR} pr-8 min-w-[8.25rem] {select_pad}",
                value: "{interval_secs}",
                "aria-label": "Auto refresh interval",
                onchange: move |e| {
                    let v = e.value().parse::<u32>().unwrap_or(0);
                    let v = if OPTIONS.iter().any(|(s, _)| *s == v) { v } else { 0 };
                    interval_secs.set(v);
                    save_interval(v);
                    generation.with_mut(|g| *g = g.wrapping_add(1));
                },
                for (secs, label) in OPTIONS {
                    option { value: "{secs}", selected: interval_secs() == secs, "{label}" }
                }
            }
            // Custom chevron so the control matches the rest of the toolbar
            // (native arrows vary and sit poorly with the live dot).
            span {
                class: "pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-2 text-[9px] leading-none",
                "aria-hidden": "true",
                "▾"
            }
        }
    }
}
