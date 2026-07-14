use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    body::Body,
    extract::connect_info::MockConnectInfo,
    http::{Request, StatusCode},
};
use chrono::Utc;
use dashmap::DashMap;
use http_body_util::BodyExt;
use tower::ServiceExt;
use ulid::Ulid;

use stomatopod_core::{
    config::{AuthConfig, Config, EmbeddedConfig},
    domain::{
        org::{Organization, Plan, User, UserRole},
        site::Site,
    },
    traits::{MetaStore, StorageBackend},
};
use stomatopod_ingest::{batch::run_batcher, geo::GeoLookup};
use stomatopod_store::embedded::EmbeddedBackend;
use stomatopod_web::{middleware::auth::sign_session, router::build_router, state::AppState};

// ---- Test harness ----

/// Test digest transport: records every message without outbound HTTP.
#[derive(Clone, Default)]
struct CapturingNotifier {
    sent: Arc<std::sync::Mutex<Vec<stomatopod_web::digest::DigestMessage>>>,
}

#[async_trait::async_trait]
impl stomatopod_web::digest::DigestNotifier for CapturingNotifier {
    async fn send(
        &self,
        _channels: &[stomatopod_core::domain::alert_channel::AlertChannel],
        msg: stomatopod_web::digest::DigestMessage,
    ) -> Result<(), String> {
        self.sent.lock().unwrap().push(msg);
        Ok(())
    }
}

/// How the test harness drains (or holds) the ingest channel.
enum IngestDrain {
    /// Background [`run_batcher`] task.
    #[allow(dead_code)]
    Batcher(tokio::task::JoinHandle<()>),
    /// Keep the receiver alive without reading (fills up for back-pressure).
    #[allow(dead_code)]
    Hold(tokio::sync::mpsc::Receiver<stomatopod_core::domain::event::Event>),
    /// Receiver already dropped (channel closed → 503).
    Closed,
}

struct TestCtx {
    state: Arc<AppState>,
    backend: Arc<EmbeddedBackend>,
    digest_sink: CapturingNotifier,
    secret: String,
    /// Keeps the ingest path alive (or intentionally closed) for the test.
    _ingest_drain: IngestDrain,
    _dir: tempfile::TempDir,
}

async fn setup() -> TestCtx {
    // Default flush is slow; most tests only care about HTTP status codes.
    setup_with_flush(100, 3600).await
}

/// Fast parquet + batcher cadence for tests that assert data lands in queries.
async fn setup_for_data() -> TestCtx {
    setup_with_flush(1, 1).await
}

/// Like [`setup`] but with a tunable Parquet flush cadence so data-path
/// tests can ingest events and have them land on disk within the test.
/// Always spawns [`run_batcher`] so HTTP ingest reaches the store.
async fn setup_with_flush(flush_rows: usize, flush_interval_s: u64) -> TestCtx {
    setup_ingest(flush_rows, flush_interval_s, 256, IngestMode::Batcher).await
}

enum IngestMode {
    /// Drain via [`run_batcher`].
    Batcher,
    /// Leave events in the channel (no consumer).
    Hold,
    /// Drop the receiver immediately so `try_send` sees a closed channel.
    Closed,
}

