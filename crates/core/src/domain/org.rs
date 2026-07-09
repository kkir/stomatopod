use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: Ulid,
    pub name: String,
    /// URL-safe slug, unique across all orgs.
    pub slug: String,
    pub plan: Plan,
    pub created_at: DateTime<Utc>,
}

/// Deployment plan. Self-hosted is the only supported plan; the field is
/// retained for schema compatibility with existing metadata databases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Plan {
    #[default]
    SelfHosted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Ulid,
    pub org_id: Ulid,
    pub email: String,
    pub password_hash: String,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    #[default]
    Owner,
    Admin,
    Viewer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Funnel {
    pub id: Ulid,
    pub site_id: Ulid,
    pub name: String,
    /// JSON-serialized Vec<FunnelStep>.
    pub definition: String,
    pub created_at: DateTime<Utc>,
}
