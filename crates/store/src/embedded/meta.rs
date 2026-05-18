use std::{path::Path, sync::Arc};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Mutex;
use tokio::task;
use ulid::Ulid;

use stomatopod_core::{
    domain::{
        org::{Funnel, Organization, Plan, User, UserRole},
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
}
