//! Public marketing pages: the homepage, per-product pages, and per-audience
//! pages. All routes here are unauthenticated and rendered from
//! `templates/marketing/*.html`.

use std::sync::Arc;

use axum::{
    extract::State,
    response::{Html, IntoResponse},
};
use minijinja::context;

use crate::state::AppState;

fn render(state: &AppState, template: &str, active: &str) -> Html<String> {
    let tmpl = state.templates.get_template(template).unwrap();
    let html = tmpl
        .render(context! {
            active => active,
            marketing_css_url => format!("/static/marketing.{}.css", state.marketing_css_hash),
            anime_js_url => format!("/static/anime.{}.js", state.anime_js_hash),
        })
        .unwrap_or_else(|e| format!("<p>Template error: {e}</p>"));
    Html(html)
}

pub async fn home(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    render(&state, "marketing/home.html", "home")
}

pub async fn web_analytics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    render(&state, "marketing/web_analytics.html", "analytics")
}

pub async fn ai_firewall(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    render(&state, "marketing/ai_firewall.html", "firewall")
}

pub async fn for_saas(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    render(&state, "marketing/for_saas.html", "for")
}

pub async fn for_agencies(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    render(&state, "marketing/for_agencies.html", "for")
}

pub async fn for_ai_teams(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    render(&state, "marketing/for_ai_teams.html", "for")
}

pub async fn for_regulated(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    render(&state, "marketing/for_regulated.html", "for")
}
