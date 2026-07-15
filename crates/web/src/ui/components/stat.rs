use dioxus::prelude::*;

/// Direction of a stat's delta vs. the previous period, port of the
/// `delta_badge` macro in the legacy site.jinja.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DeltaDir {
    Up,
    Down,
    New,
    Flat,
}

#[derive(Clone, PartialEq, Debug)]
pub struct DeltaInfo {
    pub dir: DeltaDir,
    pub pct: f64,
}

/// A single headline number, port of `.stat` (legacy dashboard stylesheet Stats
/// section). Used for the site-overview hero and any global summary
/// cards.
#[component]
pub fn StatTile(
    label: String,
    value: String,
    delta: Option<DeltaInfo>,
    prev: Option<String>,
) -> Element {
    rsx! {
        div { class: "min-w-0",
            div { class: "text-muted-1 text-[11px] uppercase tracking-[0.14em] font-semibold", "{label}" }
            // Transparent clip-text can drop the value for some ATTs; keep a
            // plain value in an sr-only node and hide the decorative gradient.
            span { class: "sr-only", "{value}" }
            div {
                class: "text-grad-value font-display text-[28px] sm:text-[40px] font-bold tracking-tight tabular-nums mt-1.5 leading-none break-all",
                "aria-hidden": "true",
                "{value}"
            }
            if let Some(d) = delta {
                {
                    let (visible, spoken) = match d.dir {
                        DeltaDir::Up => (
                            format!("▲ {:.1}%", d.pct),
                            format!("up {:.1}% versus previous period", d.pct),
                        ),
                        DeltaDir::Down => (
                            format!("▼ {:.1}%", d.pct),
                            format!("down {:.1}% versus previous period", d.pct),
                        ),
                        DeltaDir::New => ("New".to_string(), "new versus previous period".to_string()),
                        DeltaDir::Flat => (
                            format!("- {:.1}%", d.pct),
                            format!("unchanged ({:.1}%) versus previous period", d.pct),
                        ),
                    };
                    rsx! {
                        div {
                            class: match d.dir {
                                DeltaDir::Up => "inline-flex items-center gap-1 mt-2.5 text-[12.5px] font-semibold text-green",
                                DeltaDir::Down => "inline-flex items-center gap-1 mt-2.5 text-[12.5px] font-semibold text-red",
                                DeltaDir::New | DeltaDir::Flat => "inline-flex items-center gap-1 mt-2.5 text-[12.5px] font-semibold text-muted-2",
                            },
                            span { class: "sr-only", "{spoken}" }
                            span { "aria-hidden": "true", "{visible}" }
                        }
                    }
                }
            }
            if let Some(p) = prev {
                div { class: "text-muted-2 text-[11.5px] mt-1", "prev {p}" }
            }
        }
    }
}
