use dioxus::prelude::*;
use stomatopod_ui::card::Card;

const GITHUB_URL: &str = "https://github.com/kkir/stomatopod";
const DEPLOY_URL: &str = "https://github.com/kkir/stomatopod/blob/main/DEPLOY.md";
const README_URL: &str = "https://github.com/kkir/stomatopod#readme";

#[component]
pub fn GetStarted() -> Element {
    rsx! {
        div { class: "mx-auto max-w-5xl px-4 sm:px-6 py-12 sm:py-16",
            p { class: "text-teal-hi text-[12px] font-semibold uppercase tracking-[0.16em] mb-3",
                "Install"
            }
            h1 { class: "font-display font-bold text-3xl sm:text-4xl tracking-tight text-text-1",
                "Get started"
            }
            p { class: "mt-4 text-text-2 text-[15px] leading-relaxed max-w-2xl",
                "Local development setup. You need a long auth secret, a strong first-boot admin password, and an admin email."
            }

            div { class: "mt-10 space-y-4",
                Card {
                    title: "1. Prerequisites".to_string(),
                    ul { class: "list-disc pl-5 space-y-1.5 text-[13px] text-text-2 leading-relaxed",
                        li {
                            "Install "
                            a {
                                class: "text-teal-hi underline",
                                href: "https://mise.jdx.dev/getting-started.html",
                                target: "_blank",
                                rel: "noopener noreferrer",
                                "mise"
                            }
                            "."
                        }
                    }
                }

                Card {
                    title: "2. Clone the repo".to_string(),
                    pre { class: "bg-black/40 rounded-lg p-3 sm:p-4 overflow-x-auto text-[12.5px] font-mono text-text-2 leading-relaxed",
                        {format!("git clone {GITHUB_URL}.git\ncd stomatopod")}
                    }
                }

                Card {
                    title: "3. Configure".to_string(),
                    pre { class: "bg-black/40 rounded-lg p-3 sm:p-4 overflow-x-auto text-[12.5px] font-mono text-text-2 leading-relaxed",
                        "mise install\n\
                         mise run config:init\n\
                         # Set auth.secret_key in stomatopod.toml, or:\n\
                         export STOMATOPOD_AUTH__SECRET_KEY=\"$(openssl rand -hex 32)\"\n\
                         export STOMATOPOD_ADMIN_PASSWORD=\"$(openssl rand -base64 24)\"\n\
                         export STOMATOPOD_ADMIN_EMAIL=\"you@example.com\""
                    }
                }

                Card {
                    title: "4. Run".to_string(),
                    pre { class: "bg-black/40 rounded-lg p-3 sm:p-4 overflow-x-auto text-[12.5px] font-mono text-text-2 leading-relaxed",
                        "mise run dev\n\
                         # Dashboard SSR at http://localhost:8080\n\
                         # JSON API at /api/v1"
                    }
                    p { class: "mt-3 text-muted-1 text-[13px]",
                        "Seed demo traffic with "
                        code { class: "bg-black/40 rounded px-1 py-0.5 font-mono text-[12px]", "mise run seed" }
                        " while the server is up."
                    }
                }

                Card {
                    title: "5. Production".to_string(),
                    p { class: "text-[13px] text-text-2 leading-relaxed mb-3",
                        "Use Docker Compose or the published image. Mount a volume at "
                        code { class: "bg-black/40 rounded px-1 py-0.5 font-mono text-[12px]", "/app/data" }
                        " so events survive restarts."
                    }
                    div { class: "flex flex-wrap gap-3",
                        a {
                            class: "inline-flex items-center px-[15px] py-2 rounded-[10px] text-[13px] font-semibold bg-grad-btn text-[#032621] shadow-glow no-underline",
                            href: "{DEPLOY_URL}",
                            target: "_blank",
                            rel: "noopener noreferrer",
                            "Read DEPLOY.md"
                        }
                        a {
                            class: "inline-flex items-center px-[15px] py-2 rounded-[10px] text-[13px] font-semibold border border-border-2 text-text-1 no-underline",
                            href: "{GITHUB_URL}",
                            target: "_blank",
                            rel: "noopener noreferrer",
                            "GitHub repository"
                        }
                        a {
                            class: "inline-flex items-center px-[15px] py-2 rounded-[10px] text-[13px] font-semibold border border-border-2 text-text-1 no-underline",
                            href: "{README_URL}",
                            target: "_blank",
                            rel: "noopener noreferrer",
                            "Full README"
                        }
                    }
                }
            }
        }
    }
}
