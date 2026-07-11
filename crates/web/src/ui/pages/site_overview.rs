use dioxus::prelude::*;

use crate::ui::api::get_json;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::chart::{ChartPoint, TimeseriesChart};
use crate::ui::components::install::InstallCard;
use crate::ui::components::layout::PageHead;
use crate::ui::components::refresh::AutoRefresh;
use crate::ui::components::skeleton::Skeleton;
use crate::ui::components::stat::{DeltaDir, DeltaInfo, StatTile};
use crate::ui::components::table::{BreakdownRow, BreakdownTable, EntryExitRow, EntryExitTable};
use crate::ui::components::tabs::{RangeTabs, SiteTab, SiteTabs, TabbedCard};
use crate::ui::pages::{
    active_filters, site_api_url, site_csv_url, BTN_GHOST, BTN_PRIMARY, CTRL_INPUT,
};
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
use crate::ui::series::{fill_time_buckets, range_from_query};
use crate::ui::timefmt::{
    browser_timezone, format_ts, label_style_for_buckets, timezone_short_label,
};
use crate::ui::types::{EntryPages, ExitPages, PageviewsResult, SitesList, TopList};

/// Percent change of `cur` vs `prev`, classified for the delta badge.
fn compute_delta(cur: u64, prev: u64) -> Option<DeltaInfo> {
    if prev == 0 {
        return if cur > 0 {
            Some(DeltaInfo {
                dir: DeltaDir::New,
                pct: 0.0,
            })
        } else {
            None
        };
    }
    let pct = (cur as f64 - prev as f64) / prev as f64 * 100.0;
    let dir = if pct > 0.0 {
        DeltaDir::Up
    } else if pct < 0.0 {
        DeltaDir::Down
    } else {
        DeltaDir::Flat
    };
    Some(DeltaInfo {
        dir,
        pct: pct.abs(),
    })
}

/// One top-N breakdown for the active dimension tab. Lazy: only mounts when
/// its parent tab is selected, so inactive dimensions stay unfetched.
/// Set `framed` to wrap in a standalone card (e.g. Sources with no sub-tabs).
#[component]
fn BreakdownPanel(
    site_id: String,
    endpoint: String,
    value_header: String,
    count_header: String,
    use_pageviews: bool,
    field: String,
    empty_title: String,
    /// Card title when `framed` is true; defaults to `empty_title`.
    title: Option<String>,
    csv_href: Option<String>,
    #[props(default = false)] framed: bool,
    /// Auto-refresh tick from the overview toolbar.
    #[props(default)] refresh_tick: u32,
) -> Element {
    let route = use_route::<Route>();
    let q = route.query().cloned().unwrap_or_default();
    let qs = q.to_string();
    let card_title = title.unwrap_or_else(|| empty_title.clone());

    // Range/filters live on the route as plain props; subscribe explicitly so
    // the resource restarts when the user flips 7d/30d/90d/12m (or filters).
    let res = use_resource(use_reactive!(|site_id, endpoint, qs, refresh_tick| async move {
        let _ = refresh_tick;
        let path = site_api_url(&site_id, &endpoint, &qs);
        get_json::<TopList>(&path).await
    }));

    rsx! {
        {match &*res.read() {
            None => rsx! {
                if framed {
                    Card { title: card_title.clone(), Skeleton { lines: 4 } }
                } else {
                    Skeleton { lines: 4 }
                }
            },
            Some(Err(e)) => rsx! {
                if framed {
                    Card { title: card_title.clone(),
                        EmptyState { message: format!("Failed to load ({e})") }
                    }
                } else {
                    EmptyState { message: format!("Failed to load ({e})") }
                }
            },
            Some(Ok(list)) => {
                let rows = list
                    .rows
                    .iter()
                    .map(|r| BreakdownRow {
                        value: r.value.clone(),
                        count: if use_pageviews { r.pageviews } else { r.sessions },
                        pct: r.pct,
                        spark: None,
                    })
                    .collect::<Vec<_>>();
                let route = route.clone();
                let q = q.clone();
                let field = field.clone();
                rsx! {
                    BreakdownTable {
                        title: card_title.clone(),
                        value_header: value_header.clone(),
                        count_header: count_header.clone(),
                        rows,
                        csv_href: csv_href.clone(),
                        framed,
                        empty_title: empty_title.clone(),
                        empty_message: "Nothing matched this range. Try a wider window or clear filters.".to_string(),
                        on_filter: Some(EventHandler::new(move |value: String| {
                            let mut nq = q.clone();
                            nq.filters.push(format!("{field}:eq:{value}"));
                            navigator().push(route.with_query(nq));
                        })),
                    }
                }
            }
        }}
    }
}

