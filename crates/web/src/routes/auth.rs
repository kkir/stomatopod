use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    Form,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct LoginForm {
    pub email: String,
    pub password: String,
}

pub async fn login_page(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let tmpl = state.templates.get_template("login.html").unwrap();
    axum::response::Html(tmpl.render(minijinja::context! {}).unwrap())
}

pub async fn login_submit(
    State(state): State<Arc<AppState>>,
    Form(form): Form<LoginForm>,
) -> Response {
    use argon2::{Argon2, PasswordHash, PasswordVerifier};

    let user = match state.meta.get_user_by_email(&form.email).await {
        Ok(Some(u)) => u,
        _ => {
            let tmpl = state.templates.get_template("login.html").unwrap();
            return axum::response::Html(
                tmpl.render(minijinja::context! { error => "Invalid credentials" })
                    .unwrap(),
            )
            .into_response();
        }
    };

    let hash = PasswordHash::new(&user.password_hash).unwrap();
    if Argon2::default()
        .verify_password(form.password.as_bytes(), &hash)
        .is_err()
    {
        let tmpl = state.templates.get_template("login.html").unwrap();
        return axum::response::Html(
            tmpl.render(minijinja::context! { error => "Invalid credentials" })
                .unwrap(),
        )
        .into_response();
    }

    // Set session cookie
    let cookie = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        crate::middleware::auth::SESSION_COOKIE,
        user.id,
        state.config.auth.session_ttl_s,
    );

    axum::http::Response::builder()
        .status(axum::http::StatusCode::SEE_OTHER)
        .header("Location", "/")
        .header("Set-Cookie", cookie)
        .body(axum::body::Body::empty())
        .unwrap()
}

pub async fn logout() -> impl IntoResponse {
    let cookie = format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        crate::middleware::auth::SESSION_COOKIE
    );
    axum::http::Response::builder()
        .status(axum::http::StatusCode::SEE_OTHER)
        .header("Location", "/login")
        .header("Set-Cookie", cookie)
        .body(axum::body::Body::empty())
        .unwrap()
}
