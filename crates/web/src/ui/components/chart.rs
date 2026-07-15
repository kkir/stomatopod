use dioxus::prelude::*;

/// One point on the traffic timeseries chart's x-axis.
#[derive(Clone, PartialEq, Debug)]
pub struct ChartPoint {
    pub label: String,
    pub pageviews: u64,
    pub sessions: u64,
}

/// Chart plot height in SVG units (also drives the rendered CSS height).
const CHART_H: f64 = 168.0;

fn series_polyline(values: &[u64], max_v: f64, height: f64) -> String {
    let n = values.len();
    if n < 2 {
        return String::new();
    }
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = (i as f64 / (n as f64 - 1.0)) * 800.0;
            let y = height - (*v as f64 / max_v) * (height - 12.0);
            format!("{x},{y}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Align a prior-period series to the current series length by relative
/// offset (day 0 of prior vs day 0 of current). Missing buckets pad as 0.
fn align_previous(current_len: usize, previous: &[ChartPoint]) -> (Vec<u64>, Vec<u64>) {
    let mut pvs = Vec::with_capacity(current_len);
    let mut sess = Vec::with_capacity(current_len);
    for i in 0..current_len {
        match previous.get(i) {
            Some(p) => {
                pvs.push(p.pageviews);
                sess.push(p.sessions);
            }
            None => {
                pvs.push(0);
                sess.push(0);
            }
        }
    }
    (pvs, sess)
}

/// Traffic-over-time chart: pageviews and sessions as dual series,
/// with end-cap labels, legend, and a hover tooltip for the nearest day.
///
/// When `previous` is set (compare mode), prior-period series are drawn as
/// dashed lines aligned by relative day index.
#[component]
pub fn TimeseriesChart(points: Vec<ChartPoint>, previous: Option<Vec<ChartPoint>>) -> Element {
    let n = points.len();
    let comparing = previous.as_ref().is_some_and(|p| !p.is_empty());
    let (prev_pv_vals, prev_sess_vals) = previous
        .as_ref()
        .map(|p| align_previous(n, p))
        .unwrap_or_default();

    let max_v = points
        .iter()
        .map(|p| p.pageviews.max(p.sessions))
        .chain(prev_pv_vals.iter().copied())
        .chain(prev_sess_vals.iter().copied())
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    let all_zero = points.iter().all(|p| p.pageviews == 0 && p.sessions == 0)
        && prev_pv_vals.iter().all(|&v| v == 0)
        && prev_sess_vals.iter().all(|&v| v == 0);
    let mut hover = use_signal(|| None::<usize>);

    if n == 0 || all_zero {
        return rsx! {
            div { class: "flex items-center justify-center h-[180px] text-muted-1 text-[13px]",
                "No traffic in this range yet"
            }
        };
    }

    let pageview_vals: Vec<u64> = points.iter().map(|p| p.pageviews).collect();
    let session_vals: Vec<u64> = points.iter().map(|p| p.sessions).collect();
    let pv_line = series_polyline(&pageview_vals, max_v, CHART_H);
    let sess_line = series_polyline(&session_vals, max_v, CHART_H);
    let prev_pv_line = if comparing {
        series_polyline(&prev_pv_vals, max_v, CHART_H)
    } else {
        String::new()
    };
    let prev_sess_line = if comparing {
        series_polyline(&prev_sess_vals, max_v, CHART_H)
    } else {
        String::new()
    };

    let first_label = points.first().map(|p| p.label.clone()).unwrap_or_default();
    let last_label = points.last().map(|p| p.label.clone()).unwrap_or_default();

    let hover_info = hover().and_then(|i| {
        points.get(i).cloned().map(|p| {
            let prev = if comparing {
                Some((
                    prev_pv_vals.get(i).copied().unwrap_or(0),
                    prev_sess_vals.get(i).copied().unwrap_or(0),
                ))
            } else {
                None
            };
            (i, p, prev)
        })
    });

    let plot_h = CHART_H;
    let plot_bottom = CHART_H;
    let y_scale = CHART_H - 12.0;

    let total_pv: u64 = pageview_vals.iter().sum();
    let total_sess: u64 = session_vals.iter().sum();
    let chart_summary = format!(
        "Traffic chart from {first_label} to {last_label}: {total_pv} pageviews, {total_sess} sessions"
    );
    let hover_live = hover_info.as_ref().map(|(_, p, prev)| {
        let mut s = format!(
            "{}, {} pageviews, {} sessions",
            p.label, p.pageviews, p.sessions
        );
        if let Some((ppv, psess)) = prev {
            s.push_str(&format!("; prior {ppv} pageviews, {psess} sessions"));
        }
        s
    });

    rsx! {
        div {
            class: "relative",
            role: "img",
            "aria-label": "{chart_summary}",
            // Legend
            div {
                class: "flex items-center gap-5 mb-4 flex-wrap",
                "aria-hidden": "true",
                span { class: "inline-flex items-center gap-2 text-[12px] font-semibold text-muted-1",
                    span {
                        class: "inline-block w-3.5 h-0.5 rounded-full",
                        style: "background: linear-gradient(90deg, #5eead4, #22d3ee)",
                    }
                    "Pageviews"
                }
                span { class: "inline-flex items-center gap-2 text-[12px] font-semibold text-muted-1",
                    span {
                        class: "inline-block w-3.5 h-0.5 rounded-full bg-indigo-400",
                    }
                    "Sessions"
                }
                if comparing {
                    span { class: "inline-flex items-center gap-2 text-[12px] font-semibold text-muted-1",
                        span {
                            class: "inline-block w-3.5 border-t border-dashed border-teal-hi/70",
                        }
                        "Prior pageviews"
                    }
                    span { class: "inline-flex items-center gap-2 text-[12px] font-semibold text-muted-1",
                        span {
                            class: "inline-block w-3.5 border-t border-dashed border-indigo-400/70",
                        }
                        "Prior sessions"
                    }
                }
            }
            // Screen-reader data table (hidden visually; chart is decorative).
            table { class: "sr-only",
                caption { "Daily traffic" }
                thead {
                    tr {
                        th { scope: "col", "Date" }
                        th { scope: "col", "Pageviews" }
                        th { scope: "col", "Sessions" }
                    }
                }
                tbody {
                    for p in points.iter() {
                        tr {
                            th { scope: "row", "{p.label}" }
                            td { "{p.pageviews}" }
                            td { "{p.sessions}" }
                        }
                    }
                }
            }
            svg {
                view_box: "0 0 800 {plot_h}",
                preserve_aspect_ratio: "none",
                class: "w-full h-[180px]",
                "aria-hidden": "true",
                onmouseleave: move |_| hover.set(None),
                defs {
                    linearGradient { id: "chart-line-pv", x1: "0", y1: "0", x2: "1", y2: "0",
                        stop { offset: "0%", stop_color: "#5eead4" }
                        stop { offset: "55%", stop_color: "#22d3ee" }
                        stop { offset: "100%", stop_color: "#67e8f9" }
                    }
                    linearGradient { id: "chart-area-pv", x1: "0", y1: "0", x2: "0", y2: "1",
                        stop { offset: "0%", stop_color: "rgba(45, 212, 191, 0.20)" }
                        stop { offset: "100%", stop_color: "rgba(45, 212, 191, 0)" }
                    }
                }
                // Prior period first (under current), dashed, no fill
                if !prev_pv_line.is_empty() {
                    polyline {
                        points: "{prev_pv_line}",
                        fill: "none",
                        stroke: "rgba(94, 234, 212, 0.55)",
                        stroke_width: "2",
                        stroke_linejoin: "round",
                        stroke_linecap: "round",
                        stroke_dasharray: "6 4",
                    }
                }
                if !prev_sess_line.is_empty() {
                    polyline {
                        points: "{prev_sess_line}",
                        fill: "none",
                        stroke: "rgba(129, 140, 248, 0.55)",
                        stroke_width: "1.75",
                        stroke_linejoin: "round",
                        stroke_linecap: "round",
                        stroke_dasharray: "6 4",
                    }
                }
                // Current pageviews: filled area + solid line
                if !pv_line.is_empty() {
                    polygon {
                        points: "0,{plot_bottom} {pv_line} 800,{plot_bottom}",
                        fill: "url(#chart-area-pv)",
                    }
                    polyline {
                        points: "{pv_line}",
                        fill: "none",
                        stroke: "url(#chart-line-pv)",
                        stroke_width: "2.5",
                        stroke_linejoin: "round",
                        stroke_linecap: "round",
                    }
                }
                // Current sessions: solid indigo
                if !sess_line.is_empty() {
                    polyline {
                        points: "{sess_line}",
                        fill: "none",
                        stroke: "#818cf8",
                        stroke_width: "2",
                        stroke_linejoin: "round",
                        stroke_linecap: "round",
                        opacity: "0.95",
                    }
                }
                // Transparent hit strips for hover
                if n > 1 {
                    for i in 0..n {
                        {
                            let x = (i as f64 / (n as f64 - 1.0)) * 800.0;
                            let strip_w = (800.0 / (n as f64 - 1.0)).max(8.0);
                            let left = (x - strip_w / 2.0).max(0.0);
                            rsx! {
                                rect {
                                    key: "{i}",
                                    x: "{left}",
                                    y: "0",
                                    width: "{strip_w}",
                                    height: "{plot_h}",
                                    fill: "transparent",
                                    onmouseenter: move |_| hover.set(Some(i)),
                                }
                            }
                        }
                    }
                }
                if let Some((i, p, prev)) = &hover_info {
                    {
                        let x = (*i as f64 / (n as f64 - 1.0).max(1.0)) * 800.0;
                        let y_pv = plot_h - (p.pageviews as f64 / max_v) * y_scale;
                        let y_sess = plot_h - (p.sessions as f64 / max_v) * y_scale;
                        rsx! {
                            line {
                                x1: "{x}", y1: "0", x2: "{x}", y2: "{plot_h}",
                                stroke: "rgba(148, 163, 184, 0.35)",
                                stroke_width: "1",
                                stroke_dasharray: "3 3",
                            }
                            circle {
                                cx: "{x}", cy: "{y_pv}", r: "4",
                                fill: "#5eead4",
                                stroke: "#04080b",
                                stroke_width: "1.5",
                            }
                            circle {
                                cx: "{x}", cy: "{y_sess}", r: "3.5",
                                fill: "#818cf8",
                                stroke: "#04080b",
                                stroke_width: "1.5",
                            }
                            if let Some((ppv, psess)) = prev {
                                {
                                    let y_ppv = plot_h - (*ppv as f64 / max_v) * y_scale;
                                    let y_ps = plot_h - (*psess as f64 / max_v) * y_scale;
                                    rsx! {
                                        circle {
                                            cx: "{x}", cy: "{y_ppv}", r: "3",
                                            fill: "none",
                                            stroke: "rgba(94, 234, 212, 0.9)",
                                            stroke_width: "1.5",
                                            stroke_dasharray: "2 1.5",
                                        }
                                        circle {
                                            cx: "{x}", cy: "{y_ps}", r: "2.75",
                                            fill: "none",
                                            stroke: "rgba(129, 140, 248, 0.9)",
                                            stroke_width: "1.5",
                                            stroke_dasharray: "2 1.5",
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let Some((_, p, prev)) = &hover_info {
                div {
                    class: "pointer-events-none absolute top-10 left-1/2 -translate-x-1/2 max-w-[calc(100%-0.5rem)] px-2.5 sm:px-3 py-2 rounded-lg bg-surface-2 border border-border-2 text-[11px] sm:text-[11.5px] font-semibold text-text-1 shadow-sm tabular-nums",
                    "aria-hidden": "true",
                    div { class: "text-center text-muted-1 mb-1", "{p.label}" }
                    div { class: "flex flex-wrap gap-x-3 gap-y-0.5 justify-center",
                        span { class: "text-teal-hi", "{p.pageviews} pageviews" }
                        span { class: "text-indigo-300", "{p.sessions} sessions" }
                    }
                    if let Some((ppv, psess)) = prev {
                        div { class: "flex flex-wrap gap-x-3 gap-y-0.5 justify-center mt-1 text-muted-1 font-medium",
                            span { "{ppv} pageviews" }
                            span { "{psess} sessions" }
                        }
                    }
                }
            }
            // Live region mirrors the hover tooltip for keyboard/AT users that
            // still interact via the pointer (tooltip is aria-hidden).
            div {
                class: "sr-only",
                role: "status",
                "aria-live": "polite",
                if let Some(text) = hover_live {
                    "{text}"
                }
            }
            div {
                class: "flex justify-between mt-2.5 px-0.5",
                "aria-hidden": "true",
                span { class: "text-muted-2 text-[11px] tabular-nums", "{first_label}" }
                span { class: "text-muted-2 text-[11px] tabular-nums", "{last_label}" }
            }
        }
    }
}

/// Tiny trend chart, port of the `sparkline` macro in the legacy site.jinja
/// (lines 5-18): a polyline scaled to the series' own max. Defaults to the
/// table-row size (60×18); pass larger `width`/`height` for site cards.
#[component]
pub fn Sparkline(
    points: Vec<f64>,
    #[props(default = 60.0)] width: f64,
    #[props(default = 18.0)] height: f64,
) -> Element {
    if points.len() < 2 {
        return rsx! {};
    }
    let n = points.len();
    let max_v = points.iter().cloned().fold(0.0_f64, f64::max).max(1.0);
    let pad_y = 2.0;
    let line_points = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let x = (i as f64 / (n as f64 - 1.0)) * width;
            let y = height - pad_y - (p / max_v) * (height - pad_y * 2.0);
            format!("{x},{y}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let vb = format!("0 0 {width} {height}");
    let style = format!("width: {width}px; height: {height}px");

    rsx! {
        svg {
            class: "spark inline-block align-middle",
            style: "{style}",
            view_box: "{vb}",
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
