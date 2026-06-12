use std::{path::Path, sync::Arc};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Mutex;
use tokio::task;
use ulid::Ulid;

use stomatopod_core::{
    domain::{
        agent::{Agent, AlertChannel, AlertChannelKind, SentinelToken},
        incident::{Incident, IncidentStatus, IncidentTrigger},
        org::{Funnel, Organization, Plan, User, UserRole},
        policy::Policy,
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
                 PRAGMA cache_size=-32000;
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

        CREATE TABLE IF NOT EXISTS policies (
            id                TEXT PRIMARY KEY,
            site_id           TEXT NOT NULL REFERENCES sites(id),
            repetition_max    INTEGER,
            velocity_max_tps  REAL,
            cost_cap_usd      REAL,
            hint_template     TEXT,
            created_at        TEXT NOT NULL,
            UNIQUE(site_id)
        );
        CREATE INDEX IF NOT EXISTS idx_policies_site_id ON policies(site_id);

        CREATE TABLE IF NOT EXISTS agents (
            id            TEXT PRIMARY KEY,
            site_id       TEXT NOT NULL REFERENCES sites(id),
            agent_id      TEXT NOT NULL,
            name          TEXT NOT NULL,
            policy_id     TEXT REFERENCES policies(id),
            created_at    TEXT NOT NULL,
            last_seen_at  TEXT NOT NULL,
            UNIQUE(site_id, agent_id)
        );
        CREATE INDEX IF NOT EXISTS idx_agents_site_id ON agents(site_id);

        CREATE TABLE IF NOT EXISTS sentinel_tokens (
            id            TEXT PRIMARY KEY,
            site_id       TEXT NOT NULL REFERENCES sites(id),
            name          TEXT NOT NULL,
            token_hash    TEXT UNIQUE NOT NULL,
            created_at    TEXT NOT NULL,
            last_used_at  TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_sentinel_tokens_hash ON sentinel_tokens(token_hash);
        CREATE INDEX IF NOT EXISTS idx_sentinel_tokens_site ON sentinel_tokens(site_id);

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

        CREATE TABLE IF NOT EXISTS incidents (
            id            TEXT PRIMARY KEY,
            site_id       TEXT NOT NULL REFERENCES sites(id),
            agent_id      TEXT NOT NULL,
            trigger_json  TEXT NOT NULL,
            status        TEXT NOT NULL DEFAULT 'open',
            opened_at     TEXT NOT NULL,
            closed_at     TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_incidents_site_opened ON incidents(site_id, opened_at DESC);

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
    let plan_str: String = row.get(3)?;
    let created_at_str: String = row.get(4)?;
    Ok(Organization {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        name: row.get(1)?,
        slug: row.get(2)?,
        plan: match plan_str.as_str() {
            "free" => Plan::Free,
            "pro" => Plan::Pro,
            "enterprise" => Plan::Enterprise,
            _ => Plan::SelfHosted,
        },
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

fn row_to_agent(row: &rusqlite::Row<'_>) -> rusqlite::Result<Agent> {
    let id_str: String = row.get(0)?;
    let site_id_str: String = row.get(1)?;
    let policy_id_str: Option<String> = row.get(4)?;
    let created_at_str: String = row.get(5)?;
    let last_seen_at_str: String = row.get(6)?;
    Ok(Agent {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        site_id: Ulid::from_string(&site_id_str).unwrap_or_default(),
        agent_id: row.get(2)?,
        name: row.get(3)?,
        policy_id: policy_id_str
            .as_deref()
            .and_then(|s| Ulid::from_string(s).ok()),
        created_at: parse_utc(&created_at_str),
        last_seen_at: parse_utc(&last_seen_at_str),
    })
}

fn row_to_sentinel_token(row: &rusqlite::Row<'_>) -> rusqlite::Result<SentinelToken> {
    let id_str: String = row.get(0)?;
    let site_id_str: String = row.get(1)?;
    let created_at_str: String = row.get(4)?;
    let last_used_at_str: Option<String> = row.get(5)?;
    Ok(SentinelToken {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        site_id: Ulid::from_string(&site_id_str).unwrap_or_default(),
        name: row.get(2)?,
        token_hash: row.get(3)?,
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
        kind: match kind_str.as_str() {
            "slack" => AlertChannelKind::Slack,
            _ => AlertChannelKind::Webhook,
        },
        url: row.get(3)?,
        secret: row.get(4)?,
        created_at: parse_utc(&created_at_str),
        last_error_at: parse_utc_opt(last_error_at_str),
    })
}

fn row_to_policy(row: &rusqlite::Row<'_>) -> rusqlite::Result<Policy> {
    let id_str: String = row.get(0)?;
    let site_id_str: String = row.get(1)?;
    let created_at_str: String = row.get(6)?;
    Ok(Policy {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        site_id: Ulid::from_string(&site_id_str).unwrap_or_default(),
        repetition_max: row.get::<_, Option<i64>>(2)?.map(|n| n as u32),
        velocity_max_tps: row.get(3)?,
        cost_cap_usd: row.get(4)?,
        hint_template: row.get(5)?,
        created_at: parse_utc(&created_at_str),
    })
}

fn row_to_incident(row: &rusqlite::Row<'_>) -> rusqlite::Result<Incident> {
    let id_str: String = row.get(0)?;
    let site_id_str: String = row.get(1)?;
    let trigger_json: String = row.get(3)?;
    let status_str: String = row.get(4)?;
    let opened_at_str: String = row.get(5)?;
    let closed_at_str: Option<String> = row.get(6)?;
    let trigger: IncidentTrigger =
        serde_json::from_str(&trigger_json).unwrap_or(IncidentTrigger::Manual);
    Ok(Incident {
        id: Ulid::from_string(&id_str).unwrap_or_default(),
        site_id: Ulid::from_string(&site_id_str).unwrap_or_default(),
        agent_id: row.get(2)?,
        trigger,
        status: match status_str.as_str() {
            "acknowledged" => IncidentStatus::Acknowledged,
            "resolved" => IncidentStatus::Resolved,
            _ => IncidentStatus::Open,
        },
        opened_at: parse_utc(&opened_at_str),
        closed_at: parse_utc_opt(closed_at_str),
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
            let plan = match org.plan {
                Plan::SelfHosted => "self_hosted",
                Plan::Free => "free",
                Plan::Pro => "pro",
                Plan::Enterprise => "enterprise",
            };
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

    // ---- Agents ----
    async fn upsert_agent(&self, agent: &Agent) -> Result<(), StoreError> {
        let agent = agent.clone();
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "INSERT INTO agents (id, site_id, agent_id, name, policy_id, created_at, last_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(site_id, agent_id) DO UPDATE SET
                    name = excluded.name,
                    policy_id = excluded.policy_id,
                    last_seen_at = excluded.last_seen_at",
                params![
                    agent.id.to_string(),
                    agent.site_id.to_string(),
                    agent.agent_id,
                    agent.name,
                    agent.policy_id.map(|p| p.to_string()),
                    agent.created_at.to_rfc3339(),
                    agent.last_seen_at.to_rfc3339(),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn list_agents(&self, site_id: Ulid) -> Result<Vec<Agent>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, site_id, agent_id, name, policy_id, created_at, last_seen_at
                     FROM agents WHERE site_id = ?1 ORDER BY last_seen_at DESC",
                )
                .map_err(StoreError::db)?;
            let rows = stmt
                .query_map(params![site_id.to_string()], row_to_agent)
                .map_err(StoreError::db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::db)
        })
    }

    async fn get_agent(&self, site_id: Ulid, agent_id: &str) -> Result<Option<Agent>, StoreError> {
        let agent_id = agent_id.to_string();
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, site_id, agent_id, name, policy_id, created_at, last_seen_at
                 FROM agents WHERE site_id = ?1 AND agent_id = ?2",
                params![site_id.to_string(), agent_id],
                row_to_agent,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    // ---- Sentinel tokens ----
    async fn create_sentinel_token(&self, token: &SentinelToken) -> Result<(), StoreError> {
        let token = token.clone();
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "INSERT INTO sentinel_tokens (id, site_id, name, token_hash, created_at, last_used_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    token.id.to_string(),
                    token.site_id.to_string(),
                    token.name,
                    token.token_hash,
                    token.created_at.to_rfc3339(),
                    token.last_used_at.map(|d| d.to_rfc3339()),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn list_sentinel_tokens(&self, site_id: Ulid) -> Result<Vec<SentinelToken>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, site_id, name, token_hash, created_at, last_used_at
                     FROM sentinel_tokens WHERE site_id = ?1 ORDER BY created_at DESC",
                )
                .map_err(StoreError::db)?;
            let rows = stmt
                .query_map(params![site_id.to_string()], row_to_sentinel_token)
                .map_err(StoreError::db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::db)
        })
    }

    async fn get_sentinel_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<SentinelToken>, StoreError> {
        let token_hash = token_hash.to_string();
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, site_id, name, token_hash, created_at, last_used_at
                 FROM sentinel_tokens WHERE token_hash = ?1",
                params![token_hash],
                row_to_sentinel_token,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    async fn touch_sentinel_token(&self, id: Ulid) -> Result<(), StoreError> {
        let now = Utc::now().to_rfc3339();
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "UPDATE sentinel_tokens SET last_used_at = ?1 WHERE id = ?2",
                params![now, id.to_string()],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn delete_sentinel_token(&self, id: Ulid) -> Result<(), StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "DELETE FROM sentinel_tokens WHERE id = ?1",
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

    // ---- Policies ----
    async fn upsert_policy(&self, policy: &Policy) -> Result<(), StoreError> {
        let policy = policy.clone();
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "INSERT INTO policies
                    (id, site_id, repetition_max, velocity_max_tps, cost_cap_usd, hint_template, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(site_id) DO UPDATE SET
                    repetition_max = excluded.repetition_max,
                    velocity_max_tps = excluded.velocity_max_tps,
                    cost_cap_usd = excluded.cost_cap_usd,
                    hint_template = excluded.hint_template",
                params![
                    policy.id.to_string(),
                    policy.site_id.to_string(),
                    policy.repetition_max.map(|n| n as i64),
                    policy.velocity_max_tps,
                    policy.cost_cap_usd,
                    policy.hint_template,
                    policy.created_at.to_rfc3339(),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn get_policy(&self, site_id: Ulid) -> Result<Option<Policy>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            conn.query_row(
                "SELECT id, site_id, repetition_max, velocity_max_tps, cost_cap_usd, hint_template, created_at
                 FROM policies WHERE site_id = ?1",
                params![site_id.to_string()],
                row_to_policy,
            )
            .optional()
            .map_err(StoreError::db)
        })
    }

    // ---- Incidents ----
    async fn record_incident(&self, incident: &Incident) -> Result<(), StoreError> {
        let incident = incident.clone();
        db!(self.conn, |conn: &Connection| {
            let trigger_json = serde_json::to_string(&incident.trigger)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;
            conn.execute(
                "INSERT INTO incidents
                    (id, site_id, agent_id, trigger_json, status, opened_at, closed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    incident.id.to_string(),
                    incident.site_id.to_string(),
                    incident.agent_id,
                    trigger_json,
                    incident.status.as_str(),
                    incident.opened_at.to_rfc3339(),
                    incident.closed_at.map(|d| d.to_rfc3339()),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }

    async fn list_incidents(&self, site_id: Ulid, limit: u32) -> Result<Vec<Incident>, StoreError> {
        db!(self.conn, |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, site_id, agent_id, trigger_json, status, opened_at, closed_at
                     FROM incidents WHERE site_id = ?1
                     ORDER BY opened_at DESC LIMIT ?2",
                )
                .map_err(StoreError::db)?;
            let rows = stmt
                .query_map(params![site_id.to_string(), limit], row_to_incident)
                .map_err(StoreError::db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::db)
        })
    }

    async fn update_incident_status(
        &self,
        id: Ulid,
        status: IncidentStatus,
    ) -> Result<(), StoreError> {
        let closed_at = matches!(status, IncidentStatus::Resolved).then(|| Utc::now().to_rfc3339());
        db!(self.conn, |conn: &Connection| {
            conn.execute(
                "UPDATE incidents SET status = ?1, closed_at = COALESCE(?2, closed_at)
                 WHERE id = ?3",
                params![status.as_str(), closed_at, id.to_string()],
            )
            .map(|_| ())
            .map_err(StoreError::db)
        })
    }
}
