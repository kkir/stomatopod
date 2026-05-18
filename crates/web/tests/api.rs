use std::{net::SocketAddr, sync::Arc};

use axum::{
    body::Body,
    extract::connect_info::MockConnectInfo,
    http::{Request, StatusCode},
};
use chrono::Utc;
use dashmap::DashMap;
use http_body_util::BodyExt;
use minijinja::Environment;
use tower::ServiceExt;
use ulid::Ulid;

use stomatopod_core::{
    config::{AuthConfig, Config, EmbeddedConfig},
    domain::{
        org::{Organization, Plan, User, UserRole},
        site::Site,
    },
    traits::MetaStore,
};
use stomatopod_ingest::geo::GeoLookup;
use stomatopod_store::embedded::EmbeddedBackend;
use stomatopod_web::{middleware::auth::sign_session, router::build_router, state::AppState};

// ---- Test harness ----

fn build_templates() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|name| {
        if name.ends_with(".html") {
            minijinja::AutoEscape::Html
        } else {
            minijinja::AutoEscape::None
        }
    });
    env.add_template("base.html", include_str!("../templates/base.html"))
        .unwrap();
    env.add_template("login.html", include_str!("../templates/login.html"))
        .unwrap();
    env.add_template("index.html", include_str!("../templates/index.html"))
        .unwrap();
    env.add_template("site.html", include_str!("../templates/site.html"))
        .unwrap();
    env.add_template("events.html", include_str!("../templates/events.html"))
        .unwrap();
    env.add_template("funnels.html", include_str!("../templates/funnels.html"))
        .unwrap();
    env.add_template(
        "partials/top_pages.html",
        include_str!("../templates/partials/top_pages.html"),
    )
    .unwrap();
    env.add_template(
        "partials/top_referrers.html",
        include_str!("../templates/partials/top_referrers.html"),
    )
    .unwrap();
    env.add_template(
        "partials/top_countries.html",
        include_str!("../templates/partials/top_countries.html"),
    )
    .unwrap();
    env.add_template(
        "partials/top_browsers.html",
        include_str!("../templates/partials/top_browsers.html"),
    )
    .unwrap();
    env.add_template(
        "partials/top_devices.html",
        include_str!("../templates/partials/top_devices.html"),
    )
    .unwrap();
    env
}

struct TestCtx {
    state: Arc<AppState>,
    backend: Arc<EmbeddedBackend>,
    secret: String,
    _ingest_rx: tokio::sync::mpsc::Receiver<stomatopod_core::domain::event::Event>,
    _dir: tempfile::TempDir,
}

async fn setup() -> TestCtx {
    let dir = tempfile::tempdir().unwrap();
    let cfg = EmbeddedConfig {
        data_dir: dir.path().to_path_buf(),
        wal_fsync_interval_ms: 0,
        parquet_flush_rows: 100,
        parquet_flush_interval_s: 3600,
    };
    let backend = Arc::new(EmbeddedBackend::open(&cfg).await.unwrap());

    let secret = "test-secret-key".to_string();
    let config = Arc::new(Config {
        auth: AuthConfig {
            secret_key: secret.clone(),
            session_ttl_s: 86400,
        },
        ..Config::default()
    });

    let (ingest_tx, ingest_rx) = tokio::sync::mpsc::channel(256);

    let state = Arc::new(AppState {
        backend: backend.clone(),
        meta: backend.clone(),
        templates: build_templates(),
        config,
        tracker_hash: "testhash".into(),
        ingest_tx,
        site_cache: Arc::new(DashMap::new()),
        geo: Arc::new(GeoLookup::new(None)),
    });

    TestCtx {
        state,
        backend,
        secret,
        _ingest_rx: ingest_rx,
        _dir: dir,
    }
}

/// Build a fresh test app from the shared state. Each call returns an owned service.
fn make_app(state: Arc<AppState>) -> impl tower::Service<
    Request<Body>,
    Response = axum::response::Response,
    Error = std::convert::Infallible,
> {
    build_router(state)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))))
}

async fn body_bytes(resp: axum::response::Response) -> bytes::Bytes {
    resp.into_body().collect().await.unwrap().to_bytes()
}

fn make_org() -> Organization {
    Organization {
        id: Ulid::new(),
        name: "Test Org".into(),
        slug: format!("org-{}", Ulid::new()),
        plan: Plan::SelfHosted,
        created_at: Utc::now(),
    }
}

fn make_site(org_id: Ulid) -> Site {
    Site {
        id: Ulid::new(),
        org_id,
        domain: "test.example.com".into(),
        name: "Test Site".into(),
        timezone: "UTC".into(),
        public_key: format!("pk-{}", Ulid::new()),
        created_at: Utc::now(),
        is_active: true,
    }
}

// ---- Tracker JS ----

#[tokio::test]
async fn tracker_js_served_with_correct_content_type() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/tracker.js")
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .contains("javascript"),
        "expected JavaScript content-type"
    );
}

#[tokio::test]
async fn tracker_js_has_cache_headers() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/tracker.js")
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    let cache_control = resp
        .headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        cache_control.contains("max-age"),
        "expected cache-control with max-age"
    );
}

#[tokio::test]
async fn tracker_js_body_contains_function() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/tracker.js")
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    let bytes = body_bytes(resp).await;
    let body = std::str::from_utf8(&bytes).unwrap();
    assert!(body.contains("function"), "tracker should contain JS functions");
    assert!(
        body.contains("pageview"),
        "tracker should send pageview events"
    );
}

// ---- Ingest endpoint ----

