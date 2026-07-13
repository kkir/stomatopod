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
use stomatopod_ingest::{batch::run_batcher, geo::GeoLookup};
use stomatopod_store::{embedded::EmbeddedBackend, postgres::PostgresBackend};

use crate::{middleware::auth::require_auth, router::build_router, state::AppState};

/// The Stomatopod server binary. Analytics querying lives in the separate
/// `stoma` CLI; this binary only runs the server.
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
    // Self-hosted is single-tenant / single-owner only. SaaS multi-org is not
    // implemented; refuse so operators never run multi-tenant traffic on
    // authz that assumes one admin for the whole instance.
    if cfg.mode == Mode::Saas {
        anyhow::bail!(
            "mode = \"saas\" is not supported yet. Use the default self-hosted mode \
             (single organization, single owner user). Multi-tenant SaaS is planned \
             for a future release."
        );
    }

    let cfg = Arc::new(cfg);

    // Build storage backend.
    let (backend, meta): (
        Arc<dyn stomatopod_core::traits::StorageBackend>,
        Arc<dyn stomatopod_core::traits::MetaStore>,
    ) = match &cfg.storage {
        StorageConfig::Embedded(emb_cfg) => {
            let backend = EmbeddedBackend::open(emb_cfg).await?;
            let backend = Arc::new(backend);
            (backend.clone(), backend)
        }
        StorageConfig::Postgres(pg_cfg) => {
            let backend = PostgresBackend::connect(pg_cfg).await?;
            backend.bootstrap().await?;
            let backend = Arc::new(backend);
            (backend.clone(), backend)
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

    // Geo lookup
    let geo = Arc::new(GeoLookup::new(cfg.geo.mmdb_path.as_deref()));

    // Compute tracker hash for cache-busting
    let tracker_src = include_str!("../../../../assets/tracker.js");
    let tracker_hash = {
        let h = blake3::hash(tracker_src.as_bytes());
        hex::encode(&h.as_bytes()[..8])
    };

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

    // Digest notifier + hourly scheduler. Digests post to each site's
    // configured Slack / Telegram / webhook channels.
    let digest_notifier: Arc<dyn crate::digest::DigestNotifier> =
        Arc::new(crate::digest::ChannelNotifier);
    {
        let meta_for_digest = meta.clone();
        let backend_for_digest = backend.clone();
        let notifier_for_digest = digest_notifier.clone();
        let base_url = cfg.public_base_url().to_string();
        tokio::spawn(async move {
            crate::digest::run_digest_scheduler(
                meta_for_digest,
                backend_for_digest,
                notifier_for_digest,
                base_url,
                std::time::Duration::from_secs(3600),
            )
            .await;
        });
    }

    // Optional event retention: prune old partitions/rows once a day.
    if let Some(days) = retention_days(&cfg.storage) {
        let backend_for_retention = backend.clone();
        tokio::spawn(async move {
            run_retention_loop(backend_for_retention, days).await;
        });
    }

    let listen_host = cfg.listen.host.clone();
    let listen_port = cfg.listen.port;

    let state = Arc::new(AppState {
        backend,
        meta,
        config: cfg,
        tracker_hash,
        ingest_tx,
        site_cache: Arc::new(DashMap::new()),
        api_key_cache: Arc::new(DashMap::new()),
        geo,
        digest_notifier,
        login_failures: Arc::new(DashMap::new()),
    });

    // REST/ingest/auth router (no catch-all fallback).
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

    // `dx serve` and PaaS platforms (Cloud Run, Coolify, etc.) assign the
    // listen port via the PORT environment variable; honor it when present.
    // The interface follows IP if set, otherwise the configured host (which
    // defaults to 0.0.0.0). Binding must NOT fall back to loopback here: a
    // process on 127.0.0.1 inside a container is unreachable from the
    // platform's reverse proxy, producing a 502.
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(listen_port);
    let host = std::env::var("IP").unwrap_or(listen_host);
    let addr: SocketAddr = format!("{host}:{port}").parse()?;
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
                .separator("__")
                // Coerce env-var strings into their target scalar type
                // (`true` -> bool, `200` -> int). Required for typed fields
                // under the internally-tagged `storage` enum
                // (e.g. STOMATOPOD_STORAGE__ALLOW_EPHEMERAL=true), which is
                // buffered through serde's self-describing path and would
                // otherwise reject the raw string with "invalid type: string".
                // `list_separator` is intentionally left unset so string
                // fields are not split into arrays.
                .try_parsing(true),
        )
        .build()?;
    Ok(cfg.try_deserialize()?)
}

