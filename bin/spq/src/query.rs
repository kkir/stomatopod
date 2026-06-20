use clap::Subcommand;
use serde_json::Value;

use crate::client::ApiClient;

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
    },
    /// Weekly retention cohort grid
    Retention {
        #[arg(long)]
        site: String,
        #[arg(long, default_value = "90d")]
        range: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
    },
    /// Top user paths (page-navigation sequences)
    Paths {
        #[arg(long)]
        site: String,
        #[arg(long, default_value = "30d")]
        range: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        /// Steps per sequence (2–10).
        #[arg(long, default_value = "3")]
        depth: u32,
        #[arg(long, default_value = "25")]
        limit: u32,
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

/// Percent-encode a query-string component (RFC 3986 unreserved set passes
/// through).
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

pub async fn run(cmd: &QueryCommand, client: &ApiClient, human: bool) -> anyhow::Result<()> {
    let result: Value = match cmd {
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
            let cq = if *compare { "&compare=1" } else { "" };
            client
                .get(&format!(
                    "/api/v1/sites/{site}/pageviews?{rq}&granularity={granularity}{fq}{cq}"
                ))
                .await?
        }
        QueryCommand::TopPages {
            site,
            range,
            from,
            to,
            limit,
            filters,
        } => top_query(client, site, "top-pages", range, from, to, *limit, filters).await?,
        QueryCommand::TopReferrers {
            site,
            range,
            from,
            to,
            limit,
            filters,
        } => {
            top_query(
                client,
                site,
                "top-referrers",
                range,
                from,
                to,
                *limit,
                filters,
            )
            .await?
        }
        QueryCommand::TopOs {
            site,
            range,
            from,
            to,
            limit,
            filters,
        } => top_query(client, site, "top-os", range, from, to, *limit, filters).await?,
        QueryCommand::TopRegions {
            site,
            range,
            from,
            to,
            limit,
            filters,
        } => {
            top_query(
                client,
                site,
                "top-regions",
                range,
                from,
                to,
                *limit,
                filters,
            )
            .await?
        }
        QueryCommand::Campaigns {
            site,
            range,
            from,
            to,
            limit,
        } => {
            let rq = range_qs(range, from, to);
            client
                .get(&format!(
                    "/api/v1/sites/{site}/campaigns?{rq}&limit={limit}"
                ))
                .await?
        }
        QueryCommand::Retention {
            site,
            range,
            from,
            to,
        } => {
            let rq = range_qs(range, from, to);
            client
                .get(&format!("/api/v1/sites/{site}/retention?{rq}"))
                .await?
        }
        QueryCommand::Paths {
            site,
            range,
            from,
            to,
            depth,
            limit,
        } => {
            let rq = range_qs(range, from, to);
            client
                .get(&format!(
                    "/api/v1/sites/{site}/paths?{rq}&depth={depth}&limit={limit}"
                ))
                .await?
        }
        QueryCommand::Events {
            site,
            name,
            range,
            from,
            to,
            limit,
        } => {
            let rq = range_qs(range, from, to);
            let name_param = name
                .as_deref()
                .map(|n| format!("&name={}", enc(n)))
                .unwrap_or_default();
            client
                .get(&format!(
                    "/api/v1/sites/{site}/events?{rq}&limit={limit}{name_param}"
                ))
                .await?
        }
        QueryCommand::Funnel {
            site,
            funnel,
            range,
        } => {
            client
                .get(&format!(
                    "/api/v1/sites/{site}/funnels/{funnel}?range={range}"
                ))
                .await?
        }
        QueryCommand::Funnels { site } => {
            client.get(&format!("/api/v1/sites/{site}/funnels")).await?
        }
        QueryCommand::FunnelCreate { site, name, steps } => {
            let steps_val: Value = serde_json::from_str(steps)
                .map_err(|e| anyhow::anyhow!("--steps is not valid JSON: {e}"))?;
            if !steps_val.is_array() {
                anyhow::bail!("--steps must be a JSON array of step objects");
            }
            let body = serde_json::json!({ "name": name, "steps": steps_val });
            client
                .post(&format!("/api/v1/sites/{site}/funnels"), &body)
                .await?
        }
    };

    if human {
        print_human(&result);
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }

    Ok(())
}

/// Shared request builder for the `top-*` dimension endpoints.
#[allow(clippy::too_many_arguments)]
async fn top_query(
    client: &ApiClient,
    site: &str,
    endpoint: &str,
    range: &str,
    from: &Option<String>,
    to: &Option<String>,
    limit: u32,
    filters: &[String],
) -> anyhow::Result<Value> {
    let rq = range_qs(range, from, to);
    let fq = filters_qs(filters);
    client
        .get(&format!(
            "/api/v1/sites/{site}/{endpoint}?{rq}&limit={limit}{fq}"
        ))
        .await
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
