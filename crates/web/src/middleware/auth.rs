use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    Json,
};
use axum_extra::extract::cookie::CookieJar;
use std::sync::Arc;
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
    /// API keys, or wiring alert-channel destinations).
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

/// Returns a signed cookie value: `{user_id}.{hex_mac_16}`.
pub fn sign_session(secret: &str, user_id: &str) -> String {
    let key = session_key(secret);
    let mac = blake3::keyed_hash(&key, user_id.as_bytes());
    format!("{}.{}", user_id, hex::encode(&mac.as_bytes()[..16]))
}

/// Validates a signed cookie value; returns the user_id on success.
pub fn verify_session(secret: &str, value: &str) -> Option<String> {
    let (user_id, provided_sig) = value.split_once('.')?;
    let key = session_key(secret);
    let expected_sig = hex::encode(&blake3::keyed_hash(&key, user_id.as_bytes()).as_bytes()[..16]);
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
