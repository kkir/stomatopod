use dioxus::prelude::*;

/// One point on the traffic timeseries chart's x-axis.
#[derive(Clone, PartialEq, Debug)]
pub struct ChartPoint {
    pub label: String,
    pub value: u64,
}

/// A dated marker drawn as a vertical dashed line, `x` already projected
/// onto the chart's 0-800 coordinate space (see `crates/web/src/routes/
/// dashboard.rs`'s `annotations_view` for the server-side equivalent).
#[derive(Clone, PartialEq, Debug)]
pub struct ChartAnnotation {
    pub x: f64,
    pub date: String,
    pub text: String,
}

/// Traffic-over-time chart, port of the 800x120 SVG in the legacy
/// site.jinja (lines 193-223): gradient polyline + area fill under it,
/// with dashed red vertical lines marking annotations.
#[component]
pub fn TimeseriesChart(points: Vec<ChartPoint>, annotations: Vec<ChartAnnotation>) -> Element {
    let n = points.len();
    let max_v = points.iter().map(|p| p.value).max().unwrap_or(1).max(1) as f64;
    let line_points = if n > 1 {
        points
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let x = (i as f64 / (n as f64 - 1.0)) * 800.0;
                let y = 120.0 - (p.value as f64 / max_v) * 110.0;
                format!("{x},{y}")
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        String::new()
    };

    rsx! {
        svg {
            view_box: "0 0 800 120",
            preserve_aspect_ratio: "none",
            class: "w-full h-[120px]",
            defs {
                linearGradient { id: "chart-line", x1: "0", y1: "0", x2: "1", y2: "0",
                    stop { offset: "0%", stop_color: "#5eead4" }
                    stop { offset: "55%", stop_color: "#22d3ee" }
                    stop { offset: "100%", stop_color: "#818cf8" }
                }
                linearGradient { id: "chart-area", x1: "0", y1: "0", x2: "0", y2: "1",
                    stop { offset: "0%", stop_color: "rgba(45, 212, 191, 0.22)" }
                    stop { offset: "100%", stop_color: "rgba(45, 212, 191, 0)" }
                }
            }
            if !line_points.is_empty() {
                polygon { points: "0,120 {line_points} 800,120", fill: "url(#chart-area)" }
                polyline {
                    points: "{line_points}",
                    fill: "none",
                    stroke: "url(#chart-line)",
                    stroke_width: "2.5",
                    stroke_linejoin: "round",
                    stroke_linecap: "round",
                }
            }
            for a in annotations {
                line {
                    key: "{a.date}-{a.text}",
                    x1: "{a.x}",
                    y1: "0",
                    x2: "{a.x}",
                    y2: "120",
                    stroke: "rgba(248,113,113,0.55)",
                    stroke_width: "1",
                    stroke_dasharray: "3 3",
                    title { "{a.date}: {a.text}" }
                }
            }
        }
    }
}

/// Tiny per-row trend chart, port of the `sparkline` macro in the legacy
/// site.jinja (lines 5-18): a 60x18 polyline scaled to the row's own max.
#[component]
pub fn Sparkline(points: Vec<f64>) -> Element {
    if points.len() < 2 {
        return rsx! {};
    }
    let n = points.len();
    let max_v = points.iter().cloned().fold(0.0_f64, f64::max).max(1.0);
    let line_points = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let x = (i as f64 / (n as f64 - 1.0)) * 60.0;
            let y = 16.0 - (p / max_v) * 14.0;
            format!("{x},{y}")
        })
        .collect::<Vec<_>>()
        .join(" ");

    rsx! {
        svg {
            class: "spark inline-block w-[60px] h-[18px] align-middle",
            view_box: "0 0 60 18",
            preserve_aspect_ratio: "none",
            "aria-hidden": "true",
            polyline {
                points: "{line_points}",
                fill: "none",
                stroke: "#22d3ee",
                stroke_width: "1.5",
                stroke_linejoin: "round",
                stroke_linecap: "round",
            }
        }
    }
}
