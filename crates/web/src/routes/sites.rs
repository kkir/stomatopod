use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Redirect},
    Form,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use ulid::Ulid;

use stomatopod_core::domain::site::Site;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateSiteForm {
    pub domain: String,
    pub name: String,
    #[serde(default = "default_tz")]
    pub timezone: String,
}

fn default_tz() -> String {
    "UTC".into()
}

pub async fn sites_list(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let orgs = state.meta.list_orgs().await.unwrap_or_default();
    let org_id = orgs.first().map(|o| o.id).unwrap_or_default();
    let sites = state.meta.list_sites(org_id).await.unwrap_or_default();

    let tmpl = state.templates.get_template("index.html").unwrap();
    axum::response::Html(
        tmpl.render(minijinja::context! {
            sites => serde_json::to_value(&sites).unwrap(),
        })
        .unwrap(),
    )
}

pub async fn create_site(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CreateSiteForm>,
) -> impl IntoResponse {
    let orgs = state.meta.list_orgs().await.unwrap_or_default();
    let org_id = match orgs.first() {
        Some(o) => o.id,
        None => return (StatusCode::BAD_REQUEST, "No organization found").into_response(),
    };

    let site = Site {
        id: Ulid::new(),
        org_id,
        domain: form.domain,
        name: form.name,
        timezone: form.timezone,
        public_key: generate_api_key(),
        created_at: Utc::now(),
        is_active: true,
    };

    match state.meta.create_site(&site).await {
        Ok(_) => Redirect::to(&format!("/app/sites/{}", site.id)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn generate_api_key() -> String {
    use blake3::Hasher;
    let mut h = Hasher::new();
    h.update(&Ulid::new().to_bytes());
    h.update(
        &std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_le_bytes(),
    );
    let hash = h.finalize();
    // 32 hex chars = 128 bits of entropy
    hex::encode(&hash.as_bytes()[..16])
}
