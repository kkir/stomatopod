use axum::{
    extract::{ConnectInfo, State},
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use cookie::time::Duration;
use serde::Deserialize;
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration as StdDuration, Instant},
};

use crate::{
    error::AppError,
    middleware::auth::{sign_session, verify_session, SESSION_COOKIE},
    server::html,
    state::AppState,
};

#[derive(Deserialize)]
pub struct LoginForm {
    pub email: String,
    pub password: String,
}

/// Fixed dummy Argon2 PHC string used when the email is unknown so the
/// response timing does not leak whether the account exists. Salt/hash are
/// arbitrary but well-formed; verification always fails.
const DUMMY_PASSWORD_HASH: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$AAAAAAAAAAAAAAAAAAAAAA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// Login attempt throttle: max failures per IP in a sliding window.
const LOGIN_MAX_FAILURES: u32 = 10;
const LOGIN_WINDOW: StdDuration = StdDuration::from_secs(15 * 60);

pub async fn login_page(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    if jar
        .get(SESSION_COOKIE)
        .and_then(|c| verify_session(&state.config.auth.secret_key, c.value()))
        .is_some()
    {
        return Ok(Redirect::to("/").into_response());
    }
    Ok(Html(html::login_page(None)).into_response())
}

pub async fn login_submit(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    jar: CookieJar,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    use argon2::{Argon2, PasswordHash, PasswordVerifier};

    let render_invalid = || -> Result<Response, AppError> {
        Ok(Html(html::login_page(Some("Invalid credentials"))).into_response())
    };
    let render_rate_limited = || -> Result<Response, AppError> {
        Ok(Html(html::login_page(Some(
            "Too many login attempts. Try again later.",
        )))
        .into_response())
    };

    let ip_key = peer.ip().to_string();
    if is_login_rate_limited(&state, &ip_key) {
        return render_rate_limited();
    }

    let user = state.meta.get_user_by_email(&form.email).await?;
    // Always run Argon2 so missing emails take comparable time to wrong passwords.
    let hash_str = user
        .as_ref()
        .map(|u| u.password_hash.as_str())
        .unwrap_or(DUMMY_PASSWORD_HASH);
    let Ok(hash) = PasswordHash::new(hash_str) else {
        record_login_failure(&state, &ip_key);
        return render_invalid();
    };
    let password_ok = Argon2::default()
        .verify_password(form.password.as_bytes(), &hash)
        .is_ok();
    let Some(user) = user.filter(|_| password_ok) else {
        record_login_failure(&state, &ip_key);
        return render_invalid();
    };

    clear_login_failures(&state, &ip_key);

    let ttl_secs = state.config.auth.session_ttl_s;
    let session_value = sign_session(
        &state.config.auth.secret_key,
        &user.id.to_string(),
        ttl_secs,
    );
    // Clamp the configured TTL to `i64::MAX` (≈ 292 billion years) so an
    // accidentally absurd config value can't wrap to a negative max-age
    // and immediately invalidate every cookie.
    let cookie_ttl = i64::try_from(ttl_secs).unwrap_or(i64::MAX);
    let secure = state
        .config
        .auth
        .effective_cookie_secure(state.config.public_base_url());
    let cookie = Cookie::build((SESSION_COOKIE, session_value))
        .path("/")
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .max_age(Duration::seconds(cookie_ttl))
        .build();
    Ok((jar.add(cookie), Redirect::to("/")).into_response())
}

pub async fn logout(jar: CookieJar) -> impl IntoResponse {
    let cookie = Cookie::build((SESSION_COOKIE, ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(Duration::seconds(0))
        .build();
    (jar.add(cookie), Redirect::to("/login"))
}

fn is_login_rate_limited(state: &AppState, ip_key: &str) -> bool {
    let now = Instant::now();
    if let Some(mut entry) = state.login_failures.get_mut(ip_key) {
        if now.duration_since(entry.window_start) > LOGIN_WINDOW {
            entry.count = 0;
            entry.window_start = now;
            return false;
        }
        return entry.count >= LOGIN_MAX_FAILURES;
    }
    false
}

fn record_login_failure(state: &AppState, ip_key: &str) {
    let now = Instant::now();
    state
        .login_failures
        .entry(ip_key.to_string())
        .and_modify(|e| {
            if now.duration_since(e.window_start) > LOGIN_WINDOW {
                e.count = 1;
                e.window_start = now;
            } else {
                e.count = e.count.saturating_add(1);
            }
        })
        .or_insert(crate::state::LoginFailureWindow {
            count: 1,
            window_start: now,
        });
}

fn clear_login_failures(state: &AppState, ip_key: &str) {
    state.login_failures.remove(ip_key);
}
