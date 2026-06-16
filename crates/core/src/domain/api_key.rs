use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// What an API key is allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyScope {
    /// Read-only analytics queries (used by the CLI / LLM agents).
    Read,
    /// Server-side custom event ingestion.
    Ingest,
}

impl ApiKeyScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApiKeyScope::Read => "read",
            ApiKeyScope::Ingest => "ingest",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(ApiKeyScope::Read),
            "ingest" => Some(ApiKeyScope::Ingest),
            _ => None,
        }
    }

    /// Token prefix used to make keys self-identifying and to cheaply
    /// discriminate read keys from signed-session bearers on the hot path.
    fn prefix(&self) -> &'static str {
        match self {
            ApiKeyScope::Read => "rk",
            ApiKeyScope::Ingest => "sk_live",
        }
    }
}

/// A long-lived, revocable credential. Stored as a BLAKE3 hash; the plaintext
/// is shown to the user exactly once at creation time. Org-scoped, with an
/// optional site binding (`None` = org-wide; required `Some` for ingest keys).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: Ulid,
    pub org_id: Ulid,
    pub site_id: Option<Ulid>,
    pub name: String,
    pub scope: ApiKeyScope,
    /// Hex-encoded BLAKE3 hash of the plaintext (64 chars).
    pub key_hash: String,
    /// Non-secret prefix shown in listings, e.g. `rk_a1b2`.
    pub display_prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl ApiKey {
    /// Canonical hashing for API keys. Must match the lookup path in the
    /// auth middleware. Full 64-char hex (no truncation).
    pub fn hash(plaintext: &str) -> String {
        blake3::hash(plaintext.as_bytes()).to_hex().to_string()
    }

    /// Generate a fresh secret for the given scope. Returns
    /// `(plaintext, display_prefix, key_hash)`. The plaintext is never stored.
    pub fn generate(scope: ApiKeyScope) -> (String, String, String) {
        let mut h = blake3::Hasher::new();
        h.update(&Ulid::new().to_bytes());
        h.update(&Ulid::new().to_bytes());
        h.update(
            &std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                .to_le_bytes(),
        );
        // 128 bits of body entropy, hex-encoded → 32 chars.
        let body = hex::encode(&h.finalize().as_bytes()[..16]);
        let prefix = scope.prefix();
        let plaintext = format!("{prefix}_{body}");
        let display_prefix = format!("{prefix}_{}", &body[..4]);
        let key_hash = Self::hash(&plaintext);
        (plaintext, display_prefix, key_hash)
    }

    fn new(scope: ApiKeyScope, org_id: Ulid, site_id: Option<Ulid>, name: String) -> (Self, String) {
        let (plaintext, display_prefix, key_hash) = Self::generate(scope);
        let key = ApiKey {
            id: Ulid::new(),
            org_id,
            site_id,
            name,
            scope,
            key_hash,
            display_prefix,
            created_at: Utc::now(),
            last_used_at: None,
        };
        (key, plaintext)
    }

    /// Build a read-only key. `site_id` `None` grants org-wide read access.
    pub fn new_read(org_id: Ulid, site_id: Option<Ulid>, name: String) -> (Self, String) {
        Self::new(ApiKeyScope::Read, org_id, site_id, name)
    }

    /// Build an ingest key bound to a single site.
    pub fn new_ingest(org_id: Ulid, site_id: Ulid, name: String) -> (Self, String) {
        Self::new(ApiKeyScope::Ingest, org_id, Some(site_id), name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_generated() {
        let (plaintext, _prefix, key_hash) = ApiKey::generate(ApiKeyScope::Read);
        assert_eq!(ApiKey::hash(&plaintext), key_hash);
        assert_eq!(key_hash.len(), 64);
    }

    #[test]
    fn prefixes_are_scope_specific() {
        let (read_pt, read_disp, _) = ApiKey::generate(ApiKeyScope::Read);
        assert!(read_pt.starts_with("rk_"));
        assert!(read_pt.starts_with(&read_disp));

        let (ingest_pt, ingest_disp, _) = ApiKey::generate(ApiKeyScope::Ingest);
        assert!(ingest_pt.starts_with("sk_live_"));
        assert!(ingest_pt.starts_with(&ingest_disp));
    }

    #[test]
    fn scope_str_round_trip() {
        for scope in [ApiKeyScope::Read, ApiKeyScope::Ingest] {
            assert_eq!(ApiKeyScope::parse(scope.as_str()), Some(scope));
        }
        assert_eq!(ApiKeyScope::parse("bogus"), None);
    }

    #[test]
    fn constructors_set_site_binding() {
        let org = Ulid::new();
        let site = Ulid::new();
        let (read_key, _) = ApiKey::new_read(org, None, "agent".into());
        assert_eq!(read_key.site_id, None);
        assert_eq!(read_key.scope, ApiKeyScope::Read);

        let (ingest_key, _) = ApiKey::new_ingest(org, site, "backend".into());
        assert_eq!(ingest_key.site_id, Some(site));
        assert_eq!(ingest_key.scope, ApiKeyScope::Ingest);
    }
}
