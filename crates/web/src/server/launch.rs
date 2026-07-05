//! The `stomatopod serve` command: load config, open storage, spawn the
//! background workers, and serve the Dioxus fullstack application (SSR +
//! hydration + static assets) merged onto the REST/ingest router.

use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum::middleware::from_fn_with_state;
use clap::Parser;
use config::{Config as ConfigBuilder, Environment, File};
use dioxus_server::{DioxusRouterExt, ServeConfig};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use dashmap::DashMap;
use stomatopod_core::config::{Config, Mode, StorageConfig};
use stomatopod_ingest::{batch::run_batcher, geo::GeoLookup, span_batch::run_span_batcher};
use stomatopod_store::{
    clickhouse::ClickhouseBackend, embedded::EmbeddedBackend, postgres::PostgresBackend,
};

use crate::{
    alerts::{run_alert_dispatcher, AlertDispatcher},
    middleware::auth::require_auth,
    router::build_router,
    state::AppState,
};

/// The Stomatopod server binary. Analytics querying lives in the separate
/// `spq` CLI; this binary only runs the server.
#[derive(Parser)]
#[command(name = "stomatopod", about = "Stomatopod analytics server")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Config file path.
    #[arg(long, default_value = "stomatopod.toml")]
    config: String,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Start the analytics server.
    Serve {
        #[arg(long)]
        port: Option<u16>,
    },
}

/// Synchronous entry point called from `main` on the native (server) build.
/// Builds a Tokio runtime and drives the async server to completion.
pub fn run() -> Result<()> {
    // Initialize tracing.
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async_main())
}

async fn async_main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Serve { port } => {
            let cfg = load_config(&cli.config)?;
            let cfg = if let Some(p) = port {
                Config {
                    listen: stomatopod_core::config::ListenConfig {
                        port: *p,
                        ..cfg.listen
                    },
                    ..cfg
                }
            } else {
                cfg
            };
            serve(cfg).await
        }
    }
}

