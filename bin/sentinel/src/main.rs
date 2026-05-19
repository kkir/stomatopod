mod config;

use anyhow::Result;
use clap::Parser;
use config::SentinelConfig;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Parser)]
#[command(name = "sentinel", about = "Stomatopod AI firewall sidecar")]
struct Cli {
    /// Config file path.
    #[arg(long, default_value = "sentinel.toml")]
    config: String,

    /// Override the listen port.
    #[arg(long)]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let mut cfg = load_config(&cli.config)?;
    if let Some(p) = cli.port {
        cfg.listen.port = p;
    }

    info!(
        upstream = %cfg.upstream.url,
        vendor = %cfg.upstream.vendor,
        listen = format!("{}:{}", cfg.listen.host, cfg.listen.port),
        "sentinel skeleton up — proxy + control plane not yet implemented"
    );

    // Phase 0: skeleton only. Phases 2/3/4 will wire the proxy,
    // span batcher, and SSE control client.

    Ok(())
}

fn load_config(path: &str) -> Result<SentinelConfig> {
    use ::config::{Config as ConfigBuilder, Environment, File};
    let cfg = ConfigBuilder::builder()
        .add_source(File::with_name(path).required(false))
        .add_source(
            Environment::with_prefix("SENTINEL")
                .prefix_separator("_")
                .separator("__"),
        )
        .build()?;
    Ok(cfg.try_deserialize()?)
}