/// Minimum length for the first-boot admin password.
const MIN_ADMIN_PASSWORD_LEN: usize = 12;

async fn bootstrap_self_hosted(
    meta: &Arc<dyn stomatopod_core::traits::MetaStore>,
    _cfg: &Config,
) -> Result<()> {
    use chrono::Utc;
    use stomatopod_core::domain::org::{Organization, Plan, User, UserRole};
    use ulid::Ulid;

    let orgs = meta.list_orgs().await?;
    if !orgs.is_empty() {
        // Self-hosted is a single-org appliance. Extra orgs (manual DB edits)
        // are not used by the API (handlers take the first org); warn so
        // operators know the second org is effectively dead weight.
        if orgs.len() > 1 {
            tracing::warn!(
                org_count = orgs.len(),
                "self-hosted mode expects a single organization; only the first is used"
            );
        }
        return Ok(());
    }

    // First boot: require an explicit admin password. Never ship a known
    // default like "changeme" — empty data dirs on the public internet would
    // otherwise be takeable in one login attempt.
    let password = match std::env::var("STOMATOPOD_ADMIN_PASSWORD") {
        Ok(p) if p.len() >= MIN_ADMIN_PASSWORD_LEN => p,
        Ok(_) => anyhow::bail!(
            "STOMATOPOD_ADMIN_PASSWORD must be at least {MIN_ADMIN_PASSWORD_LEN} characters \
             (first-boot admin user). Generate one with: openssl rand -base64 24"
        ),
        Err(_) => anyhow::bail!(
            "First boot requires STOMATOPOD_ADMIN_PASSWORD (min {MIN_ADMIN_PASSWORD_LEN} chars). \
             Example: export STOMATOPOD_ADMIN_PASSWORD=\"$(openssl rand -base64 24)\". \
             Optional: STOMATOPOD_ADMIN_EMAIL (default admin@localhost)."
        ),
    };
    let email =
        std::env::var("STOMATOPOD_ADMIN_EMAIL").unwrap_or_else(|_| "admin@localhost".into());

    // Create the single default org + owner user for this instance.
    let org = Organization {
        id: Ulid::new(),
        name: "Default Organization".into(),
        slug: "default".into(),
        plan: Plan::SelfHosted,
        created_at: Utc::now(),
    };
    meta.create_org(&org).await?;

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

    info!(
        "First-boot: created single-tenant org and owner user ({email}). \
         Self-hosted mode supports one owner account per instance."
    );
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

/// `Some(days)` when retention is enabled (`days > 0`).
fn retention_days(storage: &StorageConfig) -> Option<u64> {
    let days = match storage {
        StorageConfig::Embedded(c) => c.retention_days,
        StorageConfig::Postgres(c) => c.retention_days,
    };
    if days > 0 {
        Some(days)
    } else {
        None
    }
}

/// Periodic prune of events older than `retention_days`. Runs immediately
/// once at boot, then every 24h.
async fn run_retention_loop(
    backend: Arc<dyn stomatopod_core::traits::StorageBackend>,
    retention_days: u64,
) {
    use chrono::{Duration, Utc};
    let interval = std::time::Duration::from_secs(24 * 3600);
    loop {
        let cutoff = Utc::now() - Duration::days(retention_days as i64);
        match stomatopod_core::traits::StorageBackend::prune_events_before(backend.as_ref(), cutoff)
            .await
        {
            Ok(n) if n > 0 => info!(
                removed = n,
                retention_days,
                cutoff = %cutoff.to_rfc3339(),
                "retention prune removed old event data"
            ),
            Ok(_) => tracing::debug!(
                retention_days,
                cutoff = %cutoff.to_rfc3339(),
                "retention prune: nothing older than cutoff"
            ),
            Err(e) => tracing::warn!("retention prune failed: {e}"),
        }
        tokio::time::sleep(interval).await;
    }
}
