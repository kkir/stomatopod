use dioxus::prelude::*;

use crate::ui::api::get_json;
use crate::ui::components::card::{Card, EmptyState};
use crate::ui::components::chart::{ChartPoint, TimeseriesChart};
use crate::ui::components::install::InstallCard;
use crate::ui::components::layout::PageHead;
use crate::ui::components::stat::{DeltaDir, DeltaInfo, StatTile};
use crate::ui::components::table::{BreakdownRow, BreakdownTable, EntryExitRow, EntryExitTable};
use crate::ui::components::tabs::{RangeTabs, SiteTab, SiteTabs};
use crate::ui::pages::{
    active_filters, site_api_url, site_csv_url, BTN_GHOST, BTN_PRIMARY, CTRL_INPUT,
};
use crate::ui::query::DashQuery;
use crate::ui::routes::Route;
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

/// One top-N breakdown card. Self-contained: fetches its own `TopList`
/// and, on a row click, pushes the current route with a new
/// `field:eq:value` filter appended (mirrors the toptable macro's
/// clickable rows in the legacy site.jinja).
#[component]
fn BreakdownCard(
    site_id: String,
    endpoint: String,
    title: String,
    value_header: String,
    count_header: String,
    use_pageviews: bool,
    field: String,
) -> Element {
    let route = use_route::<Route>();
    let q = route.query().cloned().unwrap_or_default();
    let qs = q.to_string();

    let res = use_resource({
        let site_id = site_id.clone();
        let endpoint = endpoint.clone();
        let qs = qs.clone();
        move || {
            let path = site_api_url(&site_id, &endpoint, &qs);
            async move { get_json::<TopList>(&path).await }
        }
    });
    let csv = site_csv_url(&site_id, &endpoint, &qs);

    rsx! {
        {match &*res.read() {
            None => rsx! { Skeleton { lines: 4 } },
            Some(Err(e)) => rsx! {
                Card { title: title.clone(),
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
                        title: title.clone(),
                        value_header: value_header.clone(),
                        count_header: count_header.clone(),
                        rows,
                        csv_href: Some(csv.clone()),
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

use crate::ui::components::skeleton::Skeleton;

/// The site overview dashboard, port of crates/web/templates/site.jinja:
/// hero stats + traffic chart, seven breakdown tables, entry/exit tables,
/// filter controls, and period comparison.
#[component]
pub fn SiteOverview(site_id: String, q: DashQuery) -> Element {
    let route = use_route::<Route>();
    let range = q.range.clone().unwrap_or_else(|| "30d".to_string());
    let qs = q.to_string();

    // Resolve this site's name/domain/public key from the org list (there is
    // no single-site GET). Drives the page title and the install snippet.
    let site = use_resource({
        let site_id = site_id.clone();
        move || {
            let site_id = site_id.clone();
            async move {
                get_json::<SitesList>("/api/v1/sites")
                    .await
                    .ok()
                    .and_then(|l| l.sites.into_iter().find(|s| s.id == site_id))
            }
        }
    });
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

    let pv = use_resource({
        let site_id = site_id.clone();
        let qs = qs.clone();
        move || {
            let path = site_api_url(&site_id, "pageviews", &qs);
            async move { get_json::<PageviewsResult>(&path).await }
        }
    });
    let entry = use_resource({
        let site_id = site_id.clone();
        let qs = qs.clone();
        move || {
            let path = site_api_url(&site_id, "top-entry-pages", &qs);
            async move { get_json::<EntryPages>(&path).await }
        }
    });
    let exit = use_resource({
        let site_id = site_id.clone();
        let qs = qs.clone();
        move || {
            let path = site_api_url(&site_id, "top-exit-pages", &qs);
            async move { get_json::<ExitPages>(&path).await }
        }
    });

    let mut f_field = use_signal(|| "url".to_string());
    let mut f_op = use_signal(|| "eq".to_string());
    let mut f_value = use_signal(String::new);

    let compare_route = {
        let mut nq = q.clone();
        nq.compare = !nq.compare;
        route.with_query(nq)
    };
    let comparing = q.compare;

    let entry_csv = site_csv_url(&site_id, "top-entry-pages", &qs);
    let exit_csv = site_csv_url(&site_id, "top-exit-pages", &qs);
    let events_csv = site_csv_url(&site_id, "export/events", &qs);
    let sessions_csv = site_csv_url(&site_id, "export/sessions", &qs);
    let pageviews_csv = site_csv_url(&site_id, "pageviews", &qs);

    // The install snippet needs a resolved public key; `zero_data` decides
    // whether it leads the page (onboarding) or trails it (reference).
    let can_install = public_key.as_deref().is_some_and(|k| !k.is_empty());
    let zero_data = matches!(&*pv.read(), Some(Ok(d)) if d.total_pageviews == 0);

    rsx! {
        PageHead { title: head_title, subtitle: site_domain.clone(),
            RangeTabs { active: range.clone() }
        }
        SiteTabs { site_id: site_id.clone(), range: range.clone(), active: SiteTab::Overview }

        {active_filters(&route, &q)}

        if can_install && zero_data {
            div { class: "mb-4",
                InstallCard {
                    public_key: public_key.clone().unwrap_or_default(),
                    domain: site_domain.clone().unwrap_or_default(),
                    prominent: true,
                }
            }
        }

        form {
            class: "flex flex-wrap items-center gap-2 mb-6",
            onsubmit: {
                let route = route.clone();
                let q = q.clone();
                move |evt: FormEvent| {
                    evt.prevent_default();
                    let value = f_value().trim().to_string();
                    if !value.is_empty() {
                        let mut nq = q.clone();
                        nq.filters.push(format!("{}:{}:{}", f_field(), f_op(), value));
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
            Link {
                to: compare_route,
                class: BTN_GHOST,
                if comparing { "Comparing prior period" } else { "Compare period" }
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
                let points = d
                    .buckets
                    .iter()
                    .map(|b| ChartPoint {
                        label: b.ts.format("%m/%d").to_string(),
                        value: b.pageviews,
                    })
                    .collect::<Vec<_>>();
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
                        TimeseriesChart { points }
                    }
                }
            }
        }}

        div { class: "grid grid-cols-1 md:grid-cols-2 gap-4 mt-4",
            BreakdownCard {
                site_id: site_id.clone(),
                endpoint: "top-pages",
                title: "Top Pages",
                value_header: "Page",
                count_header: "Views",
                use_pageviews: true,
                field: "url",
            }
            BreakdownCard {
                site_id: site_id.clone(),
                endpoint: "top-referrers",
                title: "Top Referrers",
                value_header: "Source",
                count_header: "Visitors",
                use_pageviews: false,
                field: "referrer",
            }
            BreakdownCard {
                site_id: site_id.clone(),
                endpoint: "top-countries",
                title: "Countries",
                value_header: "Country",
                count_header: "Visitors",
                use_pageviews: false,
                field: "country",
            }
            BreakdownCard {
                site_id: site_id.clone(),
                endpoint: "top-regions",
                title: "Regions",
                value_header: "Region",
                count_header: "Visitors",
                use_pageviews: false,
                field: "region",
            }
            BreakdownCard {
                site_id: site_id.clone(),
                endpoint: "top-browsers",
                title: "Browsers",
                value_header: "Browser",
                count_header: "Visitors",
                use_pageviews: false,
                field: "browser",
            }
            BreakdownCard {
                site_id: site_id.clone(),
                endpoint: "top-os",
                title: "Operating Systems",
                value_header: "OS",
                count_header: "Visitors",
                use_pageviews: false,
                field: "os",
            }
            BreakdownCard {
                site_id: site_id.clone(),
                endpoint: "top-devices",
                title: "Devices",
                value_header: "Device",
                count_header: "Visitors",
                use_pageviews: false,
                field: "device_type",
            }

            {match &*entry.read() {
                None => rsx! { Skeleton { lines: 4 } },
                Some(Err(e)) => rsx! {
                    Card { title: "Entry Pages",
                        EmptyState { message: format!("Failed to load ({e})") }
                    }
                },
                Some(Ok(p)) => {
                    let rows = p
                        .rows
                        .iter()
                        .map(|r| EntryExitRow {
                            value: r.url.clone(),
                            count: r.sessions,
                            pct: r.pct,
                            rate: r.bounce_rate,
                        })
                        .collect::<Vec<_>>();
                    rsx! {
                        EntryExitTable {
                            title: "Entry Pages",
                            rate_header: "Bounce",
                            rows,
                            csv_href: Some(entry_csv.clone()),
                        }
                    }
                }
            }}

            {match &*exit.read() {
                None => rsx! { Skeleton { lines: 4 } },
                Some(Err(e)) => rsx! {
                    Card { title: "Exit Pages",
                        EmptyState { message: format!("Failed to load ({e})") }
                    }
                },
                Some(Ok(p)) => {
                    let rows = p
                        .rows
                        .iter()
                        .map(|r| EntryExitRow {
                            value: r.url.clone(),
                            count: r.exits,
                            pct: r.pct,
                            rate: r.exit_rate,
                        })
                        .collect::<Vec<_>>();
                    rsx! {
                        EntryExitTable {
                            title: "Exit Pages",
                            rate_header: "Exit rate",
                            rows,
                            csv_href: Some(exit_csv.clone()),
                        }
                    }
                }
            }}
        }

        div { class: "mt-4",
            Card { title: "Export",
                div { class: "flex flex-wrap gap-2",
                    a { class: BTN_GHOST, href: "{events_csv}", "Events CSV" }
                    a { class: BTN_GHOST, href: "{sessions_csv}", "Sessions CSV" }
                    a { class: BTN_GHOST, href: "{pageviews_csv}", "Pageviews CSV" }
                }
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
