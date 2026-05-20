use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::any,
    Router,
};
use sentinel::{
    client::{SessionRegistry, SpanRow, SpanShipper},
    config::{
        LimitsConfig, ListenConfig, PolicyConfig as PolicyCfg, RedactConfig, SentinelConfig,
        ServerConfig, UpstreamConfig,
    },
    control::{ControlCommand, ControlEnvelope, ControlState},
    policy::{PolicyConfig, PolicyEngine},
    proxy::ProxyState,
};
use tower::ServiceExt;

/// Spin up a minimal Anthropic-like server that returns a stub JSON
/// body with a known usage block.
async fn spawn_upstream() -> SocketAddr {
    let app = Router::new().route(
        "/v1/messages",
        axum::routing::post(|_req: Request<Body>| async {
            axum::Json(serde_json::json!({
                "id": "msg_abc",
                "type": "message",
                "role": "assistant",
                "model": "claude-opus-4-7",
                "content": [],
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": {
                    "input_tokens": 11,
                    "output_tokens": 22,
                    "cache_read_input_tokens": 0,
                    "cache_creation_input_tokens": 0
                }
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// Mock stomatopod that captures the spans the shipper POSTs to it.
async fn spawn_mock_stomatopod() -> (SocketAddr, Arc<Mutex<Vec<SpanRow>>>) {
    let captured: Arc<Mutex<Vec<SpanRow>>> = Arc::new(Mutex::new(Vec::new()));
    let store = captured.clone();
    let app = Router::new().route(
        "/api/v1/spans",
        axum::routing::post(move |axum::Json(payload): axum::Json<serde_json::Value>| {
            let store = store.clone();
            async move {
                if let Some(arr) = payload.get("spans").and_then(|s| s.as_array()) {
                    let mut s = store.lock().unwrap();
                    for v in arr {
                        if let Ok(row) = serde_json::from_value::<SpanRow>(v.clone()) {
                            s.push(row);
                        }
                    }
                }
                StatusCode::NO_CONTENT
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, captured)
}

fn build_state(upstream: SocketAddr, stomatopod: SocketAddr) -> Arc<ProxyState> {
    let cfg = Arc::new(SentinelConfig {
        listen: ListenConfig::default(),
        upstream: UpstreamConfig {
            url: format!("http://{}", upstream),
            vendor: "anthropic".into(),
        },
        server: ServerConfig {
            url: format!("http://{}", stomatopod),
            token: "test-token".into(),
            agent_id: Some("integration-agent".into()),
            site_id: ulid::Ulid::new().to_string(),
        },
        policy: PolicyCfg::default(),
        redact: RedactConfig::default(),
        limits: LimitsConfig::default(),
    });
    let http = reqwest::Client::new();
    let spool = std::env::temp_dir().join(format!("sentinel-e2e-{}", ulid::Ulid::new()));
    let shipper = SpanShipper::new(cfg.server.url.clone(), cfg.server.token.clone(), spool);
    let policy = Arc::new(PolicyEngine::new(PolicyConfig::default()));
    let control = ControlState::new();
    let sessions = Arc::new(SessionRegistry::default());
    sessions.set("integration-agent".into(), "sess-e2e".into());
    Arc::new(ProxyState {
        cfg,
        http,
        shipper,
        policy,
        control,
        sessions,
    })
}

#[tokio::test]
async fn anthropic_request_forwarded_and_span_shipped() {
    let upstream = spawn_upstream().await;
    let (stomatopod, captured) = spawn_mock_stomatopod().await;
    let state = build_state(upstream, stomatopod);
    let app = Router::new()
        .route("/*p", any(sentinel::proxy::handle))
        .with_state(state);

    let body = serde_json::json!({
        "model": "claude-opus-4-7",
        "max_tokens": 100,
        "messages": [{"role": "user", "content": "hi"}]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Span shipper batches with a 500ms interval; wait a beat.
    tokio::time::sleep(Duration::from_millis(800)).await;

    let spans = captured.lock().unwrap();
    assert_eq!(spans.len(), 1, "expected exactly one span captured");
    let s = &spans[0];
    assert_eq!(s.agent_id, "integration-agent");
    assert_eq!(s.input_tokens, 11);
    assert_eq!(s.output_tokens, 22);
    assert_eq!(s.stop_reason.as_deref(), Some("end_turn"));
    assert!(s.cost_usd > 0.0);
}

#[tokio::test]
async fn kill_switch_short_circuits_outbound_request() {
    let upstream = spawn_upstream().await;
    let (stomatopod, captured) = spawn_mock_stomatopod().await;
    let state = build_state(upstream, stomatopod);

    // Inject a kill via the control plane (simulating an SSE message).
    state.control.apply(ControlEnvelope {
        seq: 1,
        agent_id: "integration-agent".into(),
        command: ControlCommand::Kill {
            reason: "test-kill".into(),
        },
    });

    let app = Router::new()
        .route("/*p", any(sentinel::proxy::handle))
        .with_state(state);

    let body = serde_json::json!({
        "model": "claude-opus-4-7",
        "messages": [{"role": "user", "content": "should never reach upstream"}]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("x-sentinel-intervention")
            .and_then(|v| v.to_str().ok()),
        Some("killed_by_firewall")
    );

    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body_json["stop_reason"], "killed_by_firewall");
    // No span should have been shipped — the upstream was never called.
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(captured.lock().unwrap().len(), 0);
}
