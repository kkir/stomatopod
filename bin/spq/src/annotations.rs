use clap::Subcommand;
use serde_json::Value;

use crate::client::ApiClient;

#[derive(Subcommand)]
pub enum AnnotationsCommand {
    /// List annotations for a site within a range.
    List {
        #[arg(long)]
        site: String,
        #[arg(long, default_value = "90d")]
        range: String,
    },
    /// Create an annotation (requires a write-capable key).
    Create {
        #[arg(long)]
        site: String,
        /// Annotation date as YYYY-MM-DD.
        #[arg(long)]
        date: String,
        /// Label text. `--text` is accepted as an alias.
        #[arg(long = "label", alias = "text")]
        label: String,
        /// Optional longer note, appended to the label.
        #[arg(long)]
        note: Option<String>,
    },
    /// Delete an annotation by id.
    Delete {
        #[arg(long)]
        site: String,
        #[arg(long)]
        id: String,
    },
}

pub async fn run(cmd: &AnnotationsCommand, client: &ApiClient) -> anyhow::Result<()> {
    match cmd {
        AnnotationsCommand::List { site, range } => {
            let result: Value = client
                .get(&format!("/api/v1/sites/{site}/annotations?range={range}"))
                .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        AnnotationsCommand::Create {
            site,
            date,
            label,
            note,
        } => {
            let text = match note {
                Some(n) if !n.is_empty() => format!("{label} — {n}"),
                _ => label.clone(),
            };
            let body = serde_json::json!({ "date": date, "text": text });
            let result: Value = client
                .post(&format!("/api/v1/sites/{site}/annotations"), &body)
                .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        AnnotationsCommand::Delete { site, id } => {
            client
                .delete(&format!("/api/v1/sites/{site}/annotations/{id}"))
                .await?;
            println!("deleted {id}");
        }
    }
    Ok(())
}
