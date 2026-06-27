//! `spq goals` — list, create, and delete conversion goals.

use clap::Subcommand;

use crate::client::ApiClient;
use crate::query::filter_objects;
use crate::req::Req;

#[derive(Subcommand)]
pub enum GoalsCommand {
    /// List goals for a site.
    List {
        #[arg(long)]
        site: String,
    },
    /// Create a goal (requires a write-capable key).
    Create {
        #[arg(long)]
        site: String,
        /// Display name for the goal.
        #[arg(long)]
        name: String,
        /// Custom event that counts as a conversion.
        #[arg(long)]
        event: String,
        /// Optional filters as `field:op:value` (repeatable), AND-combined.
        #[arg(long = "filter")]
        filters: Vec<String>,
    },
    /// Delete a goal by id.
    Delete {
        #[arg(long)]
        site: String,
        #[arg(long)]
        goal: String,
    },
}

pub fn build(cmd: &GoalsCommand) -> anyhow::Result<Req> {
    let req = match cmd {
        GoalsCommand::List { site } => Req::Get(format!("/api/v1/sites/{site}/goals")),
        GoalsCommand::Create {
            site,
            name,
            event,
            filters,
        } => {
            let body = serde_json::json!({
                "name": name,
                "event_name": event,
                "filters": filter_objects(filters)?,
            });
            Req::Post(format!("/api/v1/sites/{site}/goals"), body)
        }
        GoalsCommand::Delete { site, goal } => {
            Req::Delete(format!("/api/v1/sites/{site}/goals/{goal}"))
        }
    };
    Ok(req)
}

pub async fn run(cmd: &GoalsCommand, client: &ApiClient) -> anyhow::Result<()> {
    match build(cmd)?.send(client).await? {
        Some(value) => println!("{}", serde_json::to_string_pretty(&value)?),
        None => {
            if let GoalsCommand::Delete { goal, .. } = cmd {
                println!("deleted {goal}");
            }
        }
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
        cmd: GoalsCommand,
    }

    fn build_args(args: &[&str]) -> Req {
        let mut full = vec!["spq"];
        full.extend_from_slice(args);
        build(&Harness::try_parse_from(full).expect("parse").cmd).expect("build")
    }

    #[test]
    fn list_goals() {
        assert_eq!(
            build_args(&["list", "--site", "s"]),
            Req::Get("/api/v1/sites/s/goals".into())
        );
    }

    #[test]
    fn create_goal_with_filter() {
        match build_args(&[
            "create",
            "--site",
            "s",
            "--name",
            "Signup",
            "--event",
            "user_signed_up",
            "--filter",
            "plan:eq:pro",
        ]) {
            Req::Post(path, body) => {
                assert_eq!(path, "/api/v1/sites/s/goals");
                assert_eq!(body["name"], "Signup");
                assert_eq!(body["event_name"], "user_signed_up");
                assert_eq!(body["filters"][0]["field"], "plan");
                assert_eq!(body["filters"][0]["value"], "pro");
            }
            other => panic!("expected POST, got {other:?}"),
        }
    }

    #[test]
    fn delete_goal() {
        assert_eq!(
            build_args(&["delete", "--site", "s", "--goal", "g1"]),
            Req::Delete("/api/v1/sites/s/goals/g1".into())
        );
    }
}
