use clap::Subcommand;
use serde_json::Value;

use crate::client::ApiClient;
use crate::req::Req;

#[derive(Subcommand)]
pub enum QueryCommand {
    /// Get pageview timeseries
    Pageviews {
        #[arg(long)]
        site: String,
        #[arg(long, default_value = "30d")]
        range: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = "day")]
        granularity: String,
        /// Compare to the immediately preceding, equal-length period.
        #[arg(long)]
        compare: bool,
        /// Filter as `field:op:value` (repeatable). E.g. `country:eq:US`.
        #[arg(long = "filter")]
        filters: Vec<String>,
    },
    /// Top pages by traffic
    TopPages {
        #[arg(long)]
        site: String,
        #[arg(long, default_value = "30d")]
        range: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = "20")]
        limit: u32,
        /// Compare to the immediately preceding, equal-length period.
        #[arg(long)]
        compare: bool,
        #[arg(long = "filter")]
        filters: Vec<String>,
    },
    /// Top referrers
    TopReferrers {
        #[arg(long)]
        site: String,
        #[arg(long, default_value = "30d")]
        range: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = "20")]
        limit: u32,
        #[arg(long)]
        compare: bool,
        #[arg(long = "filter")]
        filters: Vec<String>,
    },
    /// Top operating systems
    TopOs {
        #[arg(long)]
        site: String,
        #[arg(long, default_value = "30d")]
        range: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = "20")]
        limit: u32,
        #[arg(long)]
        compare: bool,
        #[arg(long = "filter")]
        filters: Vec<String>,
    },
    /// Top regions
    TopRegions {
        #[arg(long)]
        site: String,
        #[arg(long, default_value = "30d")]
        range: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = "20")]
        limit: u32,
        #[arg(long)]
        compare: bool,
        #[arg(long = "filter")]
        filters: Vec<String>,
    },
    /// Top entry (landing) pages
    TopEntryPages {
        #[arg(long)]
        site: String,
        #[arg(long, default_value = "30d")]
        range: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = "20")]
        limit: u32,
        #[arg(long)]
        compare: bool,
        #[arg(long = "filter")]
        filters: Vec<String>,
    },
    /// Top exit pages
    TopExitPages {
        #[arg(long)]
        site: String,
        #[arg(long, default_value = "30d")]
        range: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = "20")]
        limit: u32,
        #[arg(long)]
        compare: bool,
        #[arg(long = "filter")]
        filters: Vec<String>,
    },
    /// UTM campaign breakdowns (source/medium/campaign/term/content)
    Campaigns {
        #[arg(long)]
        site: String,
        #[arg(long, default_value = "30d")]
        range: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = "20")]
        limit: u32,
        #[arg(long = "filter")]
        filters: Vec<String>,
    },
    /// Single-dimension UTM breakdown (source/medium/campaign/term/content)
    Utm {
        #[arg(long)]
        site: String,
        /// One of: source, medium, campaign, term, content.
        #[arg(long)]
        dimension: String,
        #[arg(long, default_value = "30d")]
        range: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = "20")]
        limit: u32,
        /// Restrict to a single utm_source.
        #[arg(long = "utm-source")]
        utm_source: Option<String>,
        /// Restrict to a single utm_medium.
        #[arg(long = "utm-medium")]
        utm_medium: Option<String>,
        #[arg(long = "filter")]
        filters: Vec<String>,
    },
    /// Custom events breakdown
    Events {
        #[arg(long)]
        site: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "30d")]
        range: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = "20")]
        limit: u32,
        #[arg(long = "filter")]
        filters: Vec<String>,
    },
    /// Funnel conversion analysis
    Funnel {
        #[arg(long)]
        site: String,
        #[arg(long)]
        funnel: String,
        #[arg(long, default_value = "30d")]
        range: String,
    },
    /// List all funnels for a site
    Funnels {
        #[arg(long)]
        site: String,
    },
    /// Create a new funnel from a JSON step definition
    FunnelCreate {
        #[arg(long)]
        site: String,
        /// Display name for the funnel.
        #[arg(long)]
        name: String,
        /// JSON array of steps, e.g.
        /// '[{"name":"View","event_name":"pageview","filters":[]},{"name":"Signup","event_name":"signup","filters":[]}]'
        #[arg(long)]
        steps: String,
    },
}

