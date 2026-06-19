use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
};
use std::sync::Arc;

use stomatopod_core::query::pageviews::{Filter, FilterOp, PageviewsQuery};
use stomatopod_query::reports::fetch_dashboard;

use crate::{
    error::AppError,
    extractors::{DashQuery, SiteId},
    state::AppState,
    templates,
};

pub async fn index(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    let orgs = state.meta.list_orgs().await?;
    let sites = match orgs.first() {
        Some(org) => state.meta.list_sites(org.id).await?,
        None => vec![],
    };

    if sites.is_empty() {
        return Ok(templates::render(
            &state,
            "index.jinja",
            minijinja::context! { sites => [] as [i32; 0] },
        )?
        .into_response());
    }

    Ok(Redirect::to(&format!("/app/sites/{}", sites[0].id)).into_response())
}

pub async fn site_overview(
    State(state): State<Arc<AppState>>,
    SiteId(site_id): SiteId,
    dq: DashQuery,
) -> Result<Response, AppError> {
    let query = PageviewsQuery {
        site_id,
        range: dq.range.clone(),
        granularity: dq.granularity,
        filters: dq.filters.clone(),
    };

    let report = fetch_dashboard(&state.backend, &query, 20, dq.compare).await?;

    let site = state
        .meta
        .get_site(site_id)
        .await?
        .ok_or(AppError::NotFound("site not found"))?;

    // Range portion of the query string, reused by every dashboard link so
    // the active window survives navigation.
    let q_range = if dq.custom {
        format!(
            "from={}&to={}",
            enc(dq.from.as_deref().unwrap_or("")),
            enc(dq.to.as_deref().unwrap_or("")),
        )
    } else {
        format!("range={}", dq.label)
    };

    // `&filter=…` for every active filter, URL-encoded; and the compare flag.
    let q_filters: String = dq
        .filters
        .iter()
        .map(|f| format!("&filter={}", enc(&f.to_token())))
        .collect();
    let q_compare = if dq.compare { "&compare=1" } else { "" };
    // Filters + compare, appended after any range base (used by the preset
    // range tabs, which swap only the range portion).
    let q_fc = format!("{q_filters}{q_compare}");
    // The complete current query string (no leading `?`) — the base that
    // row "click to filter" links extend with one more `&filter=`.
    let base = format!("{q_range}{q_fc}");
    // Compare toggle: link that flips the flag, keeping range + filters.
    let compare_href = if dq.compare {
        format!("?{q_range}{q_filters}")
    } else {
        format!("?{q_range}{q_filters}&compare=1")
    };

    // Per-filter view models, each with a removal link that drops just that
    // one filter (keeping range, compare, and the others).
    let filters_view: Vec<serde_json::Value> = dq
        .filters
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let others: String = dq
                .filters
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, g)| format!("&filter={}", enc(&g.to_token())))
                .collect();
            serde_json::json!({
                "token": f.to_token(),
                "label": filter_label(f),
                "remove_href": format!("?{q_range}{others}{q_compare}"),
            })
        })
        .collect();

    // Period-over-period deltas for the hero stat cards.
    let (pv_delta, sess_delta) = match &report.comparison {
        Some(cmp) => (
            delta_view(report.pageviews.total_pageviews, cmp.total_pageviews),
            delta_view(report.pageviews.total_sessions, cmp.total_sessions),
        ),
        None => (serde_json::Value::Null, serde_json::Value::Null),
    };
    let cmp_pageviews = report.comparison.as_ref().map(|c| c.total_pageviews);
    let cmp_sessions = report.comparison.as_ref().map(|c| c.total_sessions);

    let html = templates::render(
        &state,
        "site.jinja",
        minijinja::context! {
            site => serde_json::to_value(&site).unwrap(),
            range => dq.label,
            custom => dq.custom,
            from => dq.from,
            to => dq.to,
            compare => dq.compare,
            q_range => q_range,
            q_fc => q_fc,
            base => base,
            compare_href => compare_href,
            filters => filters_view,
            total_pageviews => report.pageviews.total_pageviews,
            total_sessions => report.pageviews.total_sessions,
            bounce_rate => format!("{:.1}%", report.pageviews.bounce_rate * 100.0),
            pv_delta => pv_delta,
            sess_delta => sess_delta,
            cmp_pageviews => cmp_pageviews,
            cmp_sessions => cmp_sessions,
            top_pages => serde_json::to_value(&report.top_pages.rows).unwrap(),
            top_referrers => serde_json::to_value(&report.top_referrers.rows).unwrap(),
            top_countries => serde_json::to_value(&report.top_countries.rows).unwrap(),
            top_browsers => serde_json::to_value(&report.top_browsers.rows).unwrap(),
            top_devices => serde_json::to_value(&report.top_devices.rows).unwrap(),
            top_os => serde_json::to_value(&report.top_os.rows).unwrap(),
            top_regions => serde_json::to_value(&report.top_regions.rows).unwrap(),
            buckets => serde_json::to_value(&report.pageviews.buckets).unwrap(),
        },
    )?;

    Ok(html.into_response())
}

/// Percent-encode a string for use in a URL query component. Keeps the
/// RFC 3986 unreserved set verbatim and `%XX`-escapes everything else, so
/// filter tokens (`country:eq:US`) and dates survive round-tripping.
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Human-readable pill label, e.g. `country = US` or `url contains /blog`.
fn filter_label(f: &Filter) -> String {
    let op = match f.op {
        FilterOp::Eq => "=",
        FilterOp::NotEq => "≠",
        FilterOp::Contains => "contains",
        FilterOp::StartsWith => "starts with",
    };
    format!("{} {} {}", f.field.token(), op, f.value)
}

/// Build a `{pct, dir}` delta object for a current vs prior value. `dir` is
/// `up`/`down`/`flat` so the template can colour the badge; a zero prior
/// yields a `new` direction to avoid a divide-by-zero.
fn delta_view(current: u64, prior: u64) -> serde_json::Value {
    if prior == 0 {
        let dir = if current > 0 { "new" } else { "flat" };
        return serde_json::json!({ "pct": 0.0, "dir": dir });
    }
    let pct = ((current as f64 - prior as f64) / prior as f64) * 100.0;
    let dir = if pct > 0.5 {
        "up"
    } else if pct < -0.5 {
        "down"
    } else {
        "flat"
    };
    serde_json::json!({ "pct": pct.abs(), "dir": dir })
}
