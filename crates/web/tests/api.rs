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
        if name.ends_with(".jinja") {
            minijinja::AutoEscape::Html
        } else {
            minijinja::AutoEscape::None
        }
    });
    env.add_template("base.jinja", include_str!("../templates/base.jinja"))
        .unwrap();
    env.add_template("login.jinja", include_str!("../templates/login.jinja"))
        .unwrap();
    env.add_template("index.jinja", include_str!("../templates/index.jinja"))
        .unwrap();
    env.add_template("site.jinja", include_str!("../templates/site.jinja"))
        .unwrap();
    env.add_template(
        "site_settings.jinja",
        include_str!("../templates/site_settings.jinja"),
    )
    .unwrap();
    env.add_template(
        "api_keys.jinja",
        include_str!("../templates/api_keys.jinja"),
    )
    .unwrap();
    env.add_template("keys.jinja", include_str!("../templates/keys.jinja"))
        .unwrap();
    env.add_template("docs.jinja", include_str!("../templates/docs.jinja"))
        .unwrap();
    env.add_template("events.jinja", include_str!("../templates/events.jinja"))
        .unwrap();
    env.add_template("funnels.jinja", include_str!("../templates/funnels.jinja"))
        .unwrap();
    env.add_template(
        "partials/top_pages.jinja",
        include_str!("../templates/partials/top_pages.jinja"),
    )
    .unwrap();
    env.add_template(
        "partials/top_referrers.jinja",
        include_str!("../templates/partials/top_referrers.jinja"),
    )
    .unwrap();
    env.add_template(
        "partials/top_countries.jinja",
        include_str!("../templates/partials/top_countries.jinja"),
    )
    .unwrap();
    env.add_template(
        "partials/top_browsers.jinja",
        include_str!("../templates/partials/top_browsers.jinja"),
    )
    .unwrap();
    env.add_template(
        "partials/top_devices.jinja",
        include_str!("../templates/partials/top_devices.jinja"),
    )
    .unwrap();
    env.add_template(
        "partials/top_os.jinja",
        include_str!("../templates/partials/top_os.jinja"),
    )
    .unwrap();
    env.add_template(
        "partials/top_regions.jinja",
        include_str!("../templates/partials/top_regions.jinja"),
    )
    .unwrap();
    env.add_filter("urlencode", |s: String| {
        let mut out = String::with_capacity(s.len());
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char)
                }
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    });
    env.add_template("agents.jinja", include_str!("../templates/agents.jinja"))
        .unwrap();
    env.add_template("agent.jinja", include_str!("../templates/agent.jinja"))
        .unwrap();
    env.add_template(
        "incidents.jinja",
        include_str!("../templates/incidents.jinja"),
    )
    .unwrap();
    env.add_template(
        "partials/agent_spans.jinja",
        include_str!("../templates/partials/agent_spans.jinja"),
    )
    .unwrap();
    env
}

struct TestCtx {
    state: Arc<AppState>,
    backend: Arc<EmbeddedBackend>,
    secret: String,
    _ingest_rx: tokio::sync::mpsc::Receiver<stomatopod_core::domain::event::Event>,
    _span_ingest_rx:
        tokio::sync::mpsc::Receiver<Vec<stomatopod_core::domain::agent_span::AgentSpan>>,
    alerts_rx: tokio::sync::mpsc::Receiver<stomatopod_core::domain::incident::Incident>,
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
    let (span_ingest_tx, span_ingest_rx) = tokio::sync::mpsc::channel(256);
    let (alerts, alerts_rx) = stomatopod_web::alerts::AlertDispatcher::channel();

    let state = Arc::new(AppState {
        backend: backend.clone(),
        agent_store: backend.clone(),
        meta: backend.clone(),
        templates: build_templates(),
        config,
        tracker_hash: "testhash".into(),
        ingest_tx,
        span_ingest_tx,
        site_cache: Arc::new(DashMap::new()),
        sentinel_token_cache: Arc::new(DashMap::new()),
        api_key_cache: Arc::new(DashMap::new()),
        redact_keys: Arc::new(vec!["api_key".into(), "authorization".into()]),
        geo: Arc::new(GeoLookup::new(None)),
        control_channels: DashMap::new(),
        control_seq: std::sync::atomic::AtomicU64::new(0),
        alerts,
    });

    TestCtx {
        state,
        backend,
        secret,
        _ingest_rx: ingest_rx,
        _span_ingest_rx: span_ingest_rx,
        alerts_rx,
        _dir: dir,
    }
}