#[component]
fn EntryExitPanel(
    site_id: String,
    is_entry: bool,
    #[props(default)] refresh_tick: u32,
) -> Element {
    let q = use_route::<Route>().query().cloned().unwrap_or_default();
    let qs = q.to_string();
    let endpoint = if is_entry {
        "top-entry-pages"
    } else {
        "top-exit-pages"
    };
    let title = if is_entry {
        "Entry pages"
    } else {
        "Exit pages"
    };
    let rate_header = if is_entry { "Bounce" } else { "Exit rate" };

    let endpoint = endpoint.to_string();
    let res = use_resource(use_reactive!(
        |site_id, endpoint, qs, is_entry, refresh_tick| async move {
            let _ = refresh_tick;
            let path = site_api_url(&site_id, &endpoint, &qs);
            if is_entry {
                get_json::<EntryPages>(&path).await.map(|p| {
                    p.rows
                        .into_iter()
                        .map(|r| EntryExitRow {
                            value: r.url,
                            count: r.sessions,
                            pct: r.pct,
                            rate: r.bounce_rate,
                        })
                        .collect::<Vec<_>>()
                })
            } else {
                get_json::<ExitPages>(&path).await.map(|p| {
                    p.rows
                        .into_iter()
                        .map(|r| EntryExitRow {
                            value: r.url,
                            count: r.exits,
                            pct: r.pct,
                            rate: r.exit_rate,
                        })
                        .collect::<Vec<_>>()
                })
            }
        }
    ));

    rsx! {
        {match &*res.read() {
            None => rsx! { Skeleton { lines: 4 } },
            Some(Err(e)) => rsx! {
                EmptyState { message: format!("Failed to load ({e})") }
            },
            Some(Ok(rows)) => rsx! {
                EntryExitTable {
                    title: title.to_string(),
                    rate_header: rate_header.to_string(),
                    rows: rows.clone(),
                    csv_href: None,
                    framed: false,
                    empty_title: format!("No {title} yet"),
                    empty_message: "Nothing matched this range. Try a wider window or clear filters.".to_string(),
                }
            },
        }}
    }
}

