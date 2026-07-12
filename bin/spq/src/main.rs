//! `spq` — Stomatopod query CLI.
//!
//! A read-first command-line client for the analytics API, built for humans
//! and LLM agents. Outputs JSON by default. Authenticates with a read-scoped
//! API key (`rk_...`) from `STOMATOPOD_TOKEN` or
//! `~/.config/stomatopod/credentials`.
#![recursion_limit = "512"]

mod alerts;
mod client;
mod query;
mod req;
mod sites;
mod skills;

use clap::{Parser, Subcommand};

use client::ApiClient;

#[derive(Parser)]
#[command(name = "spq", about = "Stomatopod query CLI for humans and LLM agents")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Server base URL.
    #[arg(
        long,
        global = true,
        default_value = "http://localhost:8080",
        env = "STOMATOPOD_SERVER"
    )]
    server: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Query analytics data (JSON by default).
    Query {
        #[command(subcommand)]
        cmd: query::QueryCommand,
        /// Output a human-readable table instead of JSON.
        #[arg(long)]
        human: bool,
    },
    /// List sites visible to the credential.
    Sites {
        #[command(subcommand)]
        cmd: sites::SitesCommand,
    },
    /// Manage analytics alerts.
    Alerts {
        #[command(subcommand)]
        cmd: alerts::AlertsCommand,
    },
    /// Print a machine-readable description of every command (for LLM agents).
    Describe,
    /// Manage Claude Code skills bundled with spq.
    Skills {
        #[command(subcommand)]
        cmd: skills::SkillsCommand,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let client = ApiClient::new(cli.server.clone());

    match &cli.command {
        Commands::Query { cmd, human } => query::run(cmd, &client, *human).await,
        Commands::Sites { cmd } => sites::run(cmd, &client).await,
        Commands::Alerts { cmd } => alerts::run(cmd, &client).await,
        Commands::Describe => {
            println!("{}", serde_json::to_string_pretty(&describe())?);
            Ok(())
        }
        Commands::Skills { cmd } => skills::run(cmd),
    }
}