/// Range portion of a query string: a `from`+`to` pair wins over the preset.
fn range_qs(range: &str, from: &Option<String>, to: &Option<String>) -> String {
    match (from, to) {
        (Some(f), Some(t)) => format!("from={}&to={}", enc(f), enc(t)),
        _ => format!("range={range}"),
    }
}

/// `&filter=field:op:value` fragment for each filter, URL-encoded.
fn filters_qs(filters: &[String]) -> String {
    filters
        .iter()
        .map(|f| format!("&filter={}", enc(f)))
        .collect()
}

/// `&compare=1` when comparison is requested, else empty.
fn compare_qs(compare: bool) -> &'static str {
    if compare {
        "&compare=1"
    } else {
        ""
    }
}

/// `&key=value` when `value` is present, URL-encoded; empty otherwise.
fn opt_qs(key: &str, value: &Option<String>) -> String {
    value
        .as_deref()
        .map(|v| format!("&{key}={}", enc(v)))
        .unwrap_or_default()
}

/// Percent-encode a query-string component (RFC 3986 unreserved set passes
/// through).
pub(crate) fn enc(s: &str) -> String {
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

/// Build the HTTP request for a query command without performing any I/O. Kept
/// pure so request construction can be unit-tested.
pub fn build(cmd: &QueryCommand) -> anyhow::Result<Req> {
    let req = match cmd {
        QueryCommand::Pageviews {
            site,
            range,
            from,
            to,
            granularity,
            compare,
            filters,
        } => {
            let rq = range_qs(range, from, to);
            let fq = filters_qs(filters);
            let cq = compare_qs(*compare);
            Req::Get(format!(
                "/api/v1/sites/{site}/pageviews?{rq}&granularity={granularity}{fq}{cq}"
            ))
        }
        QueryCommand::TopPages {
            site,
            range,
            from,
            to,
            limit,
            compare,
            filters,
        } => top_req(
            site,
            "top-pages",
            range,
            from,
            to,
            *limit,
            *compare,
            filters,
        ),
        QueryCommand::TopReferrers {
            site,
            range,
            from,
            to,
            limit,
            compare,
            filters,
        } => top_req(
            site,
            "top-referrers",
            range,
            from,
            to,
            *limit,
            *compare,
            filters,
        ),
        QueryCommand::TopOs {
            site,
            range,
            from,
            to,
            limit,
            compare,
            filters,
        } => top_req(site, "top-os", range, from, to, *limit, *compare, filters),
        QueryCommand::TopRegions {
            site,
            range,
            from,
            to,
            limit,
            compare,
            filters,
        } => top_req(
            site,
            "top-regions",
            range,
            from,
            to,
            *limit,
            *compare,
            filters,
        ),
        QueryCommand::TopEntryPages {
            site,
            range,
            from,
            to,
            limit,
            compare,
            filters,
        } => top_req(
            site,
            "top-entry-pages",
            range,
            from,
            to,
            *limit,
            *compare,
            filters,
        ),
        QueryCommand::TopExitPages {
            site,
            range,
            from,
            to,
            limit,
            compare,
            filters,
        } => top_req(
            site,
            "top-exit-pages",
            range,
            from,
            to,
            *limit,
            *compare,
            filters,
        ),
        QueryCommand::Campaigns {
            site,
            range,
            from,
            to,
            limit,
            filters,
        } => {
            let rq = range_qs(range, from, to);
            let fq = filters_qs(filters);
            Req::Get(format!(
                "/api/v1/sites/{site}/campaigns?{rq}&limit={limit}{fq}"
            ))
        }
        QueryCommand::Utm {
            site,
            dimension,
            range,
            from,
            to,
            limit,
            utm_source,
            utm_medium,
            filters,
        } => {
            let rq = range_qs(range, from, to);
            let fq = filters_qs(filters);
            let src = opt_qs("utm_source", utm_source);
            let med = opt_qs("utm_medium", utm_medium);
            Req::Get(format!(
                "/api/v1/sites/{site}/utm?{rq}&dimension={}&limit={limit}{src}{med}{fq}",
                enc(dimension)
            ))
        }
        QueryCommand::Events {
            site,
            name,
            range,
            from,
            to,
            limit,
            filters,
        } => {
            let rq = range_qs(range, from, to);
            let fq = filters_qs(filters);
            let name_param = opt_qs("name", name);
            Req::Get(format!(
                "/api/v1/sites/{site}/events?{rq}&limit={limit}{name_param}{fq}"
            ))
        }
        QueryCommand::Funnel {
            site,
            funnel,
            range,
        } => Req::Get(format!(
            "/api/v1/sites/{site}/funnels/{funnel}?range={range}"
        )),
        QueryCommand::Funnels { site } => Req::Get(format!("/api/v1/sites/{site}/funnels")),
        QueryCommand::FunnelCreate { site, name, steps } => {
            let steps_val: Value = serde_json::from_str(steps)
                .map_err(|e| anyhow::anyhow!("--steps is not valid JSON: {e}"))?;
            if !steps_val.is_array() {
                anyhow::bail!("--steps must be a JSON array of step objects");
            }
            let body = serde_json::json!({ "name": name, "steps": steps_val });
            Req::Post(format!("/api/v1/sites/{site}/funnels"), body)
        }
    };
    Ok(req)
}

pub async fn run(cmd: &QueryCommand, client: &ApiClient, human: bool) -> anyhow::Result<()> {
    let result: Value = build(cmd)?.send(client).await?.unwrap_or(Value::Null);

    if human {
        print_human(&result);
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }

    Ok(())
}

/// Shared request builder for the `top-*` dimension endpoints.
#[allow(clippy::too_many_arguments)]
fn top_req(
    site: &str,
    endpoint: &str,
    range: &str,
    from: &Option<String>,
    to: &Option<String>,
    limit: u32,
    compare: bool,
    filters: &[String],
) -> Req {
    let rq = range_qs(range, from, to);
    let fq = filters_qs(filters);
    let cq = compare_qs(compare);
    Req::Get(format!(
        "/api/v1/sites/{site}/{endpoint}?{rq}&limit={limit}{fq}{cq}"
    ))
}

fn print_human(value: &Value) {
    // Path report: rows carry a `steps` array rather than a single value.
    if let Some(rows) = value.get("rows").and_then(|r| r.as_array()) {
        if rows.first().is_some_and(|r| r.get("steps").is_some()) {
            println!("{:<10} {:>6}  Path", "Sessions", "%");
            println!("{}", "-".repeat(80));
            for row in rows {
                let steps: Vec<&str> = row["steps"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|s| s.as_str()).collect())
                    .unwrap_or_default();
                println!(
                    "{:<10} {:>5.1}%  {}",
                    row["sessions"].as_u64().unwrap_or(0),
                    row["pct"].as_f64().unwrap_or(0.0),
                    steps.join(" -> "),
                );
            }
            return;
        }
    }
    // Simple human table renderer for top-list results
    if let Some(rows) = value.get("rows").and_then(|r| r.as_array()) {
        println!(
            "{:<50} {:>10} {:>10} {:>6}",
            "Value", "Pageviews", "Sessions", "%"
        );
        println!("{}", "-".repeat(80));
        for row in rows {
            println!(
                "{:<50} {:>10} {:>10} {:>5.1}%",
                row["value"].as_str().unwrap_or("-"),
                row["pageviews"].as_u64().unwrap_or(0),
                row["sessions"].as_u64().unwrap_or(0),
                row["pct"].as_f64().unwrap_or(0.0),
            );
        }
    } else if let Some(steps) = value.get("steps").and_then(|s| s.as_array()) {
        println!("{:<30} {:>10} {:>12}", "Step", "Sessions", "Conversion");
        println!("{}", "-".repeat(55));
        for step in steps {
            println!(
                "{:<30} {:>10} {:>11.1}%",
                step["name"].as_str().unwrap_or("-"),
                step["sessions"].as_u64().unwrap_or(0),
                step["conversion_rate"].as_f64().unwrap_or(0.0) * 100.0,
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_default()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Tiny harness so we can parse a `query <args...>` line through clap.
    #[derive(Parser)]
    struct Harness {
        #[command(subcommand)]
        cmd: QueryCommand,
    }

    fn parse(args: &[&str]) -> QueryCommand {
        let mut full = vec!["spq"];
        full.extend_from_slice(args);
        Harness::try_parse_from(full).expect("parse").cmd
    }

    fn get_path(args: &[&str]) -> String {
        match build(&parse(args)).expect("build") {
            Req::Get(p) => p,
            other => panic!("expected GET, got {other:?}"),
        }
    }

    #[test]
    fn pageviews_compare_and_filters() {
        let p = get_path(&[
            "pageviews",
            "--site",
            "abc",
            "--range",
            "30d",
            "--compare",
            "--filter",
            "country:eq:US",
            "--filter",
            "device_type:eq:mobile",
        ]);
        assert_eq!(
            p,
            "/api/v1/sites/abc/pageviews?range=30d&granularity=day&filter=country%3Aeq%3AUS&filter=device_type%3Aeq%3Amobile&compare=1"
        );
    }

    #[test]
    fn from_to_overrides_range() {
        let p = get_path(&[
            "pageviews",
            "--site",
            "abc",
            "--from",
            "2025-01-01",
            "--to",
            "2025-01-31",
        ]);
        assert!(p.contains("from=2025-01-01&to=2025-01-31"));
        assert!(!p.contains("range="));
    }

    #[test]
    fn top_pages_limit_and_compare() {
        let p = get_path(&["top-pages", "--site", "s", "--limit", "5", "--compare"]);
        assert_eq!(p, "/api/v1/sites/s/top-pages?range=30d&limit=5&compare=1");
    }

    #[test]
    fn entry_and_exit_pages() {
        assert!(get_path(&["top-entry-pages", "--site", "s"])
            .starts_with("/api/v1/sites/s/top-entry-pages?"));
        assert!(get_path(&["top-exit-pages", "--site", "s"])
            .starts_with("/api/v1/sites/s/top-exit-pages?"));
    }

    #[test]
    fn utm_dimension_and_filters() {
        let p = get_path(&[
            "utm",
            "--site",
            "s",
            "--dimension",
            "source",
            "--utm-medium",
            "cpc",
        ]);
        assert_eq!(
            p,
            "/api/v1/sites/s/utm?range=30d&dimension=source&limit=20&utm_medium=cpc"
        );
    }

    #[test]
    fn events_filters() {
        let p = get_path(&[
            "events",
            "--site",
            "s",
            "--name",
            "signup",
            "--filter",
            "plan:eq:pro",
        ]);
        assert_eq!(
            p,
            "/api/v1/sites/s/events?range=30d&limit=20&name=signup&filter=plan%3Aeq%3Apro"
        );
    }

    #[test]
    fn funnel_create_builds_post() {
        let cmd = parse(&[
            "funnel-create",
            "--site",
            "s",
            "--name",
            "Signup",
            "--steps",
            r#"[{"name":"View","event_name":"pageview","filters":[]},{"name":"Signup","event_name":"signup","filters":[]}]"#,
        ]);
        match build(&cmd).expect("build") {
            Req::Post(path, body) => {
                assert_eq!(path, "/api/v1/sites/s/funnels");
                assert_eq!(body["name"], "Signup");
                assert!(body["steps"].is_array());
            }
            other => panic!("expected POST, got {other:?}"),
        }
    }
}