/// Build a fresh test app from the shared state. Each call returns an owned service.
fn make_app(
    state: Arc<AppState>,
) -> impl tower::Service<
    Request<Body>,
    Response = axum::response::Response,
    Error = std::convert::Infallible,
> {
    build_router(state).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))))
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
    assert!(
        body.contains("function"),
        "tracker should contain JS functions"
    );
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

// ---- Agents dashboard ----

#[tokio::test]
async fn agents_index_renders_when_no_data() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let user_id = Ulid::new().to_string();
    let cookie = format!("sp_session={}", sign_session(&ctx.secret, &user_id));
    let req = Request::builder()
        .uri("/app/agents")
        .header("cookie", &cookie)
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = body_bytes(resp).await;
    let html = std::str::from_utf8(&bytes).unwrap();
    assert!(html.contains("Sentinel Agents"), "page heading missing");
}

#[tokio::test]
async fn incidents_page_lists_manual_kill() {
    use stomatopod_core::domain::incident::{Incident, IncidentStatus, IncidentTrigger};

    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let inc = Incident {
        id: Ulid::new(),
        site_id: site.id,
        agent_id: "agent-with-incident".into(),
        trigger: IncidentTrigger::CostThreshold { usd: 5.0 },
        status: IncidentStatus::Open,
        opened_at: Utc::now(),
        closed_at: None,
    };
    ctx.backend.meta.record_incident(&inc).await.unwrap();

    let user_id = Ulid::new().to_string();
    let cookie = format!("sp_session={}", sign_session(&ctx.secret, &user_id));
    let req = Request::builder()
        .uri("/app/incidents")
        .header("cookie", &cookie)
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = std::str::from_utf8(&body_bytes(resp).await)
        .unwrap()
        .to_string();
    assert!(
        html.contains("agent-with-incident"),
        "incidents page should list the agent"
    );
    assert!(html.contains("cost"));
}

// ---- Alert dispatcher (webhook) ----

