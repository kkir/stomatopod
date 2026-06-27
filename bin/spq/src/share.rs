//! `spq share` — manage read-only dashboard share links.

use clap::Subcommand;

use crate::client::ApiClient;
use crate::req::Req;

#[derive(Subcommand)]
pub enum ShareCommand {
    /// List share links for a site.
    List {
        #[arg(long)]
        site: String,
    },
    /// Create a share link (requires a write-capable key).
    Create {
        #[arg(long)]
        site: String,
        /// Optional human-readable label.
        #[arg(long)]
        label: Option<String>,
        /// Optional expiry date as YYYY-MM-DD.
        #[arg(long)]
        expires: Option<String>,
    },
    /// Revoke a share link by id.
    Revoke {
        #[arg(long)]
        site: String,
        #[arg(long)]
        link: String,
    },
}

pub fn build(cmd: &ShareCommand) -> anyhow::Result<Req> {
    let req = match cmd {
        ShareCommand::List { site } => Req::Get(format!("/api/v1/sites/{site}/share-links")),
        ShareCommand::Create {
            site,
            label,
            expires,
        } => {
            let mut body = serde_json::Map::new();
            if let Some(l) = label {
                body.insert("label".into(), serde_json::Value::String(l.clone()));
            }
            if let Some(e) = expires {
                body.insert("expires_at".into(), serde_json::Value::String(e.clone()));
            }
            Req::Post(
                format!("/api/v1/sites/{site}/share-links"),
                serde_json::Value::Object(body),
            )
        }
        ShareCommand::Revoke { site, link } => {
            Req::Delete(format!("/api/v1/sites/{site}/share-links/{link}"))
        }
    };
    Ok(req)
}

pub async fn run(cmd: &ShareCommand, client: &ApiClient) -> anyhow::Result<()> {
    match build(cmd)?.send(client).await? {
        Some(value) => println!("{}", serde_json::to_string_pretty(&value)?),
        None => {
            if let ShareCommand::Revoke { link, .. } = cmd {
                println!("revoked {link}");
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
        cmd: ShareCommand,
    }

    fn build_args(args: &[&str]) -> Req {
        let mut full = vec!["spq"];
        full.extend_from_slice(args);
        build(&Harness::try_parse_from(full).expect("parse").cmd).expect("build")
    }

    #[test]
    fn list_share() {
        assert_eq!(
            build_args(&["list", "--site", "s"]),
            Req::Get("/api/v1/sites/s/share-links".into())
        );
    }

    #[test]
    fn create_share_with_label_and_expiry() {
        match build_args(&[
            "create",
            "--site",
            "s",
            "--label",
            "Client view",
            "--expires",
            "2025-12-31",
        ]) {
            Req::Post(path, body) => {
                assert_eq!(path, "/api/v1/sites/s/share-links");
                assert_eq!(body["label"], "Client view");
                assert_eq!(body["expires_at"], "2025-12-31");
            }
            other => panic!("expected POST, got {other:?}"),
        }
    }

    #[test]
    fn revoke_share() {
        assert_eq!(
            build_args(&["revoke", "--site", "s", "--link", "l1"]),
            Req::Delete("/api/v1/sites/s/share-links/l1".into())
        );
    }
}
