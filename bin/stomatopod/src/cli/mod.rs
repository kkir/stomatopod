pub mod client;
pub mod query;
pub mod sites;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "stomatopod", about = "Stomatopod analytics CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Config file path
    #[arg(long, default_value = "stomatopod.toml")]
    pub config: String,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the analytics server
    Serve {
        #[arg(long)]
        port: Option<u16>,
    },
    /// Query analytics data (outputs JSON for AI agent consumption).
    ///
    /// Authenticates with the bearer credential in STOMATOPOD_TOKEN (or
    /// ~/.config/stomatopod/credentials). Use a read-scoped API key (`rk_...`)
    /// minted under a site's "API Keys" page in the dashboard.
    Query {
        #[command(subcommand)]
        cmd: query::QueryCommand,
        /// Server URL
        #[arg(
            long,
            default_value = "http://localhost:8080",
            env = "STOMATOPOD_SERVER"
        )]
        server: String,
        /// Output human-readable table instead of JSON
        #[arg(long)]
        human: bool,
    },
    /// Manage sites
    Sites {
        #[command(subcommand)]
        cmd: sites::SitesCommand,
        /// Server URL
        #[arg(
            long,
            default_value = "http://localhost:8080",
            env = "STOMATOPOD_SERVER"
        )]
        server: String,
    },
}