async fn setup_ingest(
    flush_rows: usize,
    flush_interval_s: u64,
    channel_size: usize,
    mode: IngestMode,
) -> TestCtx {
    let dir = tempfile::tempdir().unwrap();
    let cfg = EmbeddedConfig {
        data_dir: dir.path().to_path_buf(),
        wal_fsync_interval_ms: 0,
        parquet_flush_rows: flush_rows,
        parquet_flush_interval_s: flush_interval_s,
        allow_ephemeral: true,
        ..Default::default()
    };
    let backend = Arc::new(EmbeddedBackend::open(&cfg).await.unwrap());

    let secret = "test-secret-key".to_string();
    let config = Arc::new(Config {
        auth: AuthConfig {
            secret_key: secret.clone(),
            session_ttl_s: 86400,
            ..AuthConfig::default()
        },
        ..Config::default()
    });

    let (ingest_tx, ingest_rx) = tokio::sync::mpsc::channel(channel_size);
    let ingest_drain = match mode {
        IngestMode::Batcher => {
            let batcher_backend: Arc<dyn StorageBackend> = backend.clone();
            IngestDrain::Batcher(tokio::spawn(async move {
                run_batcher(ingest_rx, batcher_backend, 1, 20).await;
            }))
        }
        IngestMode::Hold => IngestDrain::Hold(ingest_rx),
        IngestMode::Closed => {
            drop(ingest_rx);
            IngestDrain::Closed
        }
    };

    let digest_sink = CapturingNotifier::default();

    let state = Arc::new(AppState {
        backend: backend.clone(),
        meta: backend.clone(),
        config,
        tracker_hash: "testhash".into(),
        ingest_tx,
        site_cache: Arc::new(DashMap::new()),
        api_key_cache: Arc::new(DashMap::new()),
        geo: Arc::new(GeoLookup::new(None)),
        digest_notifier: Arc::new(digest_sink.clone()),
        login_failures: Arc::new(DashMap::new()),
    });

    TestCtx {
        state,
        backend,
        digest_sink,
        secret,
        _ingest_drain: ingest_drain,
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
    make_site_with_domain(org_id, "test.example.com")
}

fn make_site_with_domain(org_id: Ulid, domain: &str) -> Site {
    Site {
        id: Ulid::new(),
        org_id,
        domain: domain.into(),
        name: format!("Site {domain}"),
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

/// Source-level contracts for regressions that previously shipped:
/// scroll/replaceState spam, missing load pageview (wrong endpoint), etc.
#[tokio::test]
async fn tracker_js_has_pageview_and_endpoint_guards() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/tracker.js")
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    let body = String::from_utf8(body_bytes(resp).await.to_vec()).unwrap();

    // Initial pageview path exists (not SPA-only).
    assert!(
        body.contains("trackInitial") || body.contains("visibilityState"),
        "tracker should fire an initial pageview (possibly deferred on prerender)"
    );

    // Path+query dedupe so hash-only replaceState (scroll-spy) does not spam.
    assert!(
        body.contains("pathname") && body.contains("search"),
        "tracker should key pageviews on pathname+search to ignore hash-only updates"
    );
    assert!(
        body.contains("lastPath"),
        "tracker should remember the last counted path for dedupe"
    );

    // Prefer data-api; otherwise derive from script.src origin (not page origin).
    assert!(
        body.contains("data-api") && body.contains("script.src"),
        "tracker should read data-api and fall back to script.src origin"
    );
    assert!(
        body.contains("/api/v1/event"),
        "default ingest path must be /api/v1/event"
    );

    // SPA hooks still present.
    assert!(body.contains("pushState"), "must hook history.pushState");
    assert!(
        body.contains("replaceState"),
        "must hook history.replaceState"
    );
    assert!(body.contains("popstate"), "must listen for popstate");

    // Double-include and pre-load queue.
    assert!(
        body.contains(".l") || body.contains("stomatopod.l"),
        "must guard against double initialization"
    );
    assert!(
        body.contains("Array.isArray"),
        "must drain a pre-load stomatopod.push queue"
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

// ---- Alert webhook sink ----

#[tokio::test]
async fn webhook_alert_delivered_to_mock_sink() {
    use std::sync::Mutex;
    use stomatopod_core::domain::{
        alert_channel::{AlertChannel, AlertChannelKind},
        incident::{Incident, IncidentTrigger},
    };
    use stomatopod_web::alerts::sinks::{AlertSink, WebhookSink};

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

    let channel = AlertChannel {
        id: Ulid::new(),
        site_id: site.id,
        kind: AlertChannelKind::Webhook,
        url: format!("http://{}/hook", addr),
        secret: Some("topsecret".into()),
        created_at: Utc::now(),
        last_error_at: None,
    };

    let incident = Incident {
        id: Ulid::new(),
        site_id: site.id,
        source: "analytics".into(),
        trigger: IncidentTrigger::AnalyticsAlert {
            alert_type: "traffic_spike".into(),
            value: 200.0,
            threshold: 100.0,
        },
        opened_at: Utc::now(),
    };

    let sink = WebhookSink::new(reqwest::Client::new());
    // Loopback destinations are rejected by SSRF hardening before the HTTP call.
    let err = sink
        .dispatch(&channel, &incident)
        .await
        .expect_err("loopback webhook must be blocked");
    assert!(
        err.to_string().contains("not publicly routable")
            || err.to_string().contains("not allowed"),
        "unexpected error: {err}"
    );
    assert!(
        received.lock().unwrap().is_none(),
        "SSRF block must prevent delivery"
    );
    // Payload shape is still covered by unit tests on incident_payload.
    let _ = incident;
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
    let token = sign_session(&ctx.secret, &user_id, 3600);

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
    let token = sign_session(&ctx.secret, &user_id, 3600);

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
    let token = sign_session(&ctx.secret, &user_id, 3600);

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
    let token = sign_session(&ctx.secret, &Ulid::new().to_string(), 3600);
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
async fn api_top_pages_json_includes_optional_spark() {
    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    // Without spark=1, JSON is the plain TopList shape (no spark field required).
    let (status, json) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/top-pages?range=7d", site.id),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = json["rows"].as_array().expect("rows array");
    assert!(rows.is_empty() || rows[0].get("value").is_some());

    // With spark=1, response still succeeds (empty site has no spark series).
    let (status, json) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/top-pages?range=7d&spark=1", site.id),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = json["rows"].as_array().expect("rows array");
    assert!(rows.is_empty() || rows[0].get("value").is_some());
}

#[tokio::test]
async fn api_utm_dimension_route_returns_rows() {
    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    let (status, json) = get_json(
        ctx.state.clone(),
        &format!(
            "/api/v1/sites/{}/utm?range=30d&dimension=source&limit=20",
            site.id
        ),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "utm route should resolve: {json}");
    assert!(
        json.get("rows").map(|r| r.is_array()).unwrap_or(false),
        "utm response should carry a rows array, got {json}"
    );

    let (status, _) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/utm?range=30d", site.id),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "dimension is required");
}

#[tokio::test]
async fn api_funnel_delete_removes_definition() {
    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    let create_body = serde_json::json!({
        "name": "Signup",
        "steps": [
            {"name": "Landing", "event_name": "pageview", "filters": []},
            {"name": "Signup", "event_name": "signup", "filters": []}
        ]
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/funnels", site.id))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(create_body.to_string()))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: serde_json::Value =
        serde_json::from_slice(&body_bytes(resp).await).expect("create body");
    let funnel_id = created["id"].as_str().expect("funnel id");

    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/sites/{}/funnels/{funnel_id}", site.id))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let (status, json) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/funnels", site.id),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let funnels = json["funnels"].as_array().expect("funnels array");
    assert!(
        funnels.iter().all(|f| f["id"].as_str() != Some(funnel_id)),
        "deleted funnel should not appear in list: {json}"
    );
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
    assert_eq!(
        location, "/",
        "successful login should redirect to the dashboard root"
    );
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
async fn unauthenticated_api_rejects_without_token() {
    // Session-authenticated SPA routes are guarded outside `build_router`.
    // The JSON API requires a bearer token (or session) via `require_api_auth`.
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/api/v1/sites")
        .body(Body::empty())
        .unwrap();

    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
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
async fn read_key_cannot_create_analytics_alert() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();
    let (key, plaintext) =
        stomatopod_core::domain::api_key::ApiKey::new_read(org.id, None, "agent".into());
    ctx.backend.meta.create_api_key(&key).await.unwrap();

    let (status, json) = send_json(
        ctx.state.clone(),
        "POST",
        &format!("/api/v1/sites/{}/analytics-alerts", site.id),
        &plaintext,
        Some(serde_json::json!({
            "type": "traffic_spike",
            "threshold": 100.0,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "got {json}");
}

#[tokio::test]
async fn expired_session_bearer_is_rejected() {
    let ctx = setup().await;
    // Craft a correctly signed but already-expired token.
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(60);
    let user_id = Ulid::new().to_string();
    let payload = format!("{user_id}.{exp}");
    let key = blake3::derive_key("stomatopod session signing key v1", ctx.secret.as_bytes());
    let mac = blake3::keyed_hash(&key, payload.as_bytes());
    let token = format!("{payload}.{}", hex::encode(&mac.as_bytes()[..16]));

    let req = Request::builder()
        .uri("/api/v1/sites")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn responses_include_security_headers() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/tracker.js")
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    assert_eq!(
        resp.headers()
            .get("x-frame-options")
            .and_then(|v| v.to_str().ok()),
        Some("DENY")
    );
    assert!(resp.headers().get("content-security-policy").is_some());
}

// ---- Docs / OpenAPI ----

#[tokio::test]
async fn health_and_ready_are_public() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&body_bytes(resp).await).expect("health json");
    assert_eq!(body["status"], "ok");

    let req = Request::builder()
        .uri("/ready")
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&body_bytes(resp).await).expect("ready json");
    assert_eq!(body["status"], "ready");
}

#[tokio::test]
async fn change_password_requires_session_and_correct_current() {
    use stomatopod_web::middleware::auth::sign_session_bound;

    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let password = "correcthorsebatterystaple";
    let hash = hash_password(password);
    let user = User {
        id: Ulid::new(),
        org_id: org.id,
        email: "owner@example.com".into(),
        password_hash: hash.clone(),
        role: UserRole::Owner,
        created_at: Utc::now(),
    };
    ctx.backend.meta.create_user(&user).await.unwrap();
    let token = sign_session_bound(&ctx.secret, &user.id.to_string(), 3600, &hash);

    let body = serde_json::json!({
        "current_password": "wrong-password",
        "new_password": "brandnewpassword99"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/me/password")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let body = serde_json::json!({
        "current_password": password,
        "new_password": "brandnewpassword99"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/me/password")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let updated = ctx
        .backend
        .meta
        .get_user(user.id)
        .await
        .unwrap()
        .expect("user");
    assert_ne!(updated.password_hash, user.password_hash);

    // Old password-bound token must be rejected after rotation.
    let req = Request::builder()
        .uri("/api/v1/me")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "password change should invalidate prior sessions"
    );

    // A token bound to the new hash still works.
    let new_token = sign_session_bound(
        &ctx.secret,
        &user.id.to_string(),
        3600,
        &updated.password_hash,
    );
    let req = Request::builder()
        .uri("/api/v1/me")
        .header("authorization", format!("Bearer {new_token}"))
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn openapi_json_is_public_and_lists_core_paths() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/openapi.json")
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .contains("json"));
    let body: serde_json::Value =
        serde_json::from_slice(&body_bytes(resp).await).expect("openapi json");
    assert_eq!(body["openapi"].as_str().unwrap_or(""), "3.1.0");
    let paths = body["paths"].as_object().expect("paths object");
    assert!(paths.contains_key("/api/v1/event"));
    assert!(paths.contains_key("/api/v1/ingest"));
    assert!(paths.contains_key("/api/v1/sites"));
    assert!(paths.contains_key("/api/v1/sites/{site}/pageviews"));
    assert!(paths.contains_key("/openapi.json"));
    assert!(body["components"]["securitySchemes"]
        .as_object()
        .map(|s| s.contains_key("read_key") && s.contains_key("ingest_key"))
        .unwrap_or(false));
}

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

/// Install snippet needs data-site + absolute tracker.js src; data-api is
/// optional (derived from script origin when omitted).
#[tokio::test]
async fn llms_txt_install_snippet_includes_data_site() {
    let ctx = setup().await;
    let req = Request::builder()
        .uri("/llms.txt")
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    let body = String::from_utf8(body_bytes(resp).await.to_vec()).unwrap();

    assert!(
        body.contains("data-site="),
        "docs install snippet must include data-site"
    );
    assert!(
        body.contains("src=\"https://your-host/tracker.js\"")
            || body.contains("src='https://your-host/tracker.js'"),
        "docs should show an absolute tracker.js src in the install example"
    );
    assert!(
        body.contains("/tracker.js"),
        "docs should reference /tracker.js"
    );
    // Default example should not force data-api; document override separately.
    let install_example = body
        .split("```html")
        .nth(1)
        .and_then(|s| s.split("```").next())
        .unwrap_or("");
    assert!(
        !install_example.contains("data-api"),
        "default install example should omit data-api (derived from script src)"
    );
}

#[tokio::test]
async fn api_docs_json_includes_heading_ids_for_ui_deep_links() {
    let ctx = setup().await;
    let token = sign_session(&ctx.secret, &Ulid::new().to_string(), 3600);
    let req = Request::builder()
        .uri("/api/v1/docs")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    let html = json["html"].as_str().unwrap_or("");
    let toc = json["toc"].as_array().cloned().unwrap_or_default();
    assert!(!html.is_empty(), "docs html should be non-empty");
    assert!(!toc.is_empty(), "docs toc should list sections");

    for slug in ["installing-the-browser-tracker", "emitting-custom-events"] {
        assert!(
            html.contains(&format!("id=\"{slug}\"")),
            "docs HTML missing id={slug}"
        );
        assert!(
            toc.iter().any(|t| t["slug"].as_str() == Some(slug)),
            "docs TOC missing slug={slug}"
        );
    }
}

#[tokio::test]
async fn api_sites_list_includes_timezone() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let mut site = make_site(org.id);
    site.timezone = "America/New_York".into();
    ctx.backend.meta.create_site(&site).await.unwrap();

    let token = sign_session(&ctx.secret, &Ulid::new().to_string(), 3600);
    let req = Request::builder()
        .uri("/api/v1/sites")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    let sites = json["sites"].as_array().unwrap();
    let found = sites.iter().find(|s| s["id"] == site.id.to_string());
    let found = found.expect("site present in list");
    assert_eq!(found["timezone"], "America/New_York");
}

#[tokio::test]
async fn api_patch_site_timezone_validates_iana() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();
    let token = sign_session(&ctx.secret, &Ulid::new().to_string(), 3600);

    let bad = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/sites/{}", site.id))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"timezone":"NotAZone"}"#))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(bad).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let good = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/sites/{}", site.id))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"timezone":"Europe/Berlin"}"#))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(good).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(json["timezone"], "Europe/Berlin");
}

// ---- Tier-2 analytics endpoints ----

/// Send an authorized request with an optional JSON body; return (status, json).
async fn send_json(
    state: Arc<AppState>,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(b) => builder
            .header("content-type", "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => {
            builder = builder.header("content-type", "application/json");
            builder.body(Body::empty()).unwrap()
        }
    };
    let resp = make_app(state).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = body_bytes(resp).await;
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test]
async fn entry_exit_routes_resolve() {
    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    for path in ["top-entry-pages", "top-exit-pages"] {
        let (status, json) = get_json(
            ctx.state.clone(),
            &format!("/api/v1/sites/{}/{path}", site.id),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{path} should resolve");
        assert!(json.get("rows").is_some(), "{path} carries rows");
    }
}

#[tokio::test]
async fn top_pages_csv_export_sets_attachment_header() {
    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    let req = Request::builder()
        .uri(format!("/api/v1/sites/{}/top-pages?format=csv", site.id))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .contains("csv"));
    assert!(resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .contains("top-pages.csv"));
    let body = String::from_utf8(body_bytes(resp).await.to_vec()).unwrap();
    assert!(body.starts_with("value,pageviews,sessions,pct"));
}

#[tokio::test]
async fn export_endpoints_serve_csv_and_json() {
    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    // JSON shape.
    let (status, json) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/export/sessions", site.id),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("rows").is_some());

    // CSV header row for events export.
    let req = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/export/events?format=csv",
            site.id
        ))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(body_bytes(resp).await.to_vec()).unwrap();
    assert!(body.starts_with("id,name,kind,timestamp"));
}

#[tokio::test]
async fn create_site_seeds_default_analytics_alerts() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let user = stomatopod_core::domain::org::User {
        id: Ulid::new(),
        org_id: org.id,
        email: format!("owner-{}@example.com", Ulid::new()),
        password_hash: "x".into(),
        role: stomatopod_core::domain::org::UserRole::Owner,
        created_at: Utc::now(),
    };
    ctx.backend.meta.create_user(&user).await.unwrap();
    let token = sign_session(&ctx.secret, &user.id.to_string(), 3600);

    let (status, json) = send_json(
        ctx.state.clone(),
        "POST",
        "/api/v1/sites",
        &token,
        Some(serde_json::json!({
            "domain": "seeded-alerts.example",
            "name": "Seeded Alerts",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "got {json}");
    let site_id = json["id"].as_str().unwrap();

    let (_, list) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{site_id}/analytics-alerts"),
        &token,
    )
    .await;
    let alerts = list["alerts"].as_array().unwrap();
    assert_eq!(
        alerts.len(),
        3,
        "expected starter spike/drop/referrer rules"
    );
    let kinds: std::collections::HashSet<_> =
        alerts.iter().filter_map(|a| a["kind"].as_str()).collect();
    assert!(kinds.contains("traffic_spike"));
    assert!(kinds.contains("traffic_drop"));
    assert!(kinds.contains("new_referrer_spike"));
}

#[tokio::test]
async fn analytics_alert_requires_channel_then_round_trips() {
    use stomatopod_core::domain::alert_channel::{AlertChannel, AlertChannelKind};

    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    // Without any notification destination, creation is rejected.
    let (status, json) = send_json(
        ctx.state.clone(),
        "POST",
        &format!("/api/v1/sites/{}/analytics-alerts", site.id),
        &token,
        Some(serde_json::json!({
            "type": "traffic_spike",
            "threshold": 200.0,
            "window_minutes": 60,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got {json}");

    // Register a channel, then creation succeeds without a channel_id body field.
    let channel = AlertChannel {
        id: Ulid::new(),
        site_id: site.id,
        kind: AlertChannelKind::Webhook,
        url: "http://example.invalid/hook".into(),
        secret: None,
        created_at: Utc::now(),
        last_error_at: None,
    };
    ctx.backend
        .meta
        .create_alert_channel(&channel)
        .await
        .unwrap();

    let (status, json) = send_json(
        ctx.state.clone(),
        "POST",
        &format!("/api/v1/sites/{}/analytics-alerts", site.id),
        &token,
        Some(serde_json::json!({
            "type": "traffic_spike",
            "threshold": 200.0,
            "window_minutes": 60,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "got {json}");
    let alert_id = json["id"].as_str().unwrap().to_string();

    // List, disable, delete. create_site seeds three starter rules, so the
    // manual create is a fourth row.
    let (_, json) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/analytics-alerts", site.id),
        &token,
    )
    .await;
    assert_eq!(json["alerts"].as_array().unwrap().len(), 4);

    let (status, _) = send_json(
        ctx.state.clone(),
        "PATCH",
        &format!("/api/v1/sites/{}/analytics-alerts/{alert_id}", site.id),
        &token,
        Some(serde_json::json!({"enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let disabled = ctx
        .backend
        .meta
        .get_analytics_alert(Ulid::from_string(&alert_id).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(!disabled.enabled);

    let (status, _) = send_json(
        ctx.state.clone(),
        "DELETE",
        &format!("/api/v1/sites/{}/analytics-alerts/{alert_id}", site.id),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[test]
fn analytics_alert_decide_logic() {
    use stomatopod_core::domain::analytics_alert::AnalyticsAlertKind::*;
    use stomatopod_web::alerts::decide;

    // Spike fires when current is >threshold% above baseline.
    assert!(decide(TrafficSpike, 50.0, 200.0, 100.0).is_some());
    assert!(decide(TrafficSpike, 50.0, 120.0, 100.0).is_none());
    // No baseline (new site) → never fires.
    assert!(decide(TrafficSpike, 50.0, 200.0, 0.0).is_none());
    // Drop fires when current is far below baseline.
    assert!(decide(TrafficDrop, 50.0, 30.0, 100.0).is_some());
    assert!(decide(TrafficDrop, 50.0, 80.0, 100.0).is_none());
    // Referrer spike: share above threshold percent.
    assert!(decide(NewReferrerSpike, 40.0, 55.0, 0.0).is_some());
    assert!(decide(NewReferrerSpike, 40.0, 12.0, 0.0).is_none());
}

#[tokio::test]
async fn analytics_alert_fires_records_and_respects_cooldown() {
    use std::sync::Mutex;
    use stomatopod_core::domain::{
        alert_channel::{AlertChannel, AlertChannelKind},
        analytics_alert::{AnalyticsAlert, AnalyticsAlertConfig, AnalyticsAlertKind},
    };
    use stomatopod_web::alerts::{
        process_alert,
        sinks::{SlackSink, TelegramSink, WebhookSink},
    };

    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    // Capture webhook deliveries.
    let received: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
    let counter = received.clone();
    let app = axum::Router::new().route(
        "/hook",
        axum::routing::post(move || {
            let counter = counter.clone();
            async move {
                *counter.lock().unwrap() += 1;
                StatusCode::OK
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let channel = AlertChannel {
        id: Ulid::new(),
        site_id: site.id,
        kind: AlertChannelKind::Webhook,
        url: format!("http://{addr}/hook"),
        secret: None,
        created_at: Utc::now(),
        last_error_at: None,
    };
    ctx.backend
        .meta
        .create_alert_channel(&channel)
        .await
        .unwrap();

    // A new-referrer-spike alert with a negative threshold fires when
    // the top referrer share is 0 (empty site), exercising the full
    // fire + dispatch path without needing seeded traffic.
    let alert = AnalyticsAlert {
        id: Ulid::new(),
        site_id: site.id,
        kind: AnalyticsAlertKind::NewReferrerSpike,
        config: AnalyticsAlertConfig {
            threshold: -1.0,
            window_minutes: 60,
        },
        enabled: true,
        created_at: Utc::now(),
    };
    ctx.backend
        .meta
        .create_analytics_alert(&alert)
        .await
        .unwrap();

    let backend: Arc<dyn stomatopod_core::traits::StorageBackend> = ctx.backend.clone();
    let meta: Arc<dyn stomatopod_core::traits::MetaStore> = ctx.backend.clone();
    let client = reqwest::Client::new();
    let webhook = WebhookSink::new(client.clone());
    let slack = SlackSink::new(client.clone());
    let telegram = TelegramSink::new(client);
    let now = Utc::now();

    // First evaluation fires + records. Loopback destinations are blocked by
    // SSRF checks, so delivery itself must not succeed - we only assert the
    // fire was recorded and cooldown engages.
    let fired = process_alert(&alert, &backend, &meta, &webhook, &slack, &telegram, now).await;
    assert!(fired, "negative-threshold referrer alert should fire");
    let last = ctx
        .backend
        .meta
        .last_analytics_alert_fire(alert.id)
        .await
        .unwrap();
    assert!(last.is_some(), "fire should be recorded");

    // Give any accidental dispatch a moment; loopback must stay undelivered.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        *received.lock().unwrap(),
        0,
        "loopback webhook must be blocked by SSRF checks"
    );

    // Second evaluation within the hour is suppressed by cooldown.
    let again = process_alert(&alert, &backend, &meta, &webhook, &slack, &telegram, now).await;
    assert!(!again, "cooldown should suppress a re-fire within the hour");
}

// ---- Tier-3 analytics endpoints: campaigns ----

#[tokio::test]
async fn api_campaigns_returns_utm_breakdowns() {
    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    let (status, json) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/campaigns", site.id),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for dim in [
        "utm_source",
        "utm_medium",
        "utm_campaign",
        "utm_term",
        "utm_content",
    ] {
        assert!(
            json.get(dim).and_then(|d| d.get("rows")).is_some(),
            "campaigns response should include {dim}.rows, got {json}"
        );
    }
}

// ============================================================================
// Analytics digest (channel delivery)
// ============================================================================

/// Create org + persisted user + site, returning the site and a session
/// token whose principal resolves to that real user (so digest endpoints
/// can identify the current user).
async fn site_user_and_token(ctx: &TestCtx) -> (Site, stomatopod_core::domain::org::User) {
    use stomatopod_core::domain::org::{User, UserRole};
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();
    let user = User {
        id: Ulid::new(),
        org_id: org.id,
        email: format!("user-{}@example.com", Ulid::new()),
        password_hash: "x".into(),
        role: UserRole::Owner,
        created_at: Utc::now(),
    };
    ctx.backend.meta.create_user(&user).await.unwrap();
    (site, user)
}

async fn add_test_channel(ctx: &TestCtx, site_id: Ulid) {
    use stomatopod_core::domain::alert_channel::{AlertChannel, AlertChannelKind};
    let ch = AlertChannel {
        id: Ulid::new(),
        site_id,
        kind: AlertChannelKind::Webhook,
        url: "https://example.com/hooks/digest".into(),
        secret: None,
        created_at: Utc::now(),
        last_error_at: None,
    };
    ctx.backend.meta.create_alert_channel(&ch).await.unwrap();
}

#[tokio::test]
async fn digest_subscription_crud_and_test_send() {
    let ctx = setup().await;
    let (site, user) = site_user_and_token(&ctx).await;
    let token = sign_session(&ctx.secret, &user.id.to_string(), 3600);

    // No subscription initially.
    let (status, json) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/digest-subscription", site.id),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["subscription"].is_null());

    // Bad frequency rejected.
    let (status, _) = send_json(
        ctx.state.clone(),
        "PUT",
        &format!("/api/v1/sites/{}/digest-subscription", site.id),
        &token,
        Some(serde_json::json!({"frequency": "hourly"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Create.
    let (status, json) = send_json(
        ctx.state.clone(),
        "PUT",
        &format!("/api/v1/sites/{}/digest-subscription", site.id),
        &token,
        Some(serde_json::json!({"frequency": "weekly"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create sub, got {json}");
    assert_eq!(json["frequency"], "weekly");

    // Test-send without a channel fails.
    let (status, json) = send_json(
        ctx.state.clone(),
        "POST",
        &format!("/api/v1/sites/{}/digest-subscription/test", site.id),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        json["error"]
            .as_str()
            .unwrap_or_default()
            .contains("notification destination"),
        "expected channel-required error, got {json}"
    );

    // With a channel, test-send delivers one digest message.
    add_test_channel(&ctx, site.id).await;
    let (status, _) = send_json(
        ctx.state.clone(),
        "POST",
        &format!("/api/v1/sites/{}/digest-subscription/test", site.id),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let sent = ctx.digest_sink.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "one digest captured");
    assert_eq!(sent[0].site_id, site.id);
    assert!(sent[0].subject.contains(&site.domain));
    assert!(sent[0].text.contains("Dashboard:"));

    // Delete unsubscribes.
    let (status, _) = send_json(
        ctx.state.clone(),
        "DELETE",
        &format!("/api/v1/sites/{}/digest-subscription", site.id),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, json) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/digest-subscription", site.id),
        &token,
    )
    .await;
    assert!(json["subscription"].is_null());
}

#[tokio::test]
async fn digest_scheduler_dispatches_weekly_to_subscribers() {
    use stomatopod_core::{
        domain::digest::{DigestFrequency, DigestSubscription},
        traits::{MetaStore, StorageBackend},
    };
    let ctx = setup().await;
    let (site, user) = site_user_and_token(&ctx).await;
    add_test_channel(&ctx, site.id).await;

    let sub = DigestSubscription {
        id: Ulid::new(),
        user_id: user.id,
        site_id: site.id,
        frequency: DigestFrequency::Weekly,
        enabled: true,
        bounce_count: 0,
        created_at: Utc::now(),
    };
    ctx.backend
        .meta
        .upsert_digest_subscription(&sub)
        .await
        .unwrap();

    let sink = CapturingNotifier::default();
    let meta: Arc<dyn MetaStore> = ctx.backend.clone();
    let backend: Arc<dyn StorageBackend> = ctx.backend.clone();
    let notifier: Arc<dyn stomatopod_web::digest::DigestNotifier> = Arc::new(sink.clone());

    // Weekly cadence reaches the weekly subscriber.
    let n = stomatopod_web::digest::dispatch_cadence(
        &meta,
        &backend,
        &notifier,
        "http://localhost:8080",
        DigestFrequency::Weekly,
    )
    .await;
    assert_eq!(n, 1, "one weekly digest dispatched");
    let sent = sink.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].site_id, site.id);
    assert!(sent[0].subject.contains(&site.domain));

    // Monthly cadence skips a weekly-only subscriber.
    let n = stomatopod_web::digest::dispatch_cadence(
        &meta,
        &backend,
        &notifier,
        "http://localhost:8080",
        DigestFrequency::Monthly,
    )
    .await;
    assert_eq!(n, 0, "monthly cadence skips weekly-only subscriber");
}

#[tokio::test]
async fn digest_scheduler_skips_site_without_channels() {
    use stomatopod_core::{
        domain::digest::{DigestFrequency, DigestSubscription},
        traits::{MetaStore, StorageBackend},
    };
    let ctx = setup().await;
    let (site, user) = site_user_and_token(&ctx).await;

    let sub = DigestSubscription {
        id: Ulid::new(),
        user_id: user.id,
        site_id: site.id,
        frequency: DigestFrequency::Weekly,
        enabled: true,
        bounce_count: 0,
        created_at: Utc::now(),
    };
    ctx.backend
        .meta
        .upsert_digest_subscription(&sub)
        .await
        .unwrap();

    // Production-like notifier that requires channels.
    let notifier: Arc<dyn stomatopod_web::digest::DigestNotifier> =
        Arc::new(stomatopod_web::digest::ChannelNotifier);
    let meta: Arc<dyn MetaStore> = ctx.backend.clone();
    let backend: Arc<dyn StorageBackend> = ctx.backend.clone();
    let n = stomatopod_web::digest::dispatch_cadence(
        &meta,
        &backend,
        &notifier,
        "http://localhost:8080",
        DigestFrequency::Weekly,
    )
    .await;
    assert_eq!(n, 0, "no channels => no successful dispatch");
}

// ============================================================================
// P0 integration coverage: full data plane, funnels with data, API key HTTP
// CRUD, login rate limit, ingest CORS.
// ============================================================================

const CHROME_UA: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

async fn post_browser_event(
    state: Arc<AppState>,
    site_key: &str,
    name: &str,
    url: &str,
) -> StatusCode {
    let payload = serde_json::json!({
        "k": site_key,
        "n": name,
        "u": url,
        "w": 1920,
        "h": 1080,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/event")
        .header("content-type", "application/json")
        .header("user-agent", CHROME_UA)
        .body(Body::from(payload.to_string()))
        .unwrap();
    make_app(state).oneshot(req).await.unwrap().status()
}

/// Poll pageviews until `total_pageviews >= want` or timeout.
async fn wait_for_pageviews(
    state: Arc<AppState>,
    site_id: Ulid,
    token: &str,
    want: u64,
) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut last = serde_json::Value::Null;
    while tokio::time::Instant::now() < deadline {
        let (status, json) = get_json(
            state.clone(),
            &format!("/api/v1/sites/{site_id}/pageviews?range=7d"),
            token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "pageviews poll failed: {json}");
        last = json;
        let got = last["total_pageviews"].as_u64().unwrap_or(0);
        if got >= want {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {want} pageviews; last={last}");
}

// ---- P0.1: ingest → batcher → parquet → analytics ----

#[tokio::test]
async fn ingest_round_trip_pageviews_and_top_pages() {
    let ctx = setup_for_data().await;
    let (site, token) = site_and_token(&ctx).await;

    for path in ["/home", "/home", "/pricing"] {
        let status = post_browser_event(
            ctx.state.clone(),
            &site.public_key,
            "pageview",
            &format!("https://{domain}{path}", domain = site.domain),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "ingest {path}");
    }

    let pv = wait_for_pageviews(ctx.state.clone(), site.id, &token, 3).await;
    assert_eq!(pv["total_pageviews"].as_u64().unwrap(), 3);
    assert!(
        pv["total_sessions"].as_u64().unwrap() >= 1,
        "sessions should be derived: {pv}"
    );

    let (status, top) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/top-pages?range=7d", site.id),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = top["rows"].as_array().expect("rows");
    let total: u64 = rows
        .iter()
        .map(|r| r["pageviews"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(
        total, 3,
        "top-pages should sum to ingested pageviews: {top}"
    );
    // Top-pages `value` is the full URL as stored on the event.
    let home_url = format!("https://{}/home", site.domain);
    let home = rows
        .iter()
        .find(|r| r["value"].as_str() == Some(home_url.as_str()));
    assert!(home.is_some(), "expected {home_url} in top pages: {top}");
    assert_eq!(home.unwrap()["pageviews"].as_u64().unwrap(), 2);
}

#[tokio::test]
async fn ingest_bot_user_agent_does_not_create_pageviews() {
    let ctx = setup_for_data().await;
    let (site, token) = site_and_token(&ctx).await;

    // Real browser event first so we know the pipeline works.
    assert_eq!(
        post_browser_event(
            ctx.state.clone(),
            &site.public_key,
            "pageview",
            "https://test.example.com/ok",
        )
        .await,
        StatusCode::NO_CONTENT
    );
    wait_for_pageviews(ctx.state.clone(), site.id, &token, 1).await;

    // Bot: 204 (silent drop) and counts stay at 1.
    let payload = serde_json::json!({
        "k": site.public_key,
        "n": "pageview",
        "u": "https://test.example.com/bot",
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

    tokio::time::sleep(Duration::from_millis(500)).await;
    let (status, json) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/pageviews?range=7d", site.id),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json["total_pageviews"].as_u64().unwrap(),
        1,
        "bot traffic must not land in analytics: {json}"
    );
}

#[tokio::test]
async fn ingest_utm_url_shows_up_in_utm_breakdown() {
    let ctx = setup_for_data().await;
    let (site, token) = site_and_token(&ctx).await;

    assert_eq!(
        post_browser_event(
            ctx.state.clone(),
            &site.public_key,
            "pageview",
            "https://test.example.com/lp?utm_source=newsletter&utm_medium=email&utm_campaign=spring",
        )
        .await,
        StatusCode::NO_CONTENT
    );
    wait_for_pageviews(ctx.state.clone(), site.id, &token, 1).await;

    let (status, json) = get_json(
        ctx.state.clone(),
        &format!(
            "/api/v1/sites/{}/utm?range=7d&dimension=source&limit=20",
            site.id
        ),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let rows = json["rows"].as_array().expect("rows");
    assert!(
        rows.iter()
            .any(|r| r["value"].as_str() == Some("newsletter")),
        "utm_source=newsletter should appear: {json}"
    );
}

#[tokio::test]
async fn key_ingest_custom_event_appears_in_events_api() {
    let ctx = setup_for_data().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();
    let token = sign_session(&ctx.secret, &Ulid::new().to_string(), 3600);

    let (key, plaintext) =
        stomatopod_core::domain::api_key::ApiKey::new_ingest(org.id, site.id, "backend".into());
    ctx.backend.meta.create_api_key(&key).await.unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/ingest")
        .header("authorization", format!("Bearer {plaintext}"))
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"name":"signup","properties":{"plan":"pro"},"session_id":"user-42"}"#,
        ))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let (status, json) = get_json(
            ctx.state.clone(),
            &format!("/api/v1/sites/{}/events?range=7d", site.id),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let rows = json["rows"].as_array().cloned().unwrap_or_default();
        // Custom-events breakdown uses TopList rows (`value` = event name).
        if rows.iter().any(|r| r["value"].as_str() == Some("signup")) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for signup event; last={json}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ---- P0.2: funnel evaluation with real events ----

#[tokio::test]
async fn funnel_result_reports_step_conversion_from_ingested_events() {
    let ctx = setup_for_data().await;
    let (site, token) = site_and_token(&ctx).await;

    // Three sessions hit pageview; two of them also fire signup.
    for (i, also_signup) in [(0, true), (1, true), (2, false)] {
        // Distinct sessions: cookieless session = hash(site, ip, ua, day).
        // Vary UA slightly so each loop is its own session.
        let ua = format!("{CHROME_UA} Session/{i}");
        let pv = serde_json::json!({
            "k": site.public_key,
            "n": "pageview",
            "u": format!("https://{}/start", site.domain),
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/event")
            .header("content-type", "application/json")
            .header("user-agent", &ua)
            .body(Body::from(pv.to_string()))
            .unwrap();
        assert_eq!(
            make_app(ctx.state.clone())
                .oneshot(req)
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
        if also_signup {
            let su = serde_json::json!({
                "k": site.public_key,
                "n": "signup",
                "u": format!("https://{}/thanks", site.domain),
            });
            let req = Request::builder()
                .method("POST")
                .uri("/api/v1/event")
                .header("content-type", "application/json")
                .header("user-agent", &ua)
                .body(Body::from(su.to_string()))
                .unwrap();
            assert_eq!(
                make_app(ctx.state.clone())
                    .oneshot(req)
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NO_CONTENT
            );
        }
    }

    wait_for_pageviews(ctx.state.clone(), site.id, &token, 3).await;

    let create_body = serde_json::json!({
        "name": "Signup",
        "steps": [
            {"name": "Landing", "event_name": "pageview", "filters": []},
            {"name": "Signed up", "event_name": "signup", "filters": []}
        ]
    });
    let (status, created) = send_json(
        ctx.state.clone(),
        "POST",
        &format!("/api/v1/sites/{}/funnels", site.id),
        &token,
        Some(create_body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let funnel_id = created["id"].as_str().expect("funnel id");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let result = loop {
        let (status, json) = get_json(
            ctx.state.clone(),
            &format!("/api/v1/sites/{}/funnels/{funnel_id}?range=7d", site.id),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let steps = json["steps"].as_array().cloned().unwrap_or_default();
        if steps.len() == 2 {
            let s0 = steps[0]["sessions"].as_u64().unwrap_or(0);
            let s1 = steps[1]["sessions"].as_u64().unwrap_or(0);
            if s0 >= 3 && s1 >= 2 {
                break json;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for funnel counts; last={json}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };

    let steps = result["steps"].as_array().unwrap();
    assert_eq!(steps[0]["name"].as_str().unwrap(), "Landing");
    assert_eq!(steps[0]["sessions"].as_u64().unwrap(), 3);
    assert!((steps[0]["conversion_rate"].as_f64().unwrap() - 1.0).abs() < f64::EPSILON);
    assert_eq!(steps[1]["name"].as_str().unwrap(), "Signed up");
    assert_eq!(steps[1]["sessions"].as_u64().unwrap(), 2);
    let cr = steps[1]["conversion_rate"].as_f64().unwrap();
    assert!((cr - (2.0 / 3.0)).abs() < 0.01, "conversion_rate={cr}");
}

// ---- P0.4: API key HTTP CRUD + cache eviction ----

#[tokio::test]
async fn api_keys_http_crud_and_cache_eviction() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();
    let token = sign_session(&ctx.secret, &Ulid::new().to_string(), 3600);

    // Mint an ingest key via HTTP.
    let (status, created) = send_json(
        ctx.state.clone(),
        "POST",
        "/api/v1/keys",
        &token,
        Some(serde_json::json!({
            "name": "prod-ingest",
            "scope": "ingest",
            "site_id": site.id.to_string(),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let key_id = created["id"].as_str().expect("id").to_string();
    let secret = created["secret"].as_str().expect("secret once").to_string();
    assert!(
        secret.starts_with("sk_live_"),
        "ingest key prefix: {secret}"
    );
    assert_eq!(created["scope"].as_str().unwrap(), "ingest");

    // Ingest key without site_id is rejected.
    let (status, err) = send_json(
        ctx.state.clone(),
        "POST",
        "/api/v1/keys",
        &token,
        Some(serde_json::json!({
            "name": "bad",
            "scope": "ingest",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");

    // List includes the key (no secret).
    let (status, listed) = get_json(ctx.state.clone(), "/api/v1/keys", &token).await;
    assert_eq!(status, StatusCode::OK);
    let keys = listed["keys"].as_array().expect("keys");
    assert!(
        keys.iter()
            .any(|k| k["id"].as_str() == Some(key_id.as_str())),
        "minted key should appear: {listed}"
    );
    assert!(
        keys.iter().all(|k| k.get("secret").is_none()),
        "list must never echo secrets"
    );

    // Key works for server ingest.
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/ingest")
        .header("authorization", format!("Bearer {secret}"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"ping"}"#))
        .unwrap();
    assert_eq!(
        make_app(ctx.state.clone())
            .oneshot(req)
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );

    // Populate the api_key_cache with a second successful call.
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/ingest")
        .header("authorization", format!("Bearer {secret}"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"ping2"}"#))
        .unwrap();
    assert_eq!(
        make_app(ctx.state.clone())
            .oneshot(req)
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );

    // Revoke via HTTP.
    let (status, _) = send_json(
        ctx.state.clone(),
        "DELETE",
        &format!("/api/v1/keys/{key_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Cache must be evicted: key no longer authorizes.
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/ingest")
        .header("authorization", format!("Bearer {secret}"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"after-revoke"}"#))
        .unwrap();
    assert_eq!(
        make_app(ctx.state.clone())
            .oneshot(req)
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED,
        "revoked key must fail even if previously cached"
    );

    // Read key cannot mint keys (dashboard-only).
    let (read_key, read_plain) =
        stomatopod_core::domain::api_key::ApiKey::new_read(org.id, None, "agent".into());
    ctx.backend.meta.create_api_key(&read_key).await.unwrap();
    let (status, _) = send_json(
        ctx.state.clone(),
        "POST",
        "/api/v1/keys",
        &read_plain,
        Some(serde_json::json!({
            "name": "nope",
            "scope": "read",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Site-scoped mint works.
    let (status, site_key) = send_json(
        ctx.state.clone(),
        "POST",
        &format!("/api/v1/sites/{}/keys", site.id),
        &token,
        Some(serde_json::json!({
            "name": "site-read",
            "scope": "read",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{site_key}");
    assert!(site_key["secret"].as_str().unwrap_or("").starts_with("rk_"));
}

// ---- P0.5: login rate limiting ----

#[tokio::test]
async fn login_rate_limited_after_repeated_failures() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let password = "correct-password-12";
    let hash = hash_password(password);
    let user = User {
        id: Ulid::new(),
        org_id: org.id,
        email: "ratelimit@example.com".into(),
        password_hash: hash,
        role: UserRole::Owner,
        created_at: Utc::now(),
    };
    ctx.backend.meta.create_user(&user).await.unwrap();

    // 10 failures fill the window (LOGIN_MAX_FAILURES).
    for i in 0..10 {
        let body = "email=ratelimit%40example.com&password=wrong-password";
        let req = Request::builder()
            .method("POST")
            .uri("/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "failure {i}");
        let html = String::from_utf8(body_bytes(resp).await.to_vec()).unwrap();
        assert!(
            html.contains("Invalid credentials"),
            "attempt {i} should be invalid credentials, not rate limit yet"
        );
    }

    // 11th attempt (even with the correct password) is blocked.
    let body = "email=ratelimit%40example.com&password=correct-password-12";
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = String::from_utf8(body_bytes(resp).await.to_vec()).unwrap();
    assert!(
        html.contains("Too many login attempts"),
        "expected rate-limit message, got: {html}"
    );
}

// ---- P0.6: ingest CORS ----

#[tokio::test]
async fn ingest_cors_preflight_and_post_reflect_origin() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let origin = "https://customer-site.example";

    // Preflight
    let req = Request::builder()
        .method("OPTIONS")
        .uri("/api/v1/event")
        .header("origin", origin)
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type")
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert!(
        resp.status() == StatusCode::OK
            || resp.status() == StatusCode::NO_CONTENT
            || resp.status() == StatusCode::ACCEPTED,
        "preflight status: {}",
        resp.status()
    );
    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(acao, origin, "must mirror Origin for credentialed beacons");
    let acac = resp
        .headers()
        .get("access-control-allow-credentials")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(acac, "true");

    // Actual POST carries the same CORS headers.
    let payload = serde_json::json!({
        "k": site.public_key,
        "n": "pageview",
        "u": "https://customer-site.example/",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/event")
        .header("origin", origin)
        .header("content-type", "application/json")
        .header("user-agent", CHROME_UA)
        .body(Body::from(payload.to_string()))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(acao, origin);
    let acac = resp
        .headers()
        .get("access-control-allow-credentials")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(acac, "true");
}

#[tokio::test]
async fn tracker_js_cors_allows_cross_origin_get() {
    let ctx = setup().await;
    let origin = "https://blog.example";
    let req = Request::builder()
        .method("GET")
        .uri("/tracker.js")
        .header("origin", origin)
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(acao, origin, "tracker.js is loaded cross-origin");
}

// ============================================================================
// P1 integration coverage: alert channels, ingest edges, site lifecycle,
// cookie auth, password validation, analytics with data, bootstrap.
// ============================================================================

// ---- P1.7 / helpers: seed a few browser pageviews for dimension tests ----

async fn seed_browser_pageviews(ctx: &TestCtx, site: &Site, n: usize) {
    for i in 0..n {
        let status = post_browser_event(
            ctx.state.clone(),
            &site.public_key,
            "pageview",
            &format!("https://{}/p{i}", site.domain),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "seed pageview {i}");
    }
}

// ---- P1: alert channels over HTTP ----

#[tokio::test]
async fn alert_channels_http_ssrf_and_validation() {
    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;
    let base = format!("/api/v1/sites/{}/alert-channels", site.id);

    // Empty destination.
    let (status, err) = send_json(
        ctx.state.clone(),
        "POST",
        &base,
        &token,
        Some(serde_json::json!({"kind": "webhook", "url": "  "})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");

    // Loopback blocked at the API boundary.
    for url in [
        "http://127.0.0.1/hook",
        "http://localhost/hook",
        "http://169.254.169.254/latest/meta-data",
        "http://10.0.0.1/internal",
    ] {
        let (status, err) = send_json(
            ctx.state.clone(),
            "POST",
            &base,
            &token,
            Some(serde_json::json!({"kind": "webhook", "url": url})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "url={url} err={err}");
        let msg = err["error"].as_str().unwrap_or("");
        assert!(
            msg.contains("invalid destination") || msg.contains("not"),
            "ssrf message for {url}: {err}"
        );
    }

    // Slack uses the same SSRF path.
    let (status, err) = send_json(
        ctx.state.clone(),
        "POST",
        &base,
        &token,
        Some(serde_json::json!({
            "kind": "slack",
            "url": "http://127.0.0.1/services/T/B/X"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");

    // Telegram requires a bot token; chat id skips SSRF.
    let (status, err) = send_json(
        ctx.state.clone(),
        "POST",
        &base,
        &token,
        Some(serde_json::json!({
            "kind": "telegram",
            "url": "-1001234567890"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
    assert!(
        err["error"].as_str().unwrap_or("").contains("bot token"),
        "{err}"
    );

    // Public HTTPS webhook is accepted.
    let (status, created) = send_json(
        ctx.state.clone(),
        "POST",
        &base,
        &token,
        Some(serde_json::json!({
            "kind": "webhook",
            "url": "https://example.com/hooks/stomatopod",
            "secret": "signing-secret"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["kind"], "webhook");
    assert!(created.get("secret").is_none(), "secret must not leak");
    let channel_id = created["id"].as_str().expect("id").to_string();

    // Telegram with token is accepted (chat id is not an HTTP URL).
    let (status, tg) = send_json(
        ctx.state.clone(),
        "POST",
        &base,
        &token,
        Some(serde_json::json!({
            "kind": "telegram",
            "url": "-1001234567890",
            "secret": "123456:ABC-DEF"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{tg}");

    // List returns both; no secrets.
    let (status, listed) = get_json(ctx.state.clone(), &base, &token).await;
    assert_eq!(status, StatusCode::OK);
    let channels = listed["channels"].as_array().expect("channels");
    assert!(channels.len() >= 2, "{listed}");
    assert!(channels.iter().all(|c| c.get("secret").is_none()));

    // Delete webhook channel.
    let (status, _) = send_json(
        ctx.state.clone(),
        "DELETE",
        &format!("{base}/{channel_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Read API key cannot manage channels.
    let org = ctx.backend.meta.list_orgs().await.unwrap()[0].clone();
    let (read_key, read_plain) =
        stomatopod_core::domain::api_key::ApiKey::new_read(org.id, None, "agent".into());
    ctx.backend.meta.create_api_key(&read_key).await.unwrap();
    let (status, _) = send_json(
        ctx.state.clone(),
        "POST",
        &base,
        &read_plain,
        Some(serde_json::json!({
            "kind": "webhook",
            "url": "https://example.com/hooks/nope"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn alert_channel_test_fire_reports_ssrf_failure() {
    use stomatopod_core::domain::alert_channel::{AlertChannel, AlertChannelKind};

    let ctx = setup().await;
    let (site, token) = site_and_token(&ctx).await;

    // Insert a loopback channel via meta (bypasses create-time SSRF) so
    // test-fire exercises dispatch + error reporting.
    let channel = AlertChannel {
        id: Ulid::new(),
        site_id: site.id,
        kind: AlertChannelKind::Webhook,
        url: "http://127.0.0.1:9/hook".into(),
        secret: None,
        created_at: Utc::now(),
        last_error_at: None,
    };
    ctx.backend
        .meta
        .create_alert_channel(&channel)
        .await
        .unwrap();

    let (status, json) = send_json(
        ctx.state.clone(),
        "POST",
        &format!(
            "/api/v1/sites/{}/alert-channels/{}/test",
            site.id, channel.id
        ),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["result"], "fail", "{json}");
    let err = json["error"].as_str().unwrap_or("");
    assert!(
        err.contains("not publicly routable")
            || err.contains("not allowed")
            || err.contains("loopback")
            || err.contains("resolve"),
        "expected SSRF-ish error, got {json}"
    );
}

// ---- P1: ingest back-pressure and error edges ----

#[tokio::test]
async fn ingest_returns_429_when_channel_is_full() {
    // Capacity 1, no consumer: first event fills the channel, second is Full.
    let ctx = setup_ingest(100, 3600, 1, IngestMode::Hold).await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let status = post_browser_event(
        ctx.state.clone(),
        &site.public_key,
        "pageview",
        "https://test.example.com/1",
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "first event fills channel");

    let status = post_browser_event(
        ctx.state.clone(),
        &site.public_key,
        "pageview",
        "https://test.example.com/2",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "full channel must surface as 429"
    );
}

#[tokio::test]
async fn ingest_returns_503_when_channel_is_closed() {
    let ctx = setup_ingest(100, 3600, 1, IngestMode::Closed).await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();

    let status = post_browser_event(
        ctx.state.clone(),
        &site.public_key,
        "pageview",
        "https://test.example.com/",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn key_ingest_rejects_empty_event_name() {
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
        .body(Body::from(r#"{"name":""}"#))
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ---- P1: site resolution, deactivate + cache, create validation ----

#[tokio::test]
async fn analytics_resolves_site_by_domain() {
    let ctx = setup_for_data().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site_with_domain(org.id, "by-domain.example.com");
    ctx.backend.meta.create_site(&site).await.unwrap();
    let token = sign_session(&ctx.secret, &Ulid::new().to_string(), 3600);

    seed_browser_pageviews(&ctx, &site, 2).await;
    wait_for_pageviews(ctx.state.clone(), site.id, &token, 2).await;

    let (status, json) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/pageviews?range=7d", site.domain),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["total_pageviews"].as_u64().unwrap(), 2);
}

#[tokio::test]
async fn deactivate_site_evicts_cache_and_rejects_ingest() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();
    let token = sign_session(&ctx.secret, &Ulid::new().to_string(), 3600);

    // Warm site_cache via a successful ingest.
    assert_eq!(
        post_browser_event(
            ctx.state.clone(),
            &site.public_key,
            "pageview",
            "https://test.example.com/",
        )
        .await,
        StatusCode::NO_CONTENT
    );

    // Deactivate via HTTP patch (must evict cache).
    let (status, patched) = send_json(
        ctx.state.clone(),
        "PATCH",
        &format!("/api/v1/sites/{}", site.id),
        &token,
        Some(serde_json::json!({"is_active": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["is_active"], false);

    // Ingest with the same public key must now fail (meta filters inactive).
    let status = post_browser_event(
        ctx.state.clone(),
        &site.public_key,
        "pageview",
        "https://test.example.com/after-deactivate",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "inactive site must not accept beacons"
    );
}

#[tokio::test]
async fn create_site_requires_domain_and_name() {
    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let token = sign_session(&ctx.secret, &Ulid::new().to_string(), 3600);

    for body in [
        serde_json::json!({"domain": "", "name": "Ok"}),
        serde_json::json!({"domain": "x.example", "name": ""}),
        serde_json::json!({"domain": "  ", "name": "  "}),
    ] {
        let (status, err) = send_json(
            ctx.state.clone(),
            "POST",
            "/api/v1/sites",
            &token,
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body} err={err}");
    }

    let (status, created) = send_json(
        ctx.state.clone(),
        "POST",
        "/api/v1/sites",
        &token,
        Some(serde_json::json!({
            "domain": "fresh.example.com",
            "name": "Fresh"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["domain"], "fresh.example.com");
    assert!(created["public_key"].as_str().unwrap().len() >= 16);
}

// ---- P1: session cookie auth for JSON API ----

#[tokio::test]
async fn session_cookie_authorizes_json_api() {
    use stomatopod_web::middleware::auth::SESSION_COOKIE;

    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();
    let token = sign_session(&ctx.secret, &Ulid::new().to_string(), 3600);

    // Cookie only (no Authorization header) — Principal::Session path.
    let req = Request::builder()
        .uri("/api/v1/sites")
        .header("cookie", format!("{SESSION_COOKIE}={token}"))
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert!(!json["sites"].as_array().unwrap().is_empty());

    // Tampered cookie is rejected.
    let req = Request::builder()
        .uri("/api/v1/sites")
        .header("cookie", format!("{SESSION_COOKIE}=not-a-real-session"))
        .body(Body::empty())
        .unwrap();
    let resp = make_app(ctx.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ---- P1: password change validation edges ----

#[tokio::test]
async fn change_password_rejects_weak_and_oversized() {
    use stomatopod_web::middleware::auth::sign_session_bound;

    let ctx = setup().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let password = "long-enough-password";
    let hash = hash_password(password);
    let user = User {
        id: Ulid::new(),
        org_id: org.id,
        email: "pwd@example.com".into(),
        password_hash: hash.clone(),
        role: UserRole::Owner,
        created_at: Utc::now(),
    };
    ctx.backend.meta.create_user(&user).await.unwrap();
    let token = sign_session_bound(&ctx.secret, &user.id.to_string(), 3600, &hash);

    // Too short.
    let (status, err) = send_json(
        ctx.state.clone(),
        "POST",
        "/api/v1/me/password",
        &token,
        Some(serde_json::json!({
            "current_password": password,
            "new_password": "short"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
    assert!(
        err["error"]
            .as_str()
            .unwrap_or("")
            .contains("12 characters"),
        "{err}"
    );

    // Oversize (DoS cap).
    let long = "x".repeat(129);
    let (status, err) = send_json(
        ctx.state.clone(),
        "POST",
        "/api/v1/me/password",
        &token,
        Some(serde_json::json!({
            "current_password": password,
            "new_password": long
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
    assert!(
        err["error"]
            .as_str()
            .unwrap_or("")
            .contains("128 characters"),
        "{err}"
    );
}

// ---- P1: analytics routes with non-zero data ----

#[tokio::test]
async fn analytics_dimension_routes_return_data_after_ingest() {
    let ctx = setup_for_data().await;
    let (site, token) = site_and_token(&ctx).await;
    seed_browser_pageviews(&ctx, &site, 3).await;
    wait_for_pageviews(ctx.state.clone(), site.id, &token, 3).await;

    for dim in [
        "top-browsers",
        "top-devices",
        "top-countries",
        "top-os",
        "top-pages",
    ] {
        let (status, json) = get_json(
            ctx.state.clone(),
            &format!("/api/v1/sites/{}/{dim}?range=7d", site.id),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{dim}: {json}");
        let rows = json["rows"].as_array().expect("rows");
        let total: u64 = rows
            .iter()
            .map(|r| r["pageviews"].as_u64().unwrap_or(0))
            .sum();
        assert!(total >= 1, "{dim} should reflect ingested traffic: {json}");
    }

    // Compare attaches a prior-window block even with live data.
    let (status, json) = get_json(
        ctx.state.clone(),
        &format!("/api/v1/sites/{}/pageviews?range=7d&compare=1", site.id),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("comparison").is_some(), "{json}");
    assert!(json["total_pageviews"].as_u64().unwrap() >= 3);

    // Filter that matches Chrome (our CHROME_UA) still returns rows.
    let (status, json) = get_json(
        ctx.state.clone(),
        &format!(
            "/api/v1/sites/{}/top-pages?range=7d&filter=browser:eq:Chrome",
            site.id
        ),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert!(
        !json["rows"].as_array().unwrap().is_empty(),
        "Chrome filter should keep our seeded rows: {json}"
    );
}

#[tokio::test]
async fn events_api_filters_by_event_name_after_key_ingest() {
    let ctx = setup_for_data().await;
    let org = make_org();
    ctx.backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    ctx.backend.meta.create_site(&site).await.unwrap();
    let token = sign_session(&ctx.secret, &Ulid::new().to_string(), 3600);
    let (key, plaintext) =
        stomatopod_core::domain::api_key::ApiKey::new_ingest(org.id, site.id, "backend".into());
    ctx.backend.meta.create_api_key(&key).await.unwrap();

    for name in ["signup", "signup", "purchase"] {
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/ingest")
            .header("authorization", format!("Bearer {plaintext}"))
            .header("content-type", "application/json")
            .body(Body::from(format!(r#"{{"name":"{name}"}}"#)))
            .unwrap();
        assert_eq!(
            make_app(ctx.state.clone())
                .oneshot(req)
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let json = loop {
        let (status, json) = get_json(
            ctx.state.clone(),
            &format!("/api/v1/sites/{}/events?range=7d&name=signup", site.id),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let rows = json["rows"].as_array().cloned().unwrap_or_default();
        if rows.iter().any(|r| r["value"].as_str() == Some("signup")) {
            break json;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for signup events: {json}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let rows = json["rows"].as_array().unwrap();
    assert!(
        rows.iter().all(|r| r["value"].as_str() != Some("purchase")),
        "name=signup must exclude purchase: {json}"
    );
}

// ---- P1: first-boot bootstrap ----

#[tokio::test]
async fn bootstrap_self_hosted_creates_admin_and_is_idempotent() {
    use stomatopod_core::config::Config;
    use stomatopod_web::server::bootstrap_self_hosted;

    let dir = tempfile::tempdir().unwrap();
    let cfg_emb = EmbeddedConfig {
        data_dir: dir.path().to_path_buf(),
        allow_ephemeral: true,
        ..Default::default()
    };
    let backend = Arc::new(EmbeddedBackend::open(&cfg_emb).await.unwrap());
    let meta: Arc<dyn MetaStore> = backend.clone();
    let config = Config::default();

    // Missing password fails on empty DB.
    std::env::remove_var("STOMATOPOD_ADMIN_PASSWORD");
    std::env::remove_var("STOMATOPOD_ADMIN_EMAIL");
    let err = bootstrap_self_hosted(&meta, &config)
        .await
        .expect_err("missing password");
    assert!(
        err.to_string().contains("STOMATOPOD_ADMIN_PASSWORD"),
        "{err}"
    );

    // Short password fails.
    std::env::set_var("STOMATOPOD_ADMIN_PASSWORD", "tooshort");
    let err = bootstrap_self_hosted(&meta, &config)
        .await
        .expect_err("short password");
    assert!(err.to_string().contains("at least"), "{err}");

    // Happy path.
    std::env::set_var("STOMATOPOD_ADMIN_PASSWORD", "bootstrap-password-ok");
    std::env::set_var("STOMATOPOD_ADMIN_EMAIL", "owner@bootstrap.test");
    bootstrap_self_hosted(&meta, &config)
        .await
        .expect("first boot");

    let orgs = meta.list_orgs().await.unwrap();
    assert_eq!(orgs.len(), 1);
    let user = meta
        .get_user_by_email("owner@bootstrap.test")
        .await
        .unwrap()
        .expect("admin user");
    assert_eq!(user.org_id, orgs[0].id);

    // Second boot is a no-op even if env is cleared.
    std::env::remove_var("STOMATOPOD_ADMIN_PASSWORD");
    bootstrap_self_hosted(&meta, &config)
        .await
        .expect("idempotent");
    assert_eq!(meta.list_orgs().await.unwrap().len(), 1);

    std::env::remove_var("STOMATOPOD_ADMIN_EMAIL");
}