async fn serve(cfg: Config) -> Result<()> {
    if cfg.auth.secret_key.is_empty() {
        anyhow::bail!(
            "auth.secret_key must be set. Set STOMATOPOD_AUTH__SECRET_KEY or add it to stomatopod.toml"
        );
    }

    let cfg = Arc::new(cfg);

    // Build storage backend. The Postgres backend owns metadata, events,
    // and spans in a single database; ClickHouse is analytics-only (no
    // MetaStore) and is intended to be paired with a Postgres metadata
    // store - wiring that split is out of scope here, so the ClickHouse
    // variant still bails with a precise message.
    let (backend, agent_store, meta): (
        Arc<dyn stomatopod_core::traits::StorageBackend>,
        Arc<dyn stomatopod_core::traits::AgentStore>,
        Arc<dyn stomatopod_core::traits::MetaStore>,
    ) = match &cfg.storage {
        StorageConfig::Embedded(emb_cfg) => {
            let backend = EmbeddedBackend::open(emb_cfg).await?;
            let backend = Arc::new(backend);
            (backend.clone(), backend.clone(), backend)
        }
        StorageConfig::Postgres(pg_cfg) => {
            let backend = PostgresBackend::connect(pg_cfg).await?;
            backend.bootstrap().await?;
            let backend = Arc::new(backend);
            (backend.clone(), backend.clone(), backend)
        }
        StorageConfig::Clickhouse(ch_cfg) => {
            // Force compile-time use of the symbol so the feature flag stays
            // wired up; the real ClickHouse deployment topology pairs this
            // with a separate Postgres MetaStore.
            let _ = ClickhouseBackend::new(ch_cfg);
            anyhow::bail!(
                "ClickHouse backend is implemented but not yet routed: SaaS deployments \
                 should pair it with a Postgres MetaStore. Use storage.kind = \"postgres\" \
                 for single-database SaaS, or storage.kind = \"embedded\" for self-hosted."
            )
        }
    };

    // Self-hosted: ensure a default org exists
    if cfg.mode == Mode::SelfHosted {
        bootstrap_self_hosted(&meta, &cfg).await?;
    }

    // Build ingest channel + batcher
    let (ingest_tx, ingest_rx) = tokio::sync::mpsc::channel(cfg.limits.ingest_channel_size);
    let batcher_backend = backend.clone();
    let batch_size = cfg.limits.ingest_batch_size;
    let flush_ms = cfg.limits.ingest_flush_interval_ms;
    tokio::spawn(async move {
        run_batcher(ingest_rx, batcher_backend, batch_size, flush_ms).await;
    });

    // Span ingest channel + batcher (parallel pipeline for the AI firewall).
    let (span_ingest_tx, span_ingest_rx) =
        tokio::sync::mpsc::channel(cfg.limits.ingest_channel_size);
    let span_store = agent_store.clone();
    tokio::spawn(async move {
        run_span_batcher(span_ingest_rx, span_store, batch_size, flush_ms).await;
    });

    // Geo lookup
    let geo = Arc::new(GeoLookup::new(cfg.geo.mmdb_path.as_deref()));

    // Compute tracker hash for cache-busting
    let tracker_src = include_str!("../../../../assets/tracker.js");
    let tracker_hash = {
        let h = blake3::hash(tracker_src.as_bytes());
        hex::encode(&h.as_bytes()[..8])
    };

    let redact_keys = Arc::new(cfg.sentinel.redact_keys.clone());
    let (alerts, alerts_rx) = AlertDispatcher::channel();
    let meta_for_alerts = meta.clone();
    tokio::spawn(async move {
        run_alert_dispatcher(alerts_rx, meta_for_alerts).await;
    });

    // Analytics alert evaluator: poll enabled alerts once a minute.
    let meta_for_eval = meta.clone();
    let backend_for_eval = backend.clone();
    tokio::spawn(async move {
        crate::alerts::run_analytics_alert_evaluator(
            meta_for_eval,
            backend_for_eval,
            std::time::Duration::from_secs(60),
        )
        .await;
    });

    // Email digest sender + hourly scheduler. The default LogSender keeps
    // self-hosted deployments side-effect free until a provider is wired.
    let digest_sender: Arc<dyn crate::digest::DigestSender> = Arc::new(crate::digest::LogSender);
    {
        let meta_for_digest = meta.clone();
        let backend_for_digest = backend.clone();
        let sender_for_digest = digest_sender.clone();
        let base_url = cfg.public_base_url().to_string();
        let secret = cfg.auth.secret_key.clone();
        tokio::spawn(async move {
            crate::digest::run_digest_scheduler(
                meta_for_digest,
                backend_for_digest,
                sender_for_digest,
                base_url,
                secret,
                std::time::Duration::from_secs(3600),
            )
            .await;
        });
    }

    let listen_host = cfg.listen.host.clone();
    let listen_port = cfg.listen.port;

    let state = Arc::new(AppState {
        backend,
        agent_store,
        meta,
        config: cfg,
        tracker_hash,
        ingest_tx,
        span_ingest_tx,
        site_cache: Arc::new(DashMap::new()),
        sentinel_token_cache: Arc::new(DashMap::new()),
        api_key_cache: Arc::new(DashMap::new()),
        redact_keys,
        geo,
        control_channels: dashmap::DashMap::new(),
        control_seq: std::sync::atomic::AtomicU64::new(0),
        alerts,
        digest_sender,
    });

    // REST/ingest/auth/agents router (no catch-all fallback).
    let rest = build_router(state.clone());

    // The Dioxus fullstack application: server functions, static assets, and
    // the SSR fallback for every client-side route. `AppState` is injected as
    // render/server-function context. The whole application is guarded by the
    // dashboard session middleware, which redirects unauthenticated requests
    // to `/login` before any SSR HTML or wasm is served.
    let serve_cfg = ServeConfig::new().context(state.clone());
    let dioxus_app = axum::Router::new()
        .serve_dioxus_application(serve_cfg, crate::ui::App)
        .layer(from_fn_with_state(state.clone(), require_auth));

    let app = rest.merge(dioxus_app);

    // `dx serve` (and PaaS platforms like Cloud Run) assign the listen address
    // via the IP/PORT environment variables. Honor those when present so the
    // Dioxus dev proxy and hot-reload can reach the server; otherwise fall back
    // to the configured host/port.
    let addr: SocketAddr = match std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
    {
        Some(port) => {
            let ip = std::env::var("IP")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
            SocketAddr::new(ip, port)
        }
        None => format!("{listen_host}:{listen_port}").parse()?,
    };
    let listener = TcpListener::bind(addr).await?;
    info!("Stomatopod listening on http://{addr}");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

fn load_config(path: &str) -> Result<Config> {
    let cfg = ConfigBuilder::builder()
        .add_source(File::with_name(path).required(false))
        .add_source(
            Environment::with_prefix("STOMATOPOD")
                .prefix_separator("_")
                .separator("__"),
        )
        .build()?;
    Ok(cfg.try_deserialize()?)
}

async fn bootstrap_self_hosted(
    meta: &Arc<dyn stomatopod_core::traits::MetaStore>,
    _cfg: &Config,
) -> Result<()> {
    use chrono::Utc;
    use stomatopod_core::domain::org::{Organization, Plan, User, UserRole};
    use ulid::Ulid;

    let orgs = meta.list_orgs().await?;
    if !orgs.is_empty() {
        return Ok(());
    }

    // Create default org
    let org = Organization {
        id: Ulid::new(),
        name: "Default Organization".into(),
        slug: "default".into(),
        plan: Plan::SelfHosted,
        created_at: Utc::now(),
    };
    meta.create_org(&org).await?;

    // Create admin user if STOMATOPOD_ADMIN_EMAIL/PASSWORD are set
    let email =
        std::env::var("STOMATOPOD_ADMIN_EMAIL").unwrap_or_else(|_| "admin@localhost".into());
    let password = std::env::var("STOMATOPOD_ADMIN_PASSWORD").unwrap_or_else(|_| "changeme".into());

    let hash = hash_password(&password)?;
    let user = User {
        id: Ulid::new(),
        org_id: org.id,
        email: email.clone(),
        password_hash: hash,
        role: UserRole::Owner,
        created_at: Utc::now(),
    };
    meta.create_user(&user).await?;

    info!("First-boot: created default org and admin user ({email})");
    Ok(())
}

fn hash_password(password: &str) -> Result<String> {
    use argon2::{
        password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
        Argon2,
    };
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("argon2 error: {e}"))?;
    Ok(hash.to_string())
}
