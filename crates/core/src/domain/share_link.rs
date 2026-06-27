use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// A token-scoped public, read-only view of a single site's dashboard.
///
/// Anyone holding the `token` can read the site's headline analytics at
/// `/share/{token}` without authenticating. The link is confined to one
/// site, exposes only aggregate reports (never raw events, sessions, keys,
/// or settings), and can carry an optional expiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareLink {
    pub id: Ulid,
    pub site_id: Ulid,
    /// Random URL-safe token embedded in the public URL. Unique.
    pub token: String,
    /// Optional human label, e.g. "Client view".
    pub label: Option<String>,
    /// `None` = never expires.
    pub expires_at: Option<DateTime<Utc>>,
    /// Identifier of the principal that created the link (user id when
    /// known). Stored for the management UI; not a hard FK so self-hosted
    /// session principals without a user row can still create links.
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

impl ShareLink {
    /// True when an expiry is set and now is past it.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|e| now >= e)
    }
}
