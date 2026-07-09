use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    Json,
};
use axum_extra::extract::cookie::CookieJar;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use ulid::Ulid;

use stomatopod_core::domain::api_key::{ApiKey, ApiKeyScope};

use crate::{routes::api::resolve_api_key, state::AppState};

pub const SESSION_COOKIE: &str = "sp_session";

/// Who authenticated a request, propagated to handlers via a request
/// extension so they can enforce per-key authorization.
#[derive(Debug, Clone)]
pub enum Principal {
    /// Signed-session bearer token (a logged-in user identity).
    User(String),
    /// A read-scoped API key, restricted to its org and (optionally) site.
    ApiKey {
        org_id: Ulid,
        /// `None` = org-wide read access; `Some` = a single site.
        site_id: Option<Ulid>,
    },
    /// Authenticated via the dashboard session cookie (same-origin).
    Session,
}

impl Principal {
    /// True for a logged-in dashboard user (session cookie or signed-session
    /// bearer token). False for read/ingest-scoped API keys, which must not
    /// perform account-administrative writes (creating sites, minting other
    /// API keys, wiring alert destinations, share links, or analytics alerts).
    pub fn is_dashboard(&self) -> bool {
        !matches!(self, Principal::ApiKey { .. })
    }
}

fn session_key(secret: &str) -> [u8; 32] {
    blake3::derive_key("stomatopod session signing key v1", secret.as_bytes())
}

/// Constant-time byte-slice equality. Used to compare MACs/signatures so an
/// attacker can't recover a valid signature byte-by-byte from response timing.
/// The length check leaks length only, which is fixed for our signatures.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Returns a signed cookie/bearer value: `{user_id}.{exp_unix}.{hex_mac_16}`.
/// The MAC covers `user_id.exp` so expiry cannot be stripped or extended.
pub fn sign_session(secret: &str, user_id: &str, ttl_secs: u64) -> String {
    let exp = unix_now().saturating_add(ttl_secs);
    let payload = format!("{user_id}.{exp}");
    let key = session_key(secret);
    let mac = blake3::keyed_hash(&key, payload.as_bytes());
    format!("{payload}.{}", hex::encode(&mac.as_bytes()[..16]))
}

/// Validates a signed session value; returns the user_id on success.
/// Rejects tampered, malformed, or expired tokens (Bearer and cookie share
/// this path so Max-Age alone is not load-bearing).
pub fn verify_session(secret: &str, value: &str) -> Option<String> {
    // Format: user_id.exp.sig — split from the right so user_id may contain
    // dots in future without breaking parsing (today it is a ULID).
    let (payload, provided_sig) = value.rsplit_once('.')?;
    let (user_id, exp_str) = payload.rsplit_once('.')?;
    if user_id.is_empty() || provided_sig.is_empty() {
        return None;
    }
    let exp: u64 = exp_str.parse().ok()?;
    if unix_now() > exp {
        return None;
    }
    let key = session_key(secret);
    let expected_sig = hex::encode(&blake3::keyed_hash(&key, payload.as_bytes()).as_bytes()[..16]);
    if constant_time_eq(provided_sig.as_bytes(), expected_sig.as_bytes()) {
        Some(user_id.to_string())
    } else {
        None
    }
}

fn extract_session_cookie(secret: &str, req: &Request) -> bool {
    CookieJar::from_headers(req.headers())
        .get(SESSION_COOKIE)
        .and_then(|c| verify_session(secret, c.value()))
        .is_some()
}

fn bearer_token(req: &Request) -> Option<&str> {
    req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
}

/// Dashboard middleware: redirects to /login if unauthenticated.
pub async fn require_auth(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    if !extract_session_cookie(&state.config.auth.secret_key, &req) {
        return Redirect::to("/login").into_response();
    }
    next.run(req).await
}

/// API middleware: returns JSON 401 if unauthenticated.
///
/// Accepts, in order: a signed-session bearer token, a read-scoped API key
/// (`rk_...`), or the dashboard session cookie. The resolved [`Principal`]
/// is stored as a request extension so handlers can enforce org/site scope.
pub async fn require_api_auth(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let secret = &state.config.auth.secret_key;

    // 1. Signed-session bearer (sync, cheap) — try first.
    if let Some(uid) = bearer_token(&req).and_then(|t| verify_session(secret, t)) {
        req.extensions_mut().insert(Principal::User(uid));
        return next.run(req).await;
    }

    // 2. Read-scoped API key. The `rk_` prefix is a cheap discriminator so
    //    non-key bearers never incur a store lookup.
    if let Some(token) = bearer_token(&req) {
        if token.starts_with("rk_") {
            let hash = ApiKey::hash(token);
            match resolve_api_key(&state, &hash).await {
                Ok(Some(entry)) if entry.scope == ApiKeyScope::Read => {
                    crate::routes::api::touch_api_key(&state, entry.key_id);
                    req.extensions_mut().insert(Principal::ApiKey {
                        org_id: entry.org_id,
                        site_id: entry.site_id,
                    });
                    return next.run(req).await;
                }
                _ => {}
            }
        }
    }

    // 3. Dashboard session cookie (same-origin fetch).
    if extract_session_cookie(secret, &req) {
        req.extensions_mut().insert(Principal::Session);
        return next.run(req).await;
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "authentication required"})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_round_trip() {
        let token = sign_session("sekrit", "01ARZ3NDEKTSV4RRFFQ69G5FAV", 3600);
        assert_eq!(
            verify_session("sekrit", &token).as_deref(),
            Some("01ARZ3NDEKTSV4RRFFQ69G5FAV")
        );
    }

    #[test]
    fn session_rejects_wrong_secret() {
        let token = sign_session("sekrit", "user1", 3600);
        assert!(verify_session("other", &token).is_none());
    }

    #[test]
    fn session_rejects_tamper() {
        let token = sign_session("sekrit", "user1", 3600);
        let tampered = token.replacen("user1", "user2", 1);
        assert!(verify_session("sekrit", &tampered).is_none());
    }

    #[test]
    fn session_rejects_expired() {
        // Build an already-expired payload (exp 10s in the past).
        let exp = unix_now().saturating_sub(10);
        let payload = format!("user1.{exp}");
        let key = session_key("sekrit");
        let mac = blake3::keyed_hash(&key, payload.as_bytes());
        let token = format!("{payload}.{}", hex::encode(&mac.as_bytes()[..16]));
        assert!(verify_session("sekrit", &token).is_none());
    }

    #[test]
    fn session_rejects_legacy_no_exp_format() {
        // Old format was user_id.sig without embedded expiry.
        let key = session_key("sekrit");
        let mac = blake3::keyed_hash(&key, b"user1");
        let legacy = format!("user1.{}", hex::encode(&mac.as_bytes()[..16]));
        assert!(verify_session("sekrit", &legacy).is_none());
    }
}