#[tokio::test]
async fn ingest_unknown_site_key_returns_401() {
    let ctx = setup().await;
    let payload = serde_json::json!({
        "k": "nonexistent-key",
        "n": "pageview",
        "u": "https://example.com/"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/event")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn ingest_known_site_key_returns_204() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let payload = serde_json::json!({
        "k": site.public_key,
        "n": "pageview",
        "u": "https://example.com/",
        "w": 1920,
        "h": 1080
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/event")
        .header("content-type", "application/json")
        .header(
            "user-agent",
            "Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0.0.0",
        )
        .body(Body::from(payload.to_string()))
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn ingest_bot_user_agent_returns_no_content() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let payload = serde_json::json!({
        "k": site.public_key,
        "n": "pageview",
        "u": "https://example.com/"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/event")
        .header("content-type", "application/json")
        .header(
            "user-agent",
            "Googlebot/2.1 (+http://www.google.com/bot.html)",
        )
        .body(Body::from(payload.to_string()))
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn ingest_malformed_json_returns_client_error() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/event")
        .header("content-type", "application/json")
        .body(Body::from("not json at all"))
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert!(
        resp.status().is_client_error(),
        "malformed JSON should return 4xx, got {}",
        resp.status()
    );
}

// ---- Analytics API auth ----

#[tokio::test]
async fn api_sites_requires_auth() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/api/v1/sites")
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_sites_accepts_valid_bearer_token() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();

    let user_id = Ulid::new().to_string();
    let token = sign_session(&ctx.secret, &user_id);

    let req = Request::builder()
        .uri("/api/v1/sites")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = body_bytes(resp).await;
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        json.get("sites").is_some(),
        "response should have 'sites' key"
    );
}

#[tokio::test]
async fn api_sites_rejects_tampered_token() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/api/v1/sites")
        .header(
            "authorization",
            "Bearer fakeuserid.0000000000000000000000000000000000",
        )
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_site_not_found_returns_404() {
    let ctx = setup().await;
    let user_id = Ulid::new().to_string();
    let token = sign_session(&ctx.secret, &user_id);

    let req = Request::builder()
        .uri("/api/v1/sites/nonexistent.example.com/pageviews")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_pageviews_resolves_known_site() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let user_id = Ulid::new().to_string();
    let token = sign_session(&ctx.secret, &user_id);

    let req = Request::builder()
        .uri(format!("/api/v1/sites/{}/pageviews", site.domain))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    // Auth passed and site resolved — not 401 and not 404.
    assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_ne!(resp.status(), StatusCode::NOT_FOUND);
}

// ---- Auth routes ----

#[tokio::test]
async fn login_page_returns_html() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/login")
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = body_bytes(resp).await;
    let body = std::str::from_utf8(&bytes).unwrap();
    assert!(
        body.contains("Sign in"),
        "login page should contain sign-in form"
    );
    assert!(
        body.contains(r#"name="email""#),
        "login page should have email field"
    );
    assert!(
        body.contains(r#"name="password""#),
        "login page should have password field"
    );
}

#[tokio::test]
async fn login_with_bad_credentials_shows_error() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();

    let hash = hash_password("hunter2");
    let user = User {
        id: Ulid::new(),
        org_id: org.id,
        email: "user@example.com".into(),
        password_hash: hash,
        role: UserRole::Owner,
        created_at: Utc::now(),
    };
    ctx.backend.meta.create_user(&user).await.unwrap();

    let body = "email=user%40example.com&password=wrongpassword";
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = body_bytes(resp).await;
    let html = std::str::from_utf8(&bytes).unwrap();
    assert!(
        html.contains("Invalid credentials"),
        "should show error message for bad password"
    );
}

#[tokio::test]
async fn login_with_correct_credentials_redirects() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();

    let password = "correcthorsebatterystaple";
    let hash = hash_password(password);
    let user = User {
        id: Ulid::new(),
        org_id: org.id,
        email: "admin@example.com".into(),
        password_hash: hash,
        role: UserRole::Owner,
        created_at: Utc::now(),
    };
    ctx.backend.meta.create_user(&user).await.unwrap();

    let body = "email=admin%40example.com&password=correcthorsebatterystaple";
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let location = resp
        .headers()
        .get("location")
        .and_then(|v: &axum::http::HeaderValue| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(location, "/", "successful login should redirect to /");
}

#[tokio::test]
async fn login_with_unknown_email_shows_error() {
    let ctx = setup().await;

    let body = "email=ghost%40example.com&password=anything";
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = body_bytes(resp).await;
    let html = std::str::from_utf8(&bytes).unwrap();
    assert!(html.contains("Invalid credentials"));
}

#[tokio::test]
async fn logout_clears_cookie_and_redirects_to_login() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/logout")
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let location = resp
        .headers()
        .get("location")
        .and_then(|v: &axum::http::HeaderValue| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(location, "/login");

    let set_cookie = resp
        .headers()
        .get("set-cookie")
        .and_then(|v: &axum::http::HeaderValue| v.to_str().ok())
        .unwrap_or("");
    assert!(
        set_cookie.contains("Max-Age=0"),
        "logout should expire the session cookie"
    );
}

// ---- Dashboard requires auth ----

#[tokio::test]
async fn unauthenticated_dashboard_redirects_to_login() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/")
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let location = resp
        .headers()
        .get("location")
        .and_then(|v: &axum::http::HeaderValue| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(location, "/login");
}

// ---- Helpers ----

fn hash_password(password: &str) -> String {
    use argon2::{
        password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
        Argon2,
    };
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .unwrap()
        .to_string()
}
