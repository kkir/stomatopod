use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use cookie::time::Duration;
use serde::Deserialize;
use std::sync::Arc;

use crate::{
    error::AppError,
    middleware::auth::{sign_session, SESSION_COOKIE},
    state::AppState,
    templates,
};

#[derive(Deserialize)]
pub struct LoginForm {
    pub email: String,
    pub password: String,
}

pub async fn login_page(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    templates::render(&state, "login.html", minijinja::context! {})
}

pub async fn login_submit(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    use argon2::{Argon2, PasswordHash, PasswordVerifier};

    let render_invalid = |state: &AppState| -> Result<Response, AppError> {
        Ok(templates::render(
            state,
            "login.html",
            minijinja::context! { error => "Invalid credentials" },
        )?
        .into_response())
    };

    let Some(user) = state.meta.get_user_by_email(&form.email).await? else {
        return render_invalid(&state);
    };
    let Ok(hash) = PasswordHash::new(&user.password_hash) else {
        return render_invalid(&state);
    };
    if Argon2::default()
        .verify_password(form.password.as_bytes(), &hash)
        .is_err()
    {
        return render_invalid(&state);
    }

    let session_value = sign_session(&state.config.auth.secret_key, &user.id.to_string());
    // Clamp the configured TTL to `i64::MAX` (≈ 292 billion years) so an
    // accidentally absurd config value can't wrap to a negative max-age
    // and immediately invalidate every cookie.
    let ttl_secs = i64::try_from(state.config.auth.session_ttl_s).unwrap_or(i64::MAX);
    let cookie = Cookie::build((SESSION_COOKIE, session_value))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(Duration::seconds(ttl_secs))
        .build();
    Ok((jar.add(cookie), Redirect::to("/app")).into_response())
}

pub async fn logout(jar: CookieJar) -> impl IntoResponse {
    let cookie = Cookie::build((SESSION_COOKIE, ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(Duration::seconds(0))
        .build();
    (jar.add(cookie), Redirect::to("/"))
}