/// A self-describing manifest of the CLI surface so an LLM agent can discover
/// commands and arguments without scraping `--help`. Mirrors `/llms.txt`.
fn describe() -> serde_json::Value {
    serde_json::json!({
        "tool": "spq",
        "description": "Stomatopod analytics CLI for LLM agents: read-only queries plus funnel creation.",
        "auth": {
            "env": "STOMATOPOD_TOKEN",
            "credentials_file": "~/.config/stomatopod/credentials",
            "note": "Use a read-scoped API key (rk_...) minted in the dashboard."
        },
        "server": { "env": "STOMATOPOD_SERVER", "default": "http://localhost:8080" },
        "conventions": {
            "site": "A site ULID or its domain.",
            "range": ["7d", "30d", "90d", "12m"],
            "custom_range": "Pass --from/--to as YYYY-MM-DD to override the preset range.",
            "granularity": ["hour", "day", "week", "month"],
            "filter": "Repeatable --filter as field:op:value. Fields: url, referrer, country, region, browser, os, device_type, utm_source, utm_medium, utm_campaign, utm_term, utm_content, event_name. Ops: eq, not_eq, contains, starts_with.",
            "output": "JSON on stdout unless --human is passed."
        },
        "commands": [
            { "name": "sites", "description": "List sites visible to the credential.", "args": [] },
            {
                "name": "query pageviews",
                "description": "Pageview/session timeseries.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--range", "default": "30d" },
                    { "name": "--from", "required": false, "note": "YYYY-MM-DD; with --to overrides --range." },
                    { "name": "--to", "required": false, "note": "YYYY-MM-DD." },
                    { "name": "--granularity", "default": "day" },
                    { "name": "--compare", "required": false, "note": "Flag: attach prior-period totals under `comparison`." },
                    { "name": "--filter", "required": false, "note": "Repeatable field:op:value." }
                ]
            },
            {
                "name": "query top-pages",
                "description": "Top pages by traffic.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--range", "default": "30d" },
                    { "name": "--from", "required": false },
                    { "name": "--to", "required": false },
                    { "name": "--limit", "default": "20" },
                    { "name": "--filter", "required": false, "note": "Repeatable field:op:value." }
                ]
            },
            {
                "name": "query top-referrers",
                "description": "Top referrers by traffic.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--range", "default": "30d" },
                    { "name": "--from", "required": false },
                    { "name": "--to", "required": false },
                    { "name": "--limit", "default": "20" },
                    { "name": "--filter", "required": false, "note": "Repeatable field:op:value." }
                ]
            },
            {
                "name": "query top-os",
                "description": "Top operating systems by traffic.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--range", "default": "30d" },
                    { "name": "--from", "required": false },
                    { "name": "--to", "required": false },
                    { "name": "--limit", "default": "20" },
                    { "name": "--filter", "required": false, "note": "Repeatable field:op:value." }
                ]
            },
            {
                "name": "query top-regions",
                "description": "Top regions by traffic.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--range", "default": "30d" },
                    { "name": "--from", "required": false },
                    { "name": "--to", "required": false },
                    { "name": "--limit", "default": "20" },
                    { "name": "--filter", "required": false, "note": "Repeatable field:op:value." }
                ]
            },
            {
                "name": "query top-entry-pages",
                "description": "Top entry (landing) pages by traffic.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--range", "default": "30d" },
                    { "name": "--from", "required": false },
                    { "name": "--to", "required": false },
                    { "name": "--limit", "default": "20" },
                    { "name": "--compare", "required": false, "note": "Flag: prior-period comparison." },
                    { "name": "--filter", "required": false, "note": "Repeatable field:op:value." }
                ]
            },
            {
                "name": "query top-exit-pages",
                "description": "Top exit pages by traffic.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--range", "default": "30d" },
                    { "name": "--from", "required": false },
                    { "name": "--to", "required": false },
                    { "name": "--limit", "default": "20" },
                    { "name": "--compare", "required": false, "note": "Flag: prior-period comparison." },
                    { "name": "--filter", "required": false, "note": "Repeatable field:op:value." }
                ]
            },
            {
                "name": "query events",
                "description": "Custom event breakdown.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--name", "required": false },
                    { "name": "--range", "default": "30d" },
                    { "name": "--from", "required": false },
                    { "name": "--to", "required": false },
                    { "name": "--limit", "default": "20" },
                    { "name": "--filter", "required": false, "note": "Repeatable field:op:value." }
                ]
            },
            {
                "name": "query campaigns",
                "description": "UTM campaign breakdowns (source/medium/campaign/term/content).",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--range", "default": "30d" },
                    { "name": "--from", "required": false },
                    { "name": "--to", "required": false },
                    { "name": "--limit", "default": "20" },
                    { "name": "--filter", "required": false, "note": "Repeatable field:op:value." }
                ]
            },
            {
                "name": "query utm",
                "description": "Single-dimension UTM breakdown.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--dimension", "required": true, "note": "source|medium|campaign|term|content." },
                    { "name": "--range", "default": "30d" },
                    { "name": "--from", "required": false },
                    { "name": "--to", "required": false },
                    { "name": "--limit", "default": "20" },
                    { "name": "--utm-source", "required": false },
                    { "name": "--utm-medium", "required": false },
                    { "name": "--filter", "required": false, "note": "Repeatable field:op:value." }
                ]
            },
            {
                "name": "alerts list",
                "description": "List analytics alerts for a site.",
                "args": [ { "name": "--site", "required": true } ]
            },
            {
                "name": "alerts create",
                "description": "Create an analytics alert (requires a write-capable key). Fires go to every notification channel on the site.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--type", "required": true, "note": "e.g. traffic_spike, traffic_drop, new_referrer_spike." },
                    { "name": "--threshold", "required": true },
                    { "name": "--window", "default": "60", "note": "Window in minutes." }
                ]
            },
            {
                "name": "alerts delete",
                "description": "Delete an analytics alert by id.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--alert", "required": true }
                ]
            },
            {
                "name": "alerts toggle",
                "description": "Enable or disable an analytics alert.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--alert", "required": true },
                    { "name": "--enabled", "required": true, "note": "true|false." }
                ]
            },
            {
                "name": "query funnels",
                "description": "List funnels defined for a site.",
                "args": [ { "name": "--site", "required": true } ]
            },
            {
                "name": "query funnel",
                "description": "Run a funnel conversion query.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--funnel", "required": true },
                    { "name": "--range", "default": "30d" }
                ]
            },
            {
                "name": "query funnel-create",
                "description": "Create a new funnel. Requires a read key (rk_); writes are confined to the key's org/site.",
                "args": [
                    { "name": "--site", "required": true },
                    { "name": "--name", "required": true },
                    {
                        "name": "--steps",
                        "required": true,
                        "note": "JSON array of step objects: [{\"name\":..,\"event_name\":..,\"filters\":[]}]. Minimum 2 steps."
                    }
                ]
            }
        ]
    })
}
