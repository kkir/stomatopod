use std::{path::Path, sync::Arc};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Mutex;
use tokio::task;
use ulid::Ulid;

use stomatopod_core::{
    domain::{
        agent::{AlertChannel, AlertChannelKind},
        analytics_alert::{AnalyticsAlert, AnalyticsAlertFire, AnalyticsAlertKind},
        api_key::{ApiKey, ApiKeyScope},
        digest::{DigestFrequency, DigestSubscription},
        org::{Funnel, Organization, Plan, User, UserRole},
        share_link::ShareLink,
        site::Site,
    },
    error::StoreError,
    traits::MetaStore,
};

pub struct SqliteMeta {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteMeta {
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        let path = path.to_path_buf();
        let conn = task::spawn_blocking(move || -> anyhow::Result<Connection> {
            let conn = Connection::open(&path)?;
            conn.execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA cache_size=-8000;
                 PRAGMA foreign_keys=ON;",
            )?;
            migrate(&conn)?;
            Ok(conn)
        })
        .await??;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

fn migrate(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_version (
            id            INTEGER PRIMARY KEY CHECK (id = 1),
            version       INTEGER NOT NULL,
            applied_at    TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS orgs (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            slug       TEXT UNIQUE NOT NULL,
            plan       TEXT NOT NULL DEFAULT 'self_hosted',
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS sites (
            id         TEXT PRIMARY KEY,
            org_id     TEXT NOT NULL REFERENCES orgs(id),
            domain     TEXT NOT NULL,
            name       TEXT NOT NULL,
            timezone   TEXT NOT NULL DEFAULT 'UTC',
            public_key TEXT UNIQUE NOT NULL,
            created_at TEXT NOT NULL,
            is_active  INTEGER NOT NULL DEFAULT 1
        );
        CREATE INDEX IF NOT EXISTS idx_sites_public_key ON sites(public_key);
        CREATE INDEX IF NOT EXISTS idx_sites_org_id ON sites(org_id);

        CREATE TABLE IF NOT EXISTS users (
            id            TEXT PRIMARY KEY,
            org_id        TEXT NOT NULL REFERENCES orgs(id),
            email         TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            role          TEXT NOT NULL DEFAULT 'owner',
            created_at    TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);

        CREATE TABLE IF NOT EXISTS funnels (
            id         TEXT PRIMARY KEY,
            site_id    TEXT NOT NULL REFERENCES sites(id),
            name       TEXT NOT NULL,
            definition TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_funnels_site_id ON funnels(site_id);

        CREATE TABLE IF NOT EXISTS api_keys (
            id             TEXT PRIMARY KEY,
            org_id         TEXT NOT NULL REFERENCES orgs(id),
            site_id        TEXT REFERENCES sites(id),
            name           TEXT NOT NULL,
            scope          TEXT NOT NULL,
            key_hash       TEXT UNIQUE NOT NULL,
            display_prefix TEXT NOT NULL,
            created_at     TEXT NOT NULL,
            last_used_at   TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash);
        CREATE INDEX IF NOT EXISTS idx_api_keys_org ON api_keys(org_id);

        CREATE TABLE IF NOT EXISTS alert_channels (
            id              TEXT PRIMARY KEY,
            site_id         TEXT NOT NULL REFERENCES sites(id),
            kind            TEXT NOT NULL,
            url             TEXT NOT NULL,
            secret          TEXT,
            created_at      TEXT NOT NULL,
            last_error_at   TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_alert_channels_site ON alert_channels(site_id);

        CREATE TABLE IF NOT EXISTS analytics_alerts (
            id          TEXT PRIMARY KEY,
            site_id     TEXT NOT NULL REFERENCES sites(id),
            type        TEXT NOT NULL,
            config      TEXT NOT NULL,
            channel_id  TEXT NOT NULL REFERENCES alert_channels(id),
            enabled     INTEGER NOT NULL DEFAULT 1,
            created_at  TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_analytics_alerts_site ON analytics_alerts(site_id);
        CREATE INDEX IF NOT EXISTS idx_analytics_alerts_enabled ON analytics_alerts(enabled);

        CREATE TABLE IF NOT EXISTS analytics_alert_fires (
            id          TEXT PRIMARY KEY,
            alert_id    TEXT NOT NULL REFERENCES analytics_alerts(id),
            fired_at    TEXT NOT NULL,
            payload     TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_alert_fires_alert ON analytics_alert_fires(alert_id, fired_at DESC);

        CREATE TABLE IF NOT EXISTS share_links (
            id          TEXT PRIMARY KEY,
            site_id     TEXT NOT NULL REFERENCES sites(id),
            token       TEXT UNIQUE NOT NULL,
            label       TEXT,
            expires_at  TEXT,
            created_by  TEXT NOT NULL,
            created_at  TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_share_links_site ON share_links(site_id);
        CREATE INDEX IF NOT EXISTS idx_share_links_token ON share_links(token);

        CREATE TABLE IF NOT EXISTS digest_subscriptions (
            id           TEXT PRIMARY KEY,
            user_id      TEXT NOT NULL REFERENCES users(id),
            site_id      TEXT NOT NULL REFERENCES sites(id),
            frequency    TEXT NOT NULL,
            enabled      INTEGER NOT NULL DEFAULT 1,
            bounce_count INTEGER NOT NULL DEFAULT 0,
            created_at   TEXT NOT NULL,
            UNIQUE(user_id, site_id)
        );
        CREATE INDEX IF NOT EXISTS idx_digest_subs_enabled ON digest_subscriptions(enabled);

        INSERT OR IGNORE INTO schema_version (id, version, applied_at)
        VALUES (1, 1, datetime('now'));
        "#,
    )?;
    Ok(())
}

fn row_to_site(row: &rusqlite::Row<'_>) -> rusqlite::Result<Site> {
    let id_str: String = row.get(0)?;
    let org_id_str: String = row.get(1)?;
    let created_at_str: String = row.get(6)?;
    let is_active: i32 = row.get(7)?;
    Ok(Site {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        org_id: Ulid::from_string(&org_id_str).unwrap_or_default(),
        domain: row.get(2)?,
        name: row.get(3)?,
        timezone: row.get(4)?,
        public_key: row.get(5)?,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        is_active: is_active != 0,
    })
}

fn row_to_org(row: &rusqlite::Row<'_>) -> rusqlite::Result<Organization> {
    let id_str: String = row.get(0)?;
    let _plan_str: String = row.get(3)?;
    let created_at_str: String = row.get(4)?;
    Ok(Organization {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        name: row.get(1)?,
        slug: row.get(2)?,
        // Map legacy free/pro/enterprise rows to SelfHosted.
        plan: Plan::SelfHosted,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
}

fn row_to_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    let id_str: String = row.get(0)?;
    let org_id_str: String = row.get(1)?;
    let role_str: String = row.get(4)?;
    let created_at_str: String = row.get(5)?;
    Ok(User {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        org_id: Ulid::from_string(&org_id_str).unwrap_or_default(),
        email: row.get(2)?,
        password_hash: row.get(3)?,
        role: match role_str.as_str() {
            "admin" => UserRole::Admin,
            "viewer" => UserRole::Viewer,
            _ => UserRole::Owner,
        },
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
}

fn parse_utc(s: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn parse_utc_opt(s: Option<String>) -> Option<chrono::DateTime<Utc>> {
    s.as_deref().map(parse_utc)
}

fn row_to_api_key(row: &rusqlite::Row<'_>) -> rusqlite::Result<ApiKey> {
    let id_str: String = row.get(0)?;
    let org_id_str: String = row.get(1)?;
    let site_id_str: Option<String> = row.get(2)?;
    let scope_str: String = row.get(4)?;
    let created_at_str: String = row.get(7)?;
    let last_used_at_str: Option<String> = row.get(8)?;
    Ok(ApiKey {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        org_id: Ulid::from_string(&org_id_str).unwrap_or_default(),
        site_id: site_id_str
            .as_deref()
            .and_then(|s| Ulid::from_string(s).ok()),
        name: row.get(3)?,
        scope: ApiKeyScope::parse(&scope_str).unwrap_or(ApiKeyScope::Read),
        key_hash: row.get(5)?,
        display_prefix: row.get(6)?,
        created_at: parse_utc(&created_at_str),
        last_used_at: parse_utc_opt(last_used_at_str),
    })
}

fn row_to_alert_channel(row: &rusqlite::Row<'_>) -> rusqlite::Result<AlertChannel> {
    let id_str: String = row.get(0)?;
    let site_id_str: String = row.get(1)?;
    let kind_str: String = row.get(2)?;
    let created_at_str: String = row.get(5)?;
    let last_error_at_str: Option<String> = row.get(6)?;
    Ok(AlertChannel {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        site_id: Ulid::from_string(&site_id_str).unwrap_or_default(),
        kind: AlertChannelKind::from_str(&kind_str),
        url: row.get(3)?,
        secret: row.get(4)?,
        created_at: parse_utc(&created_at_str),
        last_error_at: parse_utc_opt(last_error_at_str),
    })
}

fn row_to_funnel(row: &rusqlite::Row<'_>) -> rusqlite::Result<Funnel> {
    let id_str: String = row.get(0)?;
    let site_id_str: String = row.get(1)?;
    let created_at_str: String = row.get(4)?;
    Ok(Funnel {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        site_id: Ulid::from_string(&site_id_str).unwrap_or_default(),
        name: row.get(2)?,
        definition: row.get(3)?,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
}

fn row_to_analytics_alert(row: &rusqlite::Row<'_>) -> rusqlite::Result<AnalyticsAlert> {
    let id_str: String = row.get(0)?;
    let site_id_str: String = row.get(1)?;
    let type_str: String = row.get(2)?;
    let config_str: String = row.get(3)?;
    let channel_id_str: String = row.get(4)?;
    let enabled: i32 = row.get(5)?;
    let created_at_str: String = row.get(6)?;
    Ok(AnalyticsAlert {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        site_id: Ulid::from_string(&site_id_str).unwrap_or_default(),
        kind: AnalyticsAlertKind::from_str(&type_str).unwrap_or(AnalyticsAlertKind::TrafficSpike),
        config: serde_json::from_str(&config_str).unwrap_or_default(),
        channel_id: Ulid::from_string(&channel_id_str).unwrap_or_default(),
        enabled: enabled != 0,
        created_at: parse_utc(&created_at_str),
    })
}

fn row_to_alert_fire(row: &rusqlite::Row<'_>) -> rusqlite::Result<AnalyticsAlertFire> {
    let id_str: String = row.get(0)?;
    let alert_id_str: String = row.get(1)?;
    let fired_at_str: String = row.get(2)?;
    let payload_str: String = row.get(3)?;
    Ok(AnalyticsAlertFire {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        alert_id: Ulid::from_string(&alert_id_str).unwrap_or_default(),
        fired_at: parse_utc(&fired_at_str),
        payload: serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null),
    })
}

fn row_to_share_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<ShareLink> {
    let id_str: String = row.get(0)?;
    let site_id_str: String = row.get(1)?;
    let expires_at: Option<String> = row.get(4)?;
    let created_at_str: String = row.get(6)?;
    Ok(ShareLink {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        site_id: Ulid::from_string(&site_id_str).unwrap_or_default(),
        token: row.get(2)?,
        label: row.get(3)?,
        expires_at: parse_utc_opt(expires_at),
        created_by: row.get(5)?,
        created_at: parse_utc(&created_at_str),
    })
}

fn row_to_digest_sub(row: &rusqlite::Row<'_>) -> rusqlite::Result<DigestSubscription> {
    let id_str: String = row.get(0)?;
    let user_id_str: String = row.get(1)?;
    let site_id_str: String = row.get(2)?;
    let freq_str: String = row.get(3)?;
    let enabled: i32 = row.get(4)?;
    let bounce_count: i64 = row.get(5)?;
    let created_at_str: String = row.get(6)?;
    Ok(DigestSubscription {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        user_id: Ulid::from_string(&user_id_str).unwrap_or_default(),
        site_id: Ulid::from_string(&site_id_str).unwrap_or_default(),
        frequency: DigestFrequency::from_str(&freq_str).unwrap_or(DigestFrequency::Weekly),
        enabled: enabled != 0,
        bounce_count: bounce_count.max(0) as u32,
        created_at: parse_utc(&created_at_str),
    })
}

/// Macro to run a sync closure on the blocking thread pool with a cloned Arc<Mutex<Connection>>.
macro_rules! db {
    ($conn:expr, $body:expr) => {{
        let conn = $conn.clone();
        task::spawn_blocking(move || {
            let guard = conn.lock().map_err(|e| StoreError::db(e.to_string()))?;
            $body(&*guard)
        })
        .await
        .map_err(|e| StoreError::db(e.to_string()))?
    }};
}

#[async_trait::async_trait]
impl MetaStore for SqliteMeta {
    async fn create_site(&self, site: &Site) -> Result<(), StoreError> {
        let site = site.clone();
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "INSERT INTO sites (id, org_id, domain, name, timezone, public_key, created_at, is_active)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    site.id.to_string(),
                    site.org_id.to_string(),
                    site.domain,
                    site.name,
                    site.timezone,
                    site.public_key,
                    site.created_at.to_rfc3339(),
                    site.is_active as i32,
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn update_site(&self, site: &Site) -> Result<(), StoreError> {
        let site = site.clone();
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "UPDATE sites
                 SET org_id = ?2, domain = ?3, name = ?4, timezone = ?5, public_key = ?6, created_at = ?7, is_active = ?8
                 WHERE id = ?1",
                params![
                    site.id.to_string(),
                    site.org_id.to_string(),
                    site.domain,
                    site.name,
                    site.timezone,
                    site.public_key,
                    site.created_at.to_rfc3339(),
                    site.is_active as i32,
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn get_site(&self, id: Ulid) -> Result<Option<Site>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, org_id, domain, name, timezone, public_key, created_at, is_active
                 FROM sites WHERE id = ?1",
                params![id.to_string()],
                row_to_site,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    async fn get_site_by_key(&self, public_key: &str) -> Result<Option<Site>, StoreError> {
        let key = public_key.to_string();
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, org_id, domain, name, timezone, public_key, created_at, is_active
                 FROM sites WHERE public_key = ?1 AND is_active = 1",
                params![key],
                row_to_site,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    async fn get_site_by_domain(&self, domain: &str) -> Result<Option<Site>, StoreError> {
        let domain = domain.to_string();
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, org_id, domain, name, timezone, public_key, created_at, is_active
                 FROM sites WHERE domain = ?1 AND is_active = 1 LIMIT 1",
                params![domain],
                row_to_site,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    async fn list_sites(&self, org_id: Ulid) -> Result<Vec<Site>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, org_id, domain, name, timezone, public_key, created_at, is_active
                     FROM sites WHERE org_id = ?1 ORDER BY created_at ASC",
                )
                .map_err(StoreError::db)?;
            let rows = stmt
                .query_map(params![org_id.to_string()], row_to_site)
                .map_err(StoreError::db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::db)
        })
    }

    async fn delete_site(&self, id: Ulid) -> Result<(), StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.execute("DELETE FROM sites WHERE id = ?1", params![id.to_string()])
                .map(|_| ())
                .map_err(StoreError::db)
        })
    }

    async fn create_org(&self, org: &Organization) -> Result<(), StoreError> {
        let org = org.clone();
        db!(self.conn, |conn: &Connection| {
            let plan = "self_hosted";
            conn.execute(
                "INSERT INTO orgs (id, name, slug, plan, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    org.id.to_string(),
                    org.name,
                    org.slug,
                    plan,
                    org.created_at.to_rfc3339(),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn get_org(&self, id: Ulid) -> Result<Option<Organization>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, name, slug, plan, created_at FROM orgs WHERE id = ?1",
                params![id.to_string()],
                row_to_org,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    async fn list_orgs(&self) -> Result<Vec<Organization>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, slug, plan, created_at FROM orgs ORDER BY created_at ASC",
                )
                .map_err(StoreError::db)?;
            let rows = stmt.query_map([], row_to_org).map_err(StoreError::db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::db)
        })
    }

    async fn create_user(&self, user: &User) -> Result<(), StoreError> {
        let user = user.clone();
        db!(self.conn, |conn: &Connection| {
            let role = match user.role {
                UserRole::Owner => "owner",
                UserRole::Admin => "admin",
                UserRole::Viewer => "viewer",
            };
            conn.execute(
                "INSERT INTO users (id, org_id, email, password_hash, role, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    user.id.to_string(),
                    user.org_id.to_string(),
                    user.email,
                    user.password_hash,
                    role,
                    user.created_at.to_rfc3339(),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, StoreError> {
        let email = email.to_string();
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, org_id, email, password_hash, role, created_at FROM users WHERE email = ?1",
                params![email],
                row_to_user,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    async fn get_user(&self, id: Ulid) -> Result<Option<User>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, org_id, email, password_hash, role, created_at FROM users WHERE id = ?1",
                params![id.to_string()],
                row_to_user,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    async fn create_funnel(&self, funnel: &Funnel) -> Result<(), StoreError> {
        let funnel = funnel.clone();
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "INSERT INTO funnels (id, site_id, name, definition, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    funnel.id.to_string(),
                    funnel.site_id.to_string(),
                    funnel.name,
                    funnel.definition,
                    funnel.created_at.to_rfc3339(),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn get_funnel(&self, id: Ulid) -> Result<Option<Funnel>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, site_id, name, definition, created_at FROM funnels WHERE id = ?1",
                params![id.to_string()],
                row_to_funnel,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    async fn list_funnels(&self, site_id: Ulid) -> Result<Vec<Funnel>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, site_id, name, definition, created_at
                     FROM funnels WHERE site_id = ?1 ORDER BY created_at ASC",
                )
                .map_err(StoreError::db)?;
            let rows = stmt
                .query_map(params![site_id.to_string()], row_to_funnel)
                .map_err(StoreError::db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::db)
        })
    }

    async fn delete_funnel(&self, id: Ulid) -> Result<(), StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.execute("DELETE FROM funnels WHERE id = ?1", params![id.to_string()])
                .map(|_| ())
                .map_err(StoreError::db)
        })
    }

    // ---- Analytics alerts ----
    async fn create_analytics_alert(&self, alert: &AnalyticsAlert) -> Result<(), StoreError> {
        let alert = alert.clone();
        db!(self.conn, |conn: &Connection| {
            let config = serde_json::to_string(&alert.config)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;
            conn.execute(
                "INSERT INTO analytics_alerts (id, site_id, type, config, channel_id, enabled, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    alert.id.to_string(),
                    alert.site_id.to_string(),
                    alert.kind.as_str(),
                    config,
                    alert.channel_id.to_string(),
                    alert.enabled as i32,
                    alert.created_at.to_rfc3339(),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn get_analytics_alert(&self, id: Ulid) -> Result<Option<AnalyticsAlert>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, site_id, type, config, channel_id, enabled, created_at
                 FROM analytics_alerts WHERE id = ?1",
                params![id.to_string()],
                row_to_analytics_alert,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    async fn list_analytics_alerts(
        &self,
        site_id: Ulid,
    ) -> Result<Vec<AnalyticsAlert>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, site_id, type, config, channel_id, enabled, created_at
                     FROM analytics_alerts WHERE site_id = ?1 ORDER BY created_at DESC",
                )
                .map_err(StoreError::db)?;
            let rows = stmt
                .query_map(params![site_id.to_string()], row_to_analytics_alert)
                .map_err(StoreError::db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::db)
        })
    }

    async fn list_enabled_analytics_alerts(&self) -> Result<Vec<AnalyticsAlert>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, site_id, type, config, channel_id, enabled, created_at
                     FROM analytics_alerts WHERE enabled = 1 ORDER BY created_at ASC",
                )
                .map_err(StoreError::db)?;
            let rows = stmt
                .query_map([], row_to_analytics_alert)
                .map_err(StoreError::db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::db)
        })
    }

    async fn set_analytics_alert_enabled(&self, id: Ulid, enabled: bool) -> Result<(), StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "UPDATE analytics_alerts SET enabled = ?1 WHERE id = ?2",
                params![enabled as i32, id.to_string()],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn delete_analytics_alert(&self, id: Ulid) -> Result<(), StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "DELETE FROM analytics_alerts WHERE id = ?1",
                params![id.to_string()],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn record_analytics_alert_fire(
        &self,
        fire: &AnalyticsAlertFire,
    ) -> Result<(), StoreError> {
        let fire = fire.clone();
        db!(self.conn, |conn: &Connection| {
            let payload = serde_json::to_string(&fire.payload)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;
            conn.execute(
                "INSERT INTO analytics_alert_fires (id, alert_id, fired_at, payload)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    fire.id.to_string(),
                    fire.alert_id.to_string(),
                    fire.fired_at.to_rfc3339(),
                    payload,
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn last_analytics_alert_fire(
        &self,
        alert_id: Ulid,
    ) -> Result<Option<AnalyticsAlertFire>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, alert_id, fired_at, payload FROM analytics_alert_fires
                 WHERE alert_id = ?1 ORDER BY fired_at DESC LIMIT 1",
                params![alert_id.to_string()],
                row_to_alert_fire,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    // ---- Share links ----
    async fn create_share_link(&self, link: &ShareLink) -> Result<(), StoreError> {
        let link = link.clone();
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "INSERT INTO share_links (id, site_id, token, label, expires_at, created_by, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    link.id.to_string(),
                    link.site_id.to_string(),
                    link.token,
                    link.label,
                    link.expires_at.map(|d| d.to_rfc3339()),
                    link.created_by,
                    link.created_at.to_rfc3339(),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn list_share_links(&self, site_id: Ulid) -> Result<Vec<ShareLink>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, site_id, token, label, expires_at, created_by, created_at
                     FROM share_links WHERE site_id = ?1 ORDER BY created_at DESC",
                )
                .map_err(StoreError::db)?;
            let rows = stmt
                .query_map(params![site_id.to_string()], row_to_share_link)
                .map_err(StoreError::db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::db)
        })
    }

    async fn get_share_link(&self, id: Ulid) -> Result<Option<ShareLink>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, site_id, token, label, expires_at, created_by, created_at
                 FROM share_links WHERE id = ?1",
                params![id.to_string()],
                row_to_share_link,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    async fn get_share_link_by_token(&self, token: &str) -> Result<Option<ShareLink>, StoreError> {
        let token = token.to_string();
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, site_id, token, label, expires_at, created_by, created_at
                 FROM share_links WHERE token = ?1",
                params![token],
                row_to_share_link,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    async fn update_share_link(
        &self,
        id: Ulid,
        label: Option<String>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "UPDATE share_links SET label = ?2, expires_at = ?3 WHERE id = ?1",
                params![id.to_string(), label, expires_at.map(|d| d.to_rfc3339()),],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn delete_share_link(&self, id: Ulid) -> Result<(), StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "DELETE FROM share_links WHERE id = ?1",
                params![id.to_string()],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    // ---- Email digest subscriptions ----
    async fn upsert_digest_subscription(&self, sub: &DigestSubscription) -> Result<(), StoreError> {
        let sub = sub.clone();
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "INSERT INTO digest_subscriptions
                    (id, user_id, site_id, frequency, enabled, bounce_count, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(user_id, site_id) DO UPDATE SET
                    frequency = excluded.frequency,
                    enabled = excluded.enabled,
                    bounce_count = excluded.bounce_count",
                params![
                    sub.id.to_string(),
                    sub.user_id.to_string(),
                    sub.site_id.to_string(),
                    sub.frequency.as_str(),
                    sub.enabled as i32,
                    sub.bounce_count as i64,
                    sub.created_at.to_rfc3339(),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn get_digest_subscription(
        &self,
        user_id: Ulid,
        site_id: Ulid,
    ) -> Result<Option<DigestSubscription>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, user_id, site_id, frequency, enabled, bounce_count, created_at
                 FROM digest_subscriptions WHERE user_id = ?1 AND site_id = ?2",
                params![user_id.to_string(), site_id.to_string()],
                row_to_digest_sub,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    async fn delete_digest_subscription(
        &self,
        user_id: Ulid,
        site_id: Ulid,
    ) -> Result<(), StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "DELETE FROM digest_subscriptions WHERE user_id = ?1 AND site_id = ?2",
                params![user_id.to_string(), site_id.to_string()],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn list_enabled_digest_subscriptions(
        &self,
    ) -> Result<Vec<DigestSubscription>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, user_id, site_id, frequency, enabled, bounce_count, created_at
                     FROM digest_subscriptions WHERE enabled = 1 ORDER BY created_at ASC",
                )
                .map_err(StoreError::db)?;
            let rows = stmt
                .query_map([], row_to_digest_sub)
                .map_err(StoreError::db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::db)
        })
    }

    async fn record_digest_bounce(&self, id: Ulid, disable_at: u32) -> Result<(), StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "UPDATE digest_subscriptions
                 SET bounce_count = bounce_count + 1,
                     enabled = CASE WHEN bounce_count + 1 >= ?2 THEN 0 ELSE enabled END
                 WHERE id = ?1",
                params![id.to_string(), disable_at as i64],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    // ---- API keys ----
    async fn create_api_key(&self, key: &ApiKey) -> Result<(), StoreError> {
        let key = key.clone();
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "INSERT INTO api_keys
                    (id, org_id, site_id, name, scope, key_hash, display_prefix, created_at, last_used_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    key.id.to_string(),
                    key.org_id.to_string(),
                    key.site_id.map(|s| s.to_string()),
                    key.name,
                    key.scope.as_str(),
                    key.key_hash,
                    key.display_prefix,
                    key.created_at.to_rfc3339(),
                    key.last_used_at.map(|d| d.to_rfc3339()),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn list_api_keys(&self, org_id: Ulid) -> Result<Vec<ApiKey>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, org_id, site_id, name, scope, key_hash, display_prefix, created_at, last_used_at
                     FROM api_keys WHERE org_id = ?1 ORDER BY created_at DESC",
                )
                .map_err(StoreError::db)?;
            let rows = stmt
                .query_map(params![org_id.to_string()], row_to_api_key)
                .map_err(StoreError::db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::db)
        })
    }

    async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, StoreError> {
        let key_hash = key_hash.to_string();
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, org_id, site_id, name, scope, key_hash, display_prefix, created_at, last_used_at
                 FROM api_keys WHERE key_hash = ?1",
                params![key_hash],
                row_to_api_key,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    async fn touch_api_key(&self, id: Ulid) -> Result<(), StoreError> {
        let now = Utc::now().to_rfc3339();
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "UPDATE api_keys SET last_used_at = ?1 WHERE id = ?2",
                params![now, id.to_string()],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn delete_api_key(&self, id: Ulid) -> Result<(), StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "DELETE FROM api_keys WHERE id = ?1",
                params![id.to_string()],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    // ---- Alert channels ----
    async fn create_alert_channel(&self, channel: &AlertChannel) -> Result<(), StoreError> {
        let channel = channel.clone();
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "INSERT INTO alert_channels (id, site_id, kind, url, secret, created_at, last_error_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    channel.id.to_string(),
                    channel.site_id.to_string(),
                    channel.kind.as_str(),
                    channel.url,
                    channel.secret,
                    channel.created_at.to_rfc3339(),
                    channel.last_error_at.map(|d| d.to_rfc3339()),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn list_alert_channels(&self, site_id: Ulid) -> Result<Vec<AlertChannel>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, site_id, kind, url, secret, created_at, last_error_at
                     FROM alert_channels WHERE site_id = ?1 ORDER BY created_at DESC",
                )
                .map_err(StoreError::db)?;
            let rows = stmt
                .query_map(params![site_id.to_string()], row_to_alert_channel)
                .map_err(StoreError::db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::db)
        })
    }

    async fn delete_alert_channel(&self, id: Ulid) -> Result<(), StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "DELETE FROM alert_channels WHERE id = ?1",
                params![id.to_string()],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }
}
