//! Email digest subscription management + one-click unsubscribe.
//!
//! CRUD lives under `/api/v1/sites/:site/digest-subscription` and operates
//! on the current user's subscription. A token-based unsubscribe endpoint
//! (`/digest/unsubscribe/:token`) requires no login.

use std::sync::Arc;

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    Json,
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use ulid::Ulid;

use stomatopod_core::domain::digest::{DigestFrequency, DigestSubscription};

use crate::{
    digest::{build_digest_email, verify_unsubscribe_token},
    middleware::auth::{verify_session, Principal, SESSION_COOKIE},
    state::AppState,
};

async fn resolve_site_id(state: &AppState, site: &str) -> Option<Ulid> {
    if let Ok(id) = Ulid::from_string(site) {
        return Some(id);
    }
    state
        .meta
        .get_site_by_domain(site)
        .await
        .ok()
        .flatten()
        .map(|s| s.id)
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "not found"})),
    )
        .into_response()
}

/// Identify the acting user. A session-token bearer carries its user id
/// directly; the cookie-`Session` principal only records that a valid cookie
/// was present (see `require_api_auth`), so re-derive the id from the cookie
/// itself, mirroring `api::session_user_id`. API keys aren't tied to a user.
/// Returns `None` when no user can be determined.
fn current_user_id(state: &AppState, principal: &Principal, jar: &CookieJar) -> Option<Ulid> {
    let raw = match principal {
        Principal::User(uid) => Some(uid.clone()),
        Principal::Session => jar
            .get(SESSION_COOKIE)
            .and_then(|c| verify_session(&state.config.auth.secret_key, c.value())),
        Principal::ApiKey { .. } => None,
    }?;
    Ulid::from_string(&raw).ok()
}

#[derive(Deserialize)]
pub struct PutSubscriptionBody {
    pub frequency: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

fn subscription_json(sub: &DigestSubscription) -> serde_json::Value {
    serde_json::json!({
        "id": sub.id.to_string(),
        "site_id": sub.site_id.to_string(),
        "frequency": sub.frequency.as_str(),
        "enabled": sub.enabled,
        "created_at": sub.created_at.to_rfc3339(),
    })
}

/// GET /api/v1/sites/:site/digest-subscription
pub async fn get_subscription(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    jar: CookieJar,
    Path(site): Path<String>,
) -> Response {
    let site_id = match resolve_site_id(&state, &site).await {
        Some(id) => id,
        None => return not_found(),
    };
    let user_id = match current_user_id(&state, &principal, &jar) {
        Some(id) => id,
        None => return not_found(),
    };
    match state.meta.get_digest_subscription(user_id, site_id).await {
        Ok(Some(sub)) => Json(subscription_json(&sub)).into_response(),
        Ok(None) => Json(serde_json::json!({ "subscription": null })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// PUT /api/v1/sites/:site/digest-subscription  — create or update
pub async fn put_subscription(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    jar: CookieJar,
    Path(site): Path<String>,
    Json(body): Json<PutSubscriptionBody>,
) -> Response {
    let site_id = match resolve_site_id(&state, &site).await {
        Some(id) => id,
        None => return not_found(),
    };
    let user_id = match current_user_id(&state, &principal, &jar) {
        Some(id) => id,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "could not resolve current user"})),
            )
                .into_response()
        }
    };
    let frequency = match DigestFrequency::from_str(&body.frequency) {
        Some(f) => f,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "frequency must be weekly, monthly, or both"})),
            )
                .into_response()
        }
    };
    // Preserve the existing id/created_at on update so the row is stable.
    let existing = state
        .meta
        .get_digest_subscription(user_id, site_id)
        .await
        .ok()
        .flatten();
    let sub = DigestSubscription {
        id: existing.as_ref().map(|s| s.id).unwrap_or_else(Ulid::new),
        user_id,
        site_id,
        frequency,
        enabled: body.enabled,
        bounce_count: 0,
        created_at: existing
            .as_ref()
            .map(|s| s.created_at)
            .unwrap_or_else(chrono::Utc::now),
    };
    match state.meta.upsert_digest_subscription(&sub).await {
        Ok(_) => (StatusCode::OK, Json(subscription_json(&sub))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// DELETE /api/v1/sites/:site/digest-subscription  — unsubscribe
pub async fn delete_subscription(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    jar: CookieJar,
    Path(site): Path<String>,
) -> Response {
    let site_id = match resolve_site_id(&state, &site).await {
        Some(id) => id,
        None => return not_found(),
    };
    let user_id = match current_user_id(&state, &principal, &jar) {
        Some(id) => id,
        None => return not_found(),
    };
    match state
        .meta
        .delete_digest_subscription(user_id, site_id)
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// POST /api/v1/sites/:site/digest-subscription/test  — send a digest now
pub async fn send_test(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    jar: CookieJar,
    Path(site): Path<String>,
) -> Response {
    let site_id = match resolve_site_id(&state, &site).await {
        Some(id) => id,
        None => return not_found(),
    };
    let user_id = match current_user_id(&state, &principal, &jar) {
        Some(id) => id,
        None => return not_found(),
    };
    let sub = match state.meta.get_digest_subscription(user_id, site_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "no subscription to test"})),
            )
                .into_response()
        }
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    // Test send uses the weekly window if subscribed weekly, else monthly.
    let cadence = if sub.frequency.wants_weekly() {
        DigestFrequency::Weekly
    } else {
        DigestFrequency::Monthly
    };
    let email = build_digest_email(
        &state.backend,
        &state.meta,
        state.config.public_base_url(),
        &state.config.auth.secret_key,
        &sub,
        cadence,
    )
    .await;
    match email {
        Some(email) => match state.digest_sender.send(email).await {
            Ok(_) => Json(serde_json::json!({"status": "sent"})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response(),
        },
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "could not build digest (missing user/site)"})),
        )
            .into_response(),
    }
}

/// GET /digest/unsubscribe/:token  — one-click, no auth required.
pub async fn unsubscribe(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Response {
    let sub_id = match verify_unsubscribe_token(&state.config.auth.secret_key, &token) {
        Some(id) => id,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Html(
                    "<!doctype html><meta charset=utf-8><body><h1>Invalid link</h1></body>"
                        .to_string(),
                ),
            )
                .into_response()
        }
    };
    // The token carries the subscription id; disable by setting enabled=false
    // via a fresh upsert. We look it up across enabled subscriptions.
    let subs = state
        .meta
        .list_enabled_digest_subscriptions()
        .await
        .unwrap_or_default();
    if let Some(mut sub) = subs.into_iter().find(|s| s.id == sub_id) {
        sub.enabled = false;
        let _ = state.meta.upsert_digest_subscription(&sub).await;
    }
    Html(
        "<!doctype html><meta charset=utf-8>\
         <body style=\"font-family:system-ui;text-align:center;padding:64px\">\
         <h1>Unsubscribed</h1><p>You will no longer receive these digests.</p>\
         <p style=\"color:#999\">Powered by Stomatopod</p></body>"
            .to_string(),
    )
    .into_response()
}
