//! `spq alerts` - manage analytics alerts (traffic spikes/drops, referrer spikes).

use clap::Subcommand;

use crate::client::ApiClient;
use crate::req::Req;

#[derive(Subcommand)]
pub enum AlertsCommand {
    /// List alerts for a site.
    List {
        #[arg(long)]
        site: String,
    },
    /// Create an alert (requires a write-capable key).
    Create {
        #[arg(long)]
        site: String,
        /// Alert type, e.g. `traffic_spike` or `traffic_drop`.
        #[arg(long = "type")]
        alert_type: String,
        /// Trigger threshold (percent).
        #[arg(long)]
        threshold: f64,
        /// Evaluation window in minutes.
        #[arg(long, default_value = "60")]
        window: u32,
        /// Alert channel id to notify.
        #[arg(long)]
        channel: String,
    },
    /// Delete an alert by id.
    Delete {
        #[arg(long)]
        site: String,
        #[arg(long)]
        alert: String,
    },
    /// Enable or disable an alert.
    Toggle {
        #[arg(long)]
        site: String,
        #[arg(long)]
        alert: String,
        /// Whether the alert should be enabled.
        #[arg(long, action = clap::ArgAction::Set, required = true)]
        enabled: bool,
    },
}

pub fn build(cmd: &AlertsCommand) -> anyhow::Result<Req> {
    let req = match cmd {
        AlertsCommand::List { site } => Req::Get(format!("/api/v1/sites/{site}/analytics-alerts")),
        AlertsCommand::Create {
            site,
            alert_type,
            threshold,
            window,
            channel,
        } => {
            let body = serde_json::json!({
                "type": alert_type,
                "threshold": threshold,
                "window_minutes": window,
                "channel_id": channel,
            });
            Req::Post(format!("/api/v1/sites/{site}/analytics-alerts"), body)
        }
        AlertsCommand::Delete { site, alert } => {
            Req::Delete(format!("/api/v1/sites/{site}/analytics-alerts/{alert}"))
        }
        AlertsCommand::Toggle {
            site,
            alert,
            enabled,
        } => {
            let body = serde_json::json!({ "enabled": enabled });
            Req::Patch(
                format!("/api/v1/sites/{site}/analytics-alerts/{alert}"),
                body,
            )
        }
    };
    Ok(req)
}

pub async fn run(cmd: &AlertsCommand, client: &ApiClient) -> anyhow::Result<()> {
    match build(cmd)?.send(client).await? {
        Some(value) => println!("{}", serde_json::to_string_pretty(&value)?),
        None => match cmd {
            AlertsCommand::Delete { alert, .. } => println!("deleted {alert}"),
            AlertsCommand::Toggle { alert, enabled, .. } => {
                println!("alert {alert} enabled={enabled}")
            }
            _ => {}
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Harness {
        #[command(subcommand)]
        cmd: AlertsCommand,
    }

    fn build_args(args: &[&str]) -> Req {
        let mut full = vec!["spq"];
        full.extend_from_slice(args);
        build(&Harness::try_parse_from(full).expect("parse").cmd).expect("build")
    }

    #[test]
    fn list_alerts() {
        assert_eq!(
            build_args(&["list", "--site", "s"]),
            Req::Get("/api/v1/sites/s/analytics-alerts".into())
        );
    }

    #[test]
    fn create_alert() {
        match build_args(&[
            "create",
            "--site",
            "s",
            "--type",
            "traffic_spike",
            "--threshold",
            "200",
            "--window",
            "60",
            "--channel",
            "ch1",
        ]) {
            Req::Post(path, body) => {
                assert_eq!(path, "/api/v1/sites/s/analytics-alerts");
                assert_eq!(body["type"], "traffic_spike");
                assert_eq!(body["threshold"], 200.0);
                assert_eq!(body["window_minutes"], 60);
                assert_eq!(body["channel_id"], "ch1");
            }
            other => panic!("expected POST, got {other:?}"),
        }
    }

    #[test]
    fn toggle_alert_patches() {
        match build_args(&[
            "toggle",
            "--site",
            "s",
            "--alert",
            "a1",
            "--enabled",
            "false",
        ]) {
            Req::Patch(path, body) => {
                assert_eq!(path, "/api/v1/sites/s/analytics-alerts/a1");
                assert_eq!(body["enabled"], false);
            }
            other => panic!("expected PATCH, got {other:?}"),
        }
    }

    #[test]
    fn delete_alert() {
        assert_eq!(
            build_args(&["delete", "--site", "s", "--alert", "a1"]),
            Req::Delete("/api/v1/sites/s/analytics-alerts/a1".into())
        );
    }
}
