use dioxus::prelude::*;

use crate::ui::api::origin;
use crate::ui::components::card::Card;
use crate::ui::docs_anchors::{docs_href, INSTALLING_THE_BROWSER_TRACKER};
use crate::ui::pages::BTN_GHOST;

/// The tracker install card shown on a site's overview: the one-line script
/// snippet (pre-filled with this site's public key and this dashboard's
/// origin) plus a short "add it to your `<head>`" instruction and a link to
/// the full docs. `prominent` swaps in onboarding copy for a site that has no
/// data yet, so the empty overview doubles as a getting-started guide.
#[component]
pub fn InstallCard(public_key: String, domain: String, prominent: bool) -> Element {
    // Empty on SSR; hydration re-renders with the browser's real origin so the
    // snippet is copy-pasteable as-is.
    let host = origin();
    let src_host = if host.is_empty() {
        "https://your-stomatopod-host".to_string()
    } else {
        host
    };
    let snippet =
        format!("<script defer src=\"{src_host}/tracker.js\" data-site=\"{public_key}\"></script>");

    let title = if prominent {
        "Start collecting analytics"
    } else {
        "Install the tracker"
    };
    let copied = use_signal(|| false);

    rsx! {
        Card { title: title.to_string(),
            if prominent {
                p { class: "text-text-2 text-[13.5px] leading-relaxed mb-4",
                    "No data yet. Add the tracker snippet to "
                    span { class: "text-text-1 font-medium", "{domain}" }
                    " and pageviews will appear here within seconds - no cookies, no configuration."
                }
            } else {
                p { class: "text-muted-1 text-[12.5px] leading-relaxed mb-4",
                    "Paste this snippet into the "
                    code { class: "bg-black/40 rounded px-1 py-0.5 text-[12px] font-mono text-text-2", "<head>" }
                    " of every page on "
                    span { class: "text-text-2 font-medium", "{domain}" }
                    " you want to measure."
                }
            }

            pre { class: "bg-black/40 border border-border-1 rounded-lg p-3 overflow-x-auto mb-3",
                code { class: "text-[13px] font-mono text-teal-hi break-all whitespace-pre-wrap select-all",
                    "{snippet}"
                }
            }

            div { class: "flex flex-wrap items-center gap-x-4 gap-y-2 text-muted-1 text-[12px]",
                button {
                    r#type: "button",
                    class: BTN_GHOST,
                    onclick: {
                        let snippet = snippet.clone();
                        move |_| {
                            let snippet = snippet.clone();
                            let mut copied = copied;
                            spawn(async move {
                                if copy_text(&snippet).await {
                                    copied.set(true);
                                }
                            });
                        }
                    },
                    if copied() { "Copied" } else { "Copy snippet" }
                }
                span {
                    "data-site is your public key (safe to expose). Events post to the same host as the script."
                }
                a {
                    class: "text-teal-hi underline whitespace-nowrap",
                    href: "{docs_href(INSTALLING_THE_BROWSER_TRACKER)}",
                    "Full setup guide \u{2192}"
                }
            }
        }
    }
}

/// Best-effort clipboard write. No-ops on SSR / native.
async fn copy_text(text: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(window) = web_sys::window() else {
            return false;
        };
        let clipboard = window.navigator().clipboard();
        let promise = clipboard.write_text(text);
        wasm_bindgen_futures::JsFuture::from(promise).await.is_ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = text;
        false
    }
}
