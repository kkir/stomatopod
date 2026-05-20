mod cli;

use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use clap::Parser;
use config::{Config as ConfigBuilder, Environment, File};
use minijinja::Environment as JinjaEnv;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use dashmap::DashMap;
use stomatopod_core::config::{Config, Mode, StorageConfig};
use stomatopod_ingest::{batch::run_batcher, geo::GeoLookup, span_batch::run_span_batcher};
use stomatopod_store::{
    clickhouse::ClickhouseBackend, embedded::EmbeddedBackend, postgres::PostgresBackend,
};
use stomatopod_web::{
    alerts::{run_alert_dispatcher, AlertDispatcher},
    router::build_router,
    state::AppState,
};

use cli::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

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
        Commands::Query { cmd, server, human } => {
            let client = cli::client::ApiClient::new(server.clone());
            cli::query::run(cmd, &client, *human).await
        }
        Commands::Sites { cmd, server } => {
            let client = cli::client::ApiClient::new(server.clone());
            cli::sites::run(cmd, &client).await
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
    // store — wiring that split is out of scope here, so the ClickHouse
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

    // Build minijinja environment
    let templates = build_templates()?;

    // Compute tracker hash for cache-busting
    let tracker_src = include_str!("../../../assets/tracker.js");
    let tracker_hash = {
        let h = blake3::hash(tracker_src.as_bytes());
        hex::encode(&h.as_bytes()[..8])
    };

    // Marketing-site static assets are also embedded and cache-busted by hash.
    let marketing_css_hash = {
        let h = blake3::hash(stomatopod_web::routes::api::MARKETING_CSS.as_bytes());
        hex::encode(&h.as_bytes()[..8])
    };
    let anime_js_hash = {
        let h = blake3::hash(stomatopod_web::routes::api::ANIME_JS.as_bytes());
        hex::encode(&h.as_bytes()[..8])
    };

    let redact_keys = Arc::new(cfg.sentinel.redact_keys.clone());
    let (alerts, alerts_rx) = AlertDispatcher::channel();
    let meta_for_alerts = meta.clone();
    tokio::spawn(async move {
        run_alert_dispatcher(alerts_rx, meta_for_alerts).await;
    });

    let state = Arc::new(AppState {
        backend,
        agent_store,
        meta,
        templates,
        config: cfg.clone(),
        tracker_hash,
        marketing_css_hash,
        anime_js_hash,
        ingest_tx,
        span_ingest_tx,
        site_cache: Arc::new(DashMap::new()),
        sentinel_token_cache: Arc::new(DashMap::new()),
        redact_keys,
        geo,
        control_channels: dashmap::DashMap::new(),
        control_seq: std::sync::atomic::AtomicU64::new(0),
        alerts,
    });

    let router = build_router(state);

    let addr: SocketAddr = format!("{}:{}", cfg.listen.host, cfg.listen.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    info!("Stomatopod listening on http://{addr}");

    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
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

fn build_templates() -> Result<JinjaEnv<'static>> {
    let mut env = JinjaEnv::new();
    env.set_auto_escape_callback(|name| {
        if name.ends_with(".html") {
            minijinja::AutoEscape::Html
        } else {
            minijinja::AutoEscape::None
        }
    });

    // Embed templates at compile time
    env.add_template(
        "base.html",
        include_str!("../../../crates/web/templates/base.html"),
    )?;
    env.add_template(
        "login.html",
        include_str!("../../../crates/web/templates/login.html"),
    )?;
    env.add_template(
        "index.html",
        include_str!("../../../crates/web/templates/index.html"),
    )?;
    env.add_template(
        "site.html",
        include_str!("../../../crates/web/templates/site.html"),
    )?;
    env.add_template(
        "events.html",
        include_str!("../../../crates/web/templates/events.html"),
    )?;
    env.add_template(
        "funnels.html",
        include_str!("../../../crates/web/templates/funnels.html"),
    )?;
    env.add_template(
        "partials/top_pages.html",
        include_str!("../../../crates/web/templates/partials/top_pages.html"),
    )?;
    env.add_template(
        "partials/top_referrers.html",
        include_str!("../../../crates/web/templates/partials/top_referrers.html"),
    )?;
    env.add_template(
        "partials/top_countries.html",
        include_str!("../../../crates/web/templates/partials/top_countries.html"),
    )?;
    env.add_template(
        "partials/top_browsers.html",
        include_str!("../../../crates/web/templates/partials/top_browsers.html"),
    )?;
    env.add_template(
        "partials/top_devices.html",
        include_str!("../../../crates/web/templates/partials/top_devices.html"),
    )?;
    env.add_template(
        "agents.html",
        include_str!("../../../crates/web/templates/agents.html"),
    )?;
    env.add_template(
        "agent.html",
        include_str!("../../../crates/web/templates/agent.html"),
    )?;
    env.add_template(
        "incidents.html",
        include_str!("../../../crates/web/templates/incidents.html"),
    )?;
    env.add_template(
        "partials/agent_spans.html",
        include_str!("../../../crates/web/templates/partials/agent_spans.html"),
    )?;

    // Marketing site
    env.add_template(
        "marketing/base.html",
        include_str!("../../../crates/web/templates/marketing/base.html"),
    )?;
    env.add_template(
        "marketing/home.html",
        include_str!("../../../crates/web/templates/marketing/home.html"),
    )?;
    env.add_template(
        "marketing/web_analytics.html",
        include_str!("../../../crates/web/templates/marketing/web_analytics.html"),
    )?;
    env.add_template(
        "marketing/ai_firewall.html",
        include_str!("../../../crates/web/templates/marketing/ai_firewall.html"),
    )?;
    env.add_template(
        "marketing/for_saas.html",
        include_str!("../../../crates/web/templates/marketing/for_saas.html"),
    )?;
    env.add_template(
        "marketing/for_agencies.html",
        include_str!("../../../crates/web/templates/marketing/for_agencies.html"),
    )?;
    env.add_template(
        "marketing/for_ai_teams.html",
        include_str!("../../../crates/web/templates/marketing/for_ai_teams.html"),
    )?;
    env.add_template(
        "marketing/for_regulated.html",
        include_str!("../../../crates/web/templates/marketing/for_regulated.html"),
    )?;

    Ok(env)
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
