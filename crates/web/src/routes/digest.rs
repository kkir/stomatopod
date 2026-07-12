//! Digest subscription management.
//!
//! CRUD lives under `/api/v1/sites/:site/digest-subscription` and operates
//! on the current user's subscription. Digests are delivered through the
//! site's configured notification channels (Slack, Telegram, webhook).

use std::sync::Arc;

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use ulid::Ulid;

use stomatopod_core::domain::digest::{DigestFrequency, DigestSubscription};

use crate::{
    digest::build_digest_message,
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
        Ok(Some(sub)) => Json(serde_json::json!({
            "subscription": subscription_json(&sub),
        }))
        .into_response(),
        Ok(None) => Json(serde_json::json!({ "subscription": null })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// PUT /api/v1/sites/:site/digest-subscription  - create or update
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

/// DELETE /api/v1/sites/:site/digest-subscription  - unsubscribe
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

/// POST /api/v1/sites/:site/digest-subscription/test  - send a digest now
/// through the site's notification channels.
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
    let channels = match state.meta.list_alert_channels(site_id).await {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if channels.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "configure a notification destination (Slack, Telegram, or webhook) first"
            })),
        )
            .into_response();
    }
    let msg = build_digest_message(
        &state.backend,
        &state.meta,
        state.config.public_base_url(),
        site_id,
        cadence,
    )
    .await;
    match msg {
        Some(msg) => match state.digest_notifier.send(&channels, msg).await {
            Ok(_) => Json(serde_json::json!({"status": "sent"})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response(),
        },
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "could not build digest (missing site)"})),
        )
            .into_response(),
    }
}