#[tokio::test]
async fn webhook_alert_delivered_to_mock_sink() {
    use std::sync::Mutex;
    use stomatopod_core::domain::{
        agent::{AlertChannel, AlertChannelKind},
        incident::{Incident, IncidentStatus, IncidentTrigger},
    };
    use stomatopod_web::alerts::run_alert_dispatcher;

    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    // Spin up a tiny receiver that captures the POST body.
    let received: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
    let captured = received.clone();
    let app = axum::Router::new().route(
        "/hook",
        axum::routing::post(move |axum::Json(v): axum::Json<serde_json::Value>| {
            let captured = captured.clone();
            async move {
                *captured.lock().unwrap() = Some(v);
                StatusCode::OK
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Register the webhook channel.
    let channel = AlertChannel {
        id: Ulid::new(),
        site_id: site.id,
        kind: AlertChannelKind::Webhook,
        url: format!("http://{}/hook", addr),
        secret: Some("topsecret".into()),
        created_at: Utc::now(),
        last_error_at: None,
    };
    ctx.backend
        .meta
        .create_alert_channel(&channel)
        .await
        .unwrap();

    // Drive the dispatcher directly.
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let meta_clone = ctx.backend.clone();
    let handle = tokio::spawn(async move {
        run_alert_dispatcher(rx, meta_clone as _).await;
    });

    let incident = Incident {
        id: Ulid::new(),
        site_id: site.id,
        agent_id: "demo".into(),
        trigger: IncidentTrigger::Repetition {
            count: 7,
            args_hash: "abc".into(),
        },
        status: IncidentStatus::Open,
        opened_at: Utc::now(),
        closed_at: None,
    };
    tx.send(incident).await.unwrap();

    // Poll until the receiver got something or timeout.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if received.lock().unwrap().is_some() {
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!("webhook never received the alert");
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let body = received.lock().unwrap().clone().unwrap();
    assert_eq!(body["agent_id"], "demo");
    assert_eq!(body["trigger_kind"], "repetition");

    handle.abort();
}

// ---- Sentinel control / SSE ----

#[tokio::test]
async fn sentinel_stream_unauthenticated_returns_401() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/api/v1/sentinel/stream")
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sentinel_control_emits_alert() {
    let mut ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let user_id = Ulid::new().to_string();
    let token = sign_session(&ctx.secret, &user_id);
    let body = serde_json::json!({
        "site_id": site.id,
        "agent_id": "agent-alert-x",
        "command": "kill",
        "reason": "burned through budget"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sentinel/control")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let incident =
        tokio::time::timeout(std::time::Duration::from_millis(500), ctx.alerts_rx.recv())
            .await
            .expect("timed out waiting for alert")
            .expect("channel closed");
    assert_eq!(incident.agent_id, "agent-alert-x");
}

#[tokio::test]
async fn sentinel_control_publishes_to_broadcast_channel() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    // Pre-subscribe so the control POST has a receiver.
    let mut rx = ctx.state.control_channel(site.id).subscribe();

    let user_id = Ulid::new().to_string();
    let token = sign_session(&ctx.secret, &user_id);

    let body = serde_json::json!({
        "site_id": site.id,
        "agent_id": "agent-1",
        "command": "kill",
        "reason": "manual stop"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sentinel/control")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let env = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
        .await
        .expect("timed out")
        .expect("recv");
    assert_eq!(env.agent_id, "agent-1");
    match env.command {
        stomatopod_core::domain::control::ControlCommand::Kill { reason } => {
            assert_eq!(reason, "manual stop");
        }
        _ => panic!("expected Kill command"),
    }

    // The control endpoint must have written an Incident row too.
    let incidents = ctx.backend.meta.list_incidents(site.id, 10).await.unwrap();
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].agent_id, "agent-1");
}

// ---- Span ingest endpoint ----

#[tokio::test]
async fn span_ingest_without_bearer_returns_401() {
    let ctx = setup().await;
    let payload = serde_json::json!({"spans": []});

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/spans")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn span_ingest_unknown_token_returns_401() {
    let ctx = setup().await;
    let payload = serde_json::json!({"spans": []});

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/spans")
        .header("authorization", "Bearer not-a-real-token")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn span_ingest_valid_token_redacts_and_enqueues() {
    use stomatopod_core::domain::agent::SentinelToken;
    use stomatopod_ingest::span_handler::token_hash;

    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let raw_token = "sentinel-test-token-xyz";
    let tok = SentinelToken {
        id: Ulid::new(),
        site_id: site.id,
        name: "test-sidecar".into(),
        token_hash: token_hash(raw_token),
        created_at: Utc::now(),
        last_used_at: None,
    };
    ctx.backend.meta.create_sentinel_token(&tok).await.unwrap();

    let now = Utc::now();
    let payload = serde_json::json!({
        "spans": [{
            "agent_id": "agent-a",
            "agent_session_id": "sess-1",
            "kind": "tool_call",
            "model": "claude-opus-4-7",
            "started_at": now,
            "ended_at": now,
            "input_tokens": 10,
            "output_tokens": 20,
            "cost_usd": 0.001,
            "tool_name": "shell",
            "tool_input_hash": "deadbeef",
            "properties": {
                "headers": {"api_key": "sk-leaky"},
                "ok": "fine"
            }
        }]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/spans")
        .header("authorization", format!("Bearer {raw_token}"))
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let mut state_ctx = ctx;
    let resp = make_app(state_ctx.state.clone())
        .oneshot(req)
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // The handler enqueues onto span_ingest_tx; we should receive one
    // batch containing a single span.
    let batch = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        state_ctx._span_ingest_rx.recv(),
    )
    .await
    .expect("timed out waiting for span")
    .expect("channel closed");
    assert_eq!(batch.len(), 1);
    let received = &batch[0];
    assert_eq!(received.agent_id, "agent-a");
    assert_eq!(received.input_tokens, 10);
    assert_eq!(received.site_id, site.id);
    // Redaction applied
    let props = received.properties.clone().unwrap();
    assert_eq!(
        props["headers"]["api_key"],
        serde_json::Value::String("[redacted]".into())
    );
    assert!(props["headers"].get("ok").is_none());
    assert_eq!(props["ok"], serde_json::json!("fine"));
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

// ---- Tier-1 analytics endpoints: filters, custom range, comparison, OS/region ----

/// Create an org + site and mint a session token authorized to read it.
async fn site_and_token(ctx: &TestCtx) -> (Site, String) {
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();
    let token = sign_session(&ctx.secret, &Ulid::new().to_string());
    (site, token)
}

/// GET an authorized analytics URL and return (status, parsed-json).
async fn get_json(state: Arc<AppState>, uri: &str, token: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = make_app(state).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = body_bytes(resp).await;
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test]
async fn api_top_os_and_regions_routes_return_rows() {
    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    for dim in ["top-os", "top-regions"] {
        let (status, json) = get_json(
            ctx.state.clone(),
            &format!("/api/v1/sites/{}/{dim}", site.id),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{dim} should resolve");
        assert!(
            json.get("rows").map(|r| r.is_array()).unwrap_or(false),
            "{dim} response should carry a rows array, got {json}"
        );
    }
}

#[tokio::test]
async fn api_pageviews_compare_attaches_comparison_block() {
    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    let (status, json) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/pageviews?range=7d&compare=1", site.id),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        json.get("comparison").is_some(),
        "compare=1 must attach a comparison block, got {json}"
    );
    // Without compare, no comparison block.
    let (_, plain) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/pageviews?range=7d", site.id),
        &token,
    )
    .await;
    assert!(plain.get("comparison").is_none());
}

#[tokio::test]
async fn api_pageviews_accepts_custom_from_to_range() {
    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    let (status, json) = get_json(
        ctx.state.clone(),
        &format!(
            "/api/v1/sites/{}/pageviews?from=2025-01-01&to=2025-01-31",
            site.id
        ),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("buckets").is_some(), "expected a pageviews result");
}

#[tokio::test]
async fn api_top_pages_accepts_and_tolerates_filters() {
    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    // A well-formed, repeated filter parses without error.
    let (status, json) = get_json(
        ctx.state.clone(),
        &format!(
            "/api/v1/sites/{}/top-pages?filter=browser:eq:Chrome&filter=country:eq:US",
            site.id
        ),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("rows").is_some());

    // A malformed filter is dropped rather than 400-ing the request.
    let (status, _) = get_json(
        ctx.state.clone(),
        &format!(
            "/api/v1/sites/{}/top-pages?filter=not-a-valid-filter",
            site.id
        ),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn api_events_accepts_custom_range() {
    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    let (status, _) = get_json(
        ctx.state.clone(),
        &format!(
            "/api/v1/sites/{}/events?from=2025-01-01&to=2025-02-01",
            site.id
        ),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
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
    assert_eq!(location, "/app", "successful login should redirect to /app");
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
async fn authenticated_site_settings_page_renders_controls() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let user_id = Ulid::new().to_string();
    let cookie = format!("sp_session={}", sign_session(&ctx.secret, &user_id));
    let req = Request::builder()
        .uri(format!("/app/sites/{}/settings", site.id))
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_bytes(resp).await;
    let html = std::str::from_utf8(&body).unwrap();

    assert!(html.contains("Site Settings"));
    assert!(
        html.contains(r#"action="/app/sites/"#),
        "settings form missing"
    );
    assert!(html.contains(r#"<select id="site-timezone" name="timezone""#));
    assert!(html.contains(r#"class="switch-track""#));
    assert!(html.contains("Public Key"));
    assert!(html.contains(r#"class="js-local-time""#));
    assert!(html.contains("ago") || html.contains("just now"));
    assert!(html.contains(">Overview</a>"));
    assert!(html.contains(">Settings</a>"));
}

#[tokio::test]
async fn post_site_update_changes_site_metadata() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let user_id = Ulid::new().to_string();
    let cookie = format!("sp_session={}", sign_session(&ctx.secret, &user_id));
    let req = Request::builder()
        .method("POST")
        .uri(format!("/app/sites/{}", site.id))
        .header("cookie", cookie)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(
            "name=Updated+Site&domain=updated.example.com&timezone=America%2FChicago",
        ))
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(location, format!("/app/sites/{}/settings", site.id));

    let updated = ctx.backend.meta.get_site(site.id).await.unwrap().unwrap();
    assert_eq!(updated.name, "Updated Site");
    assert_eq!(updated.domain, "updated.example.com");
    assert_eq!(updated.timezone, "America/Chicago");
    assert!(
        !updated.is_active,
        "unchecked checkbox should deactivate site"
    );
}

#[tokio::test]
async fn unauthenticated_dashboard_redirects_to_login() {
    let ctx = setup().await;
    let req = Request::builder().uri("/app").body(Body::empty()).unwrap();

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

// ---- API keys ----

#[tokio::test]
async fn ingest_key_accepts_valid_bearer() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let (key, plaintext) =
        stomatopod_core::domain::api_key::ApiKey::new_ingest(org.id, site.id, "backend".into());
    ctx.backend.meta.create_api_key(&key).await.unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/ingest")
        .header("authorization", format!("Bearer {plaintext}"))
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"name":"signup","properties":{"plan":"pro"}}"#,
        ))
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn ingest_rejects_missing_and_unknown_keys() {
    let ctx = setup().await;

    // No Authorization header.
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/ingest")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"signup"}"#))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Unknown key.
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/ingest")
        .header("authorization", "Bearer sk_live_deadbeef")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"signup"}"#))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn ingest_rejects_read_scope_key() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let (key, plaintext) =
        stomatopod_core::domain::api_key::ApiKey::new_read(org.id, Some(site.id), "agent".into());
    ctx.backend.meta.create_api_key(&key).await.unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/ingest")
        .header("authorization", format!("Bearer {plaintext}"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"signup"}"#))
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn read_key_authorizes_analytics_query() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let (key, plaintext) =
        stomatopod_core::domain::api_key::ApiKey::new_read(org.id, None, "agent".into());
    ctx.backend.meta.create_api_key(&key).await.unwrap();

    let req = Request::builder()
        .uri(format!("/api/v1/sites/{}/events", site.id))
        .header("authorization", format!("Bearer {plaintext}"))
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn read_key_cross_org_is_forbidden() {
    let ctx = setup().await;
    // Site lives in org A.
    let org_a = make_org();
    ctx.backend.meta.create_org(&org_a).await.unwrap();
    let site = make_site(org_a.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    // Read key belongs to a different org B.
    let org_b = make_org();
    ctx.backend.meta.create_org(&org_b).await.unwrap();
    let (key, plaintext) =
        stomatopod_core::domain::api_key::ApiKey::new_read(org_b.id, None, "intruder".into());
    ctx.backend.meta.create_api_key(&key).await.unwrap();

    let req = Request::builder()
        .uri(format!("/api/v1/sites/{}/events", site.id))
        .header("authorization", format!("Bearer {plaintext}"))
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn read_key_rejects_unknown_key() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/api/v1/sites")
        .header("authorization", "Bearer rk_deadbeefdeadbeef")
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn global_keys_page_lists_and_filters() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    // Names chosen to not collide with static template copy.
    let (ingest_key, _) = stomatopod_core::domain::api_key::ApiKey::new_ingest(
        org.id,
        site.id,
        "zeta-ingest-svc".into(),
    );
    let (read_key, _) =
        stomatopod_core::domain::api_key::ApiKey::new_read(org.id, None, "omega-read-cli".into());
    ctx.backend.meta.create_api_key(&ingest_key).await.unwrap();
    ctx.backend.meta.create_api_key(&read_key).await.unwrap();

    let user_id = Ulid::new().to_string();
    let cookie = format!("sp_session={}", sign_session(&ctx.secret, &user_id));

    // Unfiltered: both keys present.
    let req = Request::builder()
        .uri("/app/keys")
        .header("cookie", &cookie)
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(body_bytes(resp).await.to_vec()).unwrap();
    assert!(body.contains("zeta-ingest-svc"));
    assert!(body.contains("omega-read-cli"));

    // Org-wide filter: only the read key.
    let req = Request::builder()
        .uri("/app/keys?filter=org")
        .header("cookie", &cookie)
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    let body = String::from_utf8(body_bytes(resp).await.to_vec()).unwrap();
    assert!(body.contains("omega-read-cli"));
    assert!(!body.contains("zeta-ingest-svc"));
}

#[tokio::test]
async fn global_create_ingest_key_requires_site() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();

    let user_id = Ulid::new().to_string();
    let cookie = format!("sp_session={}", sign_session(&ctx.secret, &user_id));

    // Ingest scope with org-wide selection is rejected.
    let req = Request::builder()
        .method("POST")
        .uri("/app/keys")
        .header("cookie", &cookie)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("name=backend&scope=ingest&site=org"))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Org-wide read key succeeds and the plaintext is shown once.
    let req = Request::builder()
        .method("POST")
        .uri("/app/keys")
        .header("cookie", &cookie)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("name=agent&scope=read&site=org"))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(body_bytes(resp).await.to_vec()).unwrap();
    assert!(body.contains("rk_"));
}

// ---- Docs ----

#[tokio::test]
async fn llms_txt_is_public_markdown() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/llms.txt")
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .contains("markdown"));
    let body = String::from_utf8(body_bytes(resp).await.to_vec()).unwrap();
    assert!(body.contains("Stomatopod Documentation"));
    assert!(body.contains("/api/v1/ingest"));
}

#[tokio::test]
async fn docs_page_renders_html_for_authed_user() {
    let ctx = setup().await;
    let user_id = Ulid::new().to_string();
    let cookie = format!("sp_session={}", sign_session(&ctx.secret, &user_id));
    let req = Request::builder()
        .uri("/app/docs")
        .header("cookie", &cookie)
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(body_bytes(resp).await.to_vec()).unwrap();
    // Markdown was rendered to HTML (headings became tags, not literal '#').
    assert!(body.contains("<h2"));
    assert!(body.contains("Emitting custom events"));
    // Headings carry anchor ids and the right-side ToC links to them.
    assert!(body.contains("id=\"emitting-custom-events\""));
    assert!(body.contains("class=\"doc-toc\""));
    assert!(body.contains("href=\"#emitting-custom-events\""));
}

#[tokio::test]
async fn docs_page_requires_auth() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/app/docs")
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    // Dashboard routes redirect unauthenticated users to /login.
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}
