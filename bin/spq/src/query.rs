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
        #[arg(long, default_value = "day")]
        granularity: String,
    },
    /// Top pages by traffic
    TopPages {
        #[arg(long)]
        site: String,
        #[arg(long, default_value = "30d")]
        range: String,
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    /// Top referrers
    TopReferrers {
        #[arg(long)]
        site: String,
        #[arg(long, default_value = "30d")]
        range: String,
        #[arg(long, default_value = "20")]
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
}

pub async fn run(cmd: &QueryCommand, client: &ApiClient, human: bool) -> anyhow::Result<()> {
    let result: Value = match cmd {
        QueryCommand::Pageviews {
            site,
            range,
            granularity,
        } => {
            client
                .get(&format!(
                    "/api/v1/sites/{site}/pageviews?range={range}&granularity={granularity}"
                ))
                .await?
        }
        QueryCommand::TopPages { site, range, limit } => {
            client
                .get(&format!(
                    "/api/v1/sites/{site}/top-pages?range={range}&limit={limit}"
                ))
                .await?
        }
        QueryCommand::TopReferrers { site, range, limit } => {
            client
                .get(&format!(
                    "/api/v1/sites/{site}/top-referrers?range={range}&limit={limit}"
                ))
                .await?
        }
        QueryCommand::Events {
            site,
            name,
            range,
            limit,
        } => {
            let name_param = name
                .as_deref()
                .map(|n| format!("&name={n}"))
                .unwrap_or_default();
            client
                .get(&format!(
                    "/api/v1/sites/{site}/events?range={range}&limit={limit}{name_param}"
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
    };

    if human {
        print_human(&result);
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }

    Ok(())
}

fn print_human(value: &Value) {
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