/// The site overview dashboard: hero stats + chart, progressive tabbed
/// breakdowns, collapsible filters, and demoted export.
#[component]
pub fn SiteOverview(site_id: String, q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let range = q.range.clone().unwrap_or_else(|| "30d".to_string());
    let qs = q.to_string();
    let refresh_tick = use_signal(|| 0u32);
    let tick = refresh_tick();

    let site = use_resource(use_reactive!(|site_id, tick| async move {
        let _ = tick;
        get_json::<SitesList>("/api/v1/sites")
            .await
            .ok()
            .and_then(|l| l.sites.into_iter().find(|s| s.id == site_id))
    }));
    let (site_name, site_domain, public_key) = {
        let guard = site.read();
        match guard.as_ref().and_then(|o| o.as_ref()) {
            Some(s) => (
                Some(s.name.clone()),
                Some(s.domain.clone()),
                Some(s.public_key.clone()),
            ),
            None => (None, None, None),
        }
    };
    let head_title = site_name.clone().unwrap_or_else(|| "Overview".to_string());
    // Chart labels use the browser's local zone (no site-level timezone UI).
    let tz = browser_timezone();
    let tz_label = timezone_short_label(&tz);

    let pv = use_resource(use_reactive!(|site_id, qs, tick| async move {
        let _ = tick;
        let path = site_api_url(&site_id, "pageviews", &qs);
        get_json::<PageviewsResult>(&path).await
    }));

    let mut f_field = use_signal(|| "url".to_string());
    let mut f_op = use_signal(|| "eq".to_string());
    let mut f_value = use_signal(String::new);
    let mut show_filter = use_signal(|| false);
    let mut show_export = use_signal(|| false);

    // Dimension tab indices for the four tabbed cards.
    let mut pages_tab = use_signal(|| 0usize);
    let mut locations_tab = use_signal(|| 0usize);
    let mut tech_tab = use_signal(|| 0usize);

    let compare_route = {
        let mut nq = q.clone();
        nq.compare = !nq.compare;
        route.with_query(nq)
    };
    let comparing = q.compare;

    let events_csv = site_csv_url(&site_id, "export/events", &qs);
    let sessions_csv = site_csv_url(&site_id, "export/sessions", &qs);
    let pageviews_csv = site_csv_url(&site_id, "pageviews", &qs);

    // CSV for the active dimension inside each tabbed card.
    let pages_csv = match pages_tab() {
        1 => Some(site_csv_url(&site_id, "top-entry-pages", &qs)),
        2 => Some(site_csv_url(&site_id, "top-exit-pages", &qs)),
        _ => Some(site_csv_url(&site_id, "top-pages", &qs)),
    };
    let sources_csv = site_csv_url(&site_id, "top-referrers", &qs);
    let locations_csv = match locations_tab() {
        1 => Some(site_csv_url(&site_id, "top-regions", &qs)),
        _ => Some(site_csv_url(&site_id, "top-countries", &qs)),
    };
    let tech_csv = match tech_tab() {
        1 => Some(site_csv_url(&site_id, "top-browsers", &qs)),
        2 => Some(site_csv_url(&site_id, "top-os", &qs)),
        _ => Some(site_csv_url(&site_id, "top-devices", &qs)),
    };

    let can_install = public_key.as_deref().is_some_and(|k| !k.is_empty());
    let zero_data = matches!(&*pv.read(), Some(Ok(d)) if d.total_pageviews == 0);

    rsx! {
        PageHead { title: head_title, subtitle: site_domain.clone(),
            div { class: "flex items-center gap-2 flex-wrap",
                RangeTabs { active: range.clone() }
                Link {
                    to: compare_route,
                    class: if comparing {
                        "px-3 py-1.5 rounded-lg text-xs font-semibold bg-teal-soft text-teal-hi border border-border-2 no-underline"
                    } else {
                        "px-3 py-1.5 rounded-lg text-xs font-semibold text-muted-1 hover:text-text-1 border border-border-2 no-underline"
                    },
                    if comparing { "Comparing" } else { "Compare" }
                }
                AutoRefresh { tick: refresh_tick }
            }
        }
        SiteTabs { site_id: site_id.clone(), range: range.clone(), active: SiteTab::Overview }

        {active_filters(&route, &q)}

        // Compact filter chrome: Add filter reveals the form.
        div { class: "flex flex-wrap items-center gap-2 mb-6",
            button {
                r#type: "button",
                class: BTN_GHOST,
                onclick: move |_| show_filter.set(!show_filter()),
                if show_filter() { "Hide filter" } else { "Add filter" }
            }
            button {
                r#type: "button",
                class: BTN_GHOST,
                onclick: move |_| show_export.set(!show_export()),
                if show_export() { "Hide export" } else { "Export" }
            }
        }

        if show_filter() {
            form {
                class: "flex flex-wrap items-center gap-2 mb-6 p-3 rounded-lg border border-border-1 bg-surface-1",
                onsubmit: {
                    let route = route.clone();
                    let q = q.clone();
                    move |evt: FormEvent| {
                        evt.prevent_default();
                        let value = f_value().trim().to_string();
                        if !value.is_empty() {
                            let mut nq = q.clone();
                            nq.filters.push(format!("{}:{}:{}", f_field(), f_op(), value));
                            show_filter.set(false);
                            f_value.set(String::new());
                            navigator().push(route.with_query(nq));
                        }
                    }
                },
                select {
                    class: CTRL_INPUT,
                    value: "{f_field}",
                    onchange: move |e| f_field.set(e.value()),
                    option { value: "url", "Page" }
                    option { value: "referrer", "Referrer" }
                    option { value: "country", "Country" }
                    option { value: "region", "Region" }
                    option { value: "browser", "Browser" }
                    option { value: "os", "OS" }
                    option { value: "device_type", "Device" }
                    option { value: "utm_source", "UTM source" }
                    option { value: "utm_medium", "UTM medium" }
                    option { value: "utm_campaign", "UTM campaign" }
                }
                select {
                    class: CTRL_INPUT,
                    value: "{f_op}",
                    onchange: move |e| f_op.set(e.value()),
                    option { value: "eq", "is" }
                    option { value: "not_eq", "is not" }
                    option { value: "contains", "contains" }
                    option { value: "starts_with", "starts with" }
                }
                input {
                    class: CTRL_INPUT,
                    r#type: "text",
                    value: "{f_value}",
                    placeholder: "filter value",
                    oninput: move |e| f_value.set(e.value()),
                }
                button { r#type: "submit", class: BTN_PRIMARY, "Apply" }
            }
        }

        if show_export() {
            div { class: "flex flex-wrap gap-2 mb-6 p-3 rounded-lg border border-border-1 bg-surface-1",
                a { class: BTN_GHOST, href: "{events_csv}", "Events CSV" }
                a { class: BTN_GHOST, href: "{sessions_csv}", "Sessions CSV" }
                a { class: BTN_GHOST, href: "{pageviews_csv}", "Pageviews CSV" }
            }
        }

        if can_install && zero_data {
            div { class: "mb-4",
                InstallCard {
                    public_key: public_key.clone().unwrap_or_default(),
                    domain: site_domain.clone().unwrap_or_default(),
                    prominent: true,
                }
            }
        }

        {match &*pv.read() {
            None => rsx! { Skeleton { lines: 3 } },
            Some(Err(e)) => rsx! {
                Card { EmptyState { message: format!("Failed to load traffic ({e})") } }
            },
            Some(Ok(d)) => {
                let (pv_delta, sess_delta, pv_prev, sess_prev) = if let Some(prev) = d.comparison.as_ref() {
                    (
                        compute_delta(d.total_pageviews, prev.total_pageviews),
                        compute_delta(d.total_sessions, prev.total_sessions),
                        Some(format!("{}", prev.total_pageviews)),
                        Some(format!("{}", prev.total_sessions)),
                    )
                } else {
                    (None, None, None, None)
                };
                // Fill missing buckets so empty days show as zero (chart gaps).
                let time_range = range_from_query(&q);
                let dense = fill_time_buckets(&d.buckets, &time_range);
                let style = label_style_for_buckets(
                    &dense.iter().map(|b| b.ts).collect::<Vec<_>>(),
                );
                let points = dense
                    .iter()
                    .map(|b| ChartPoint {
                        label: format_ts(b.ts, &tz, style),
                        pageviews: b.pageviews,
                        sessions: b.sessions,
                    })
                    .collect::<Vec<_>>();
                // Prior period: densify on its own window, then align by index.
                let previous = d.comparison.as_ref().map(|cmp| {
                    let prev_range = time_range.previous();
                    let prev_dense = fill_time_buckets(&cmp.buckets, &prev_range);
                    let prev_style = label_style_for_buckets(
                        &prev_dense.iter().map(|b| b.ts).collect::<Vec<_>>(),
                    );
                    prev_dense
                        .iter()
                        .map(|b| ChartPoint {
                            label: format_ts(b.ts, &tz, prev_style),
                            pageviews: b.pageviews,
                            sessions: b.sessions,
                        })
                        .collect::<Vec<_>>()
                });
                let tz_hint = tz_label.clone();
                rsx! {
                    Card {
                        div { class: "grid grid-cols-2 md:grid-cols-3 gap-4 mb-4",
                            StatTile {
                                label: "Pageviews",
                                value: format!("{}", d.total_pageviews),
                                delta: pv_delta,
                                prev: pv_prev,
                            }
                            StatTile {
                                label: "Sessions",
                                value: format!("{}", d.total_sessions),
                                delta: sess_delta,
                                prev: sess_prev,
                            }
                            StatTile { label: "Bounce Rate", value: format!("{:.1}%", d.bounce_rate) }
                        }
                        TimeseriesChart { points, previous }
                        div { class: "mt-1.5 text-right text-muted-2 text-[10.5px]",
                            "Times in {tz_hint}"
                        }
                    }
                }
            }
        }}

        // Primary: pages + sources
        div { class: "grid grid-cols-1 md:grid-cols-2 gap-4 mt-4",
            TabbedCard {
                title: "Pages",
                tabs: vec!["Top pages".into(), "Entry".into(), "Exit".into()],
                active: pages_tab(),
                on_select: move |i| pages_tab.set(i),
                csv_href: pages_csv,
                {match pages_tab() {
                    1 => rsx! {
                        EntryExitPanel { site_id: site_id.clone(), is_entry: true, refresh_tick: tick }
                    },
                    2 => rsx! {
                        EntryExitPanel { site_id: site_id.clone(), is_entry: false, refresh_tick: tick }
                    },
                    _ => rsx! {
                        BreakdownPanel {
                            site_id: site_id.clone(),
                            endpoint: "top-pages",
                            value_header: "Page",
                            count_header: "Views",
                            use_pageviews: true,
                            field: "url",
                            empty_title: "No pages yet",
                            refresh_tick: tick,
                        }
                    },
                }}
            }
            BreakdownPanel {
                site_id: site_id.clone(),
                endpoint: "top-referrers",
                value_header: "Source",
                count_header: "Visitors",
                use_pageviews: false,
                field: "referrer",
                empty_title: "No referrers yet",
                title: "Sources".to_string(),
                csv_href: sources_csv.clone(),
                framed: true,
                refresh_tick: tick,
            }
        }

        // Secondary: locations + technology
        div { class: "grid grid-cols-1 md:grid-cols-2 gap-4 mt-4",
            TabbedCard {
                title: "Locations",
                tabs: vec!["Countries".into(), "Regions".into()],
                active: locations_tab(),
                on_select: move |i| locations_tab.set(i),
                csv_href: locations_csv,
                {match locations_tab() {
                    1 => rsx! {
                        BreakdownPanel {
                            site_id: site_id.clone(),
                            endpoint: "top-regions",
                            value_header: "Region",
                            count_header: "Visitors",
                            use_pageviews: false,
                            field: "region",
                            empty_title: "No regions yet",
                            refresh_tick: tick,
                        }
                    },
                    _ => rsx! {
                        BreakdownPanel {
                            site_id: site_id.clone(),
                            endpoint: "top-countries",
                            value_header: "Country",
                            count_header: "Visitors",
                            use_pageviews: false,
                            field: "country",
                            empty_title: "No countries yet",
                            refresh_tick: tick,
                        }
                    },
                }}
            }
            TabbedCard {
                title: "Technology",
                tabs: vec!["Devices".into(), "Browsers".into(), "OS".into()],
                active: tech_tab(),
                on_select: move |i| tech_tab.set(i),
                csv_href: tech_csv,
                {match tech_tab() {
                    1 => rsx! {
                        BreakdownPanel {
                            site_id: site_id.clone(),
                            endpoint: "top-browsers",
                            value_header: "Browser",
                            count_header: "Visitors",
                            use_pageviews: false,
                            field: "browser",
                            empty_title: "No browsers yet",
                            refresh_tick: tick,
                        }
                    },
                    2 => rsx! {
                        BreakdownPanel {
                            site_id: site_id.clone(),
                            endpoint: "top-os",
                            value_header: "OS",
                            count_header: "Visitors",
                            use_pageviews: false,
                            field: "os",
                            empty_title: "No operating systems yet",
                            refresh_tick: tick,
                        }
                    },
                    _ => rsx! {
                        BreakdownPanel {
                            site_id: site_id.clone(),
                            endpoint: "top-devices",
                            value_header: "Device",
                            count_header: "Visitors",
                            use_pageviews: false,
                            field: "device_type",
                            empty_title: "No devices yet",
                            refresh_tick: tick,
                        }
                    },
                }}
            }
        }

        if can_install && !zero_data {
            div { class: "mt-4",
                InstallCard {
                    public_key: public_key.clone().unwrap_or_default(),
                    domain: site_domain.clone().unwrap_or_default(),
                    prominent: false,
                }
            }
        }
    }
}
