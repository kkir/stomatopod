use clap::Subcommand;
use serde_json::Value;

use crate::client::ApiClient;

#[derive(Subcommand)]
pub enum SitesCommand {
    /// List all sites
    List,
    /// Create a new site
    Create {
        #[arg(long)]
        domain: String,
        #[arg(long)]
        name: String,
    },
}

pub async fn run(cmd: &SitesCommand, client: &ApiClient) -> anyhow::Result<()> {
    match cmd {
        SitesCommand::List => {
            let result: Value = client.get("/api/v1/sites").await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        SitesCommand::Create { domain, name } => {
            // POST is handled separately; show instructions for now
            println!("To create a site, POST to /api/v1/sites:");
            println!(r#"  {{"domain": "{domain}", "name": "{name}"}}"#,);
        }
    }
    Ok(())
}
