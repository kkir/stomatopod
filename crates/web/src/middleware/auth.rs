use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    Json,
};
use std::sync::Arc;

use crate::state::AppState;

pub const SESSION_COOKIE: &str = "sp_session";

fn session_key(secret: &str) -> [u8; 32] {
    blake3::derive_key("stomatopod session signing key v1", secret.as_bytes())
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
    if provided_sig == expected_sig {
        Some(user_id.to_string())
    } else {
        None
    }
}

fn extract_session_cookie(secret: &str, req: &Request) -> bool {
    req.headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|header| {
            header.split(';').find_map(|part| {
                let part = part.trim();
                let (k, v) = part.split_once('=')?;
                (k.trim() == SESSION_COOKIE).then(|| v.trim().to_string())
            })
        })
        .and_then(|val| verify_session(secret, &val))
        .is_some()
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
/// Accepts both session cookies and `Authorization: Bearer <signed_session>` tokens.
pub async fn require_api_auth(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let secret = &state.config.auth.secret_key;

    let has_bearer = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .and_then(|token| verify_session(secret, token))
        .is_some();

    if !has_bearer && !extract_session_cookie(secret, &req) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "authentication required"})),
        )
            .into_response();
    }

    next.run(req).await
}
