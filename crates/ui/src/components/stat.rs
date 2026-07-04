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

/// A single headline number, port of `.stat` (dashboard.css Stats
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
        div {
            div { class: "text-muted-1 text-[11px] uppercase tracking-[0.12em] font-semibold", "{label}" }
            div { class: "text-grad-value font-display text-[34px] font-bold tracking-tight tabular-nums mt-[5px]", "{value}" }
            if let Some(d) = delta {
                div {
                    class: match d.dir {
                        DeltaDir::Up => "inline-flex items-center gap-1 mt-1.5 text-xs font-semibold text-green",
                        DeltaDir::Down => "inline-flex items-center gap-1 mt-1.5 text-xs font-semibold text-red",
                        DeltaDir::New | DeltaDir::Flat => "inline-flex items-center gap-1 mt-1.5 text-xs font-semibold text-muted-2",
                    },
                    {match d.dir {
                        DeltaDir::Up => rsx! { "▲ {d.pct:.1}%" },
                        DeltaDir::Down => rsx! { "▼ {d.pct:.1}%" },
                        DeltaDir::New => rsx! { "New" },
                        DeltaDir::Flat => rsx! { "- {d.pct:.1}%" },
                    }}
                }
            }
            if let Some(p) = prev {
                div { class: "text-muted-2 text-[11px] mt-1", "prev {p}" }
            }
        }
    }
}
