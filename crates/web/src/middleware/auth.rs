use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use std::sync::Arc;

use crate::state::AppState;

/// Session cookie name.
pub const SESSION_COOKIE: &str = "sp_session";

/// Extractor that checks for a valid session cookie.
/// Redirects to /login if missing.
pub async fn require_auth(
    State(_state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    // Check for session cookie
    let has_session = req
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .map(|c| c.contains(SESSION_COOKIE))
        .unwrap_or(false);

    if !has_session {
        return Redirect::to("/login").into_response();
    }

    next.run(req).await
}
