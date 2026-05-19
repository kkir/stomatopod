use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Result;
use axum::{routing::any, Router};
use clap::Parser;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use sentinel::{
    client::{SessionRegistry, SpanShipper},
    config::SentinelConfig,
    control::{run_control_loop, ControlState},
    policy::{PolicyConfig, PolicyEngine},
    proxy::{self, ProxyState},
};

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
    let cfg = Arc::new(cfg);

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let spool_dir = std::env::temp_dir().join("sentinel-spool");
    let shipper = SpanShipper::new(
        cfg.server.url.clone(),
        cfg.server.token.clone(),
        spool_dir.clone(),
    );

    let policy = Arc::new(PolicyEngine::new(PolicyConfig {
        repetition_max: cfg.policy.repetition_max.or(Some(5)),
        velocity_max_tps: cfg.policy.velocity_max_tps.or(Some(2000.0)),
        cost_cap_usd: cfg.policy.cost_cap_usd.or(Some(10.0)),
        ..PolicyConfig::default()
    }));

    let control = ControlState::new();
    let sessions = Arc::new(SessionRegistry::default());
    // Seed session from config — sidecars normally serve a single
    // long-lived agent process.
    let agent_id = cfg
        .server
        .agent_id
        .clone()
        .or_else(|| {
            std::env::var("HOSTNAME").ok().map(|h| {
                let pid = std::process::id();
                format!("{h}-{pid}")
            })
        })
        .unwrap_or_else(|| format!("sentinel-{}", std::process::id()));
    sessions.set(agent_id.clone(), default_session());

    // SSE control loop in the background.
    let ctl_state = control.clone();
    let ctl_url = cfg.server.url.clone();
    let ctl_token = cfg.server.token.clone();
    tokio::spawn(async move {
        run_control_loop(ctl_url, ctl_token, ctl_state).await;
    });

    let state = Arc::new(ProxyState {
        cfg: cfg.clone(),
        http,
        shipper,
        policy,
        control,
        sessions,
    });

    let app = Router::new()
        .route("/*path", any(proxy::handle))
        .route("/", any(proxy::handle))
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", cfg.listen.host, cfg.listen.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    info!(
        upstream = %cfg.upstream.url,
        vendor = %cfg.upstream.vendor,
        agent_id = %agent_id,
        spool_dir = %spool_dir.display(),
        "sentinel listening on http://{addr}"
    );

    axum::serve(listener, app).await?;
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

fn default_session() -> String {
    // ULID is monotonic per-process — fine for the single-session
    // sidecar default. Agents that want multiple logical sessions can
    // set a header to override (future work).
    let _ = PathBuf::new();
    ulid::Ulid::new().to_string()
}
