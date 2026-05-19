use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use chrono::Utc;
use futures_util::StreamExt;
use serde_json::json;
use tracing::warn;

use crate::{
    client::{SessionRegistry, SpanRow, SpanShipper},
    config::SentinelConfig,
    control::ControlState,
    extractor::{
        parse_anthropic_request, parse_anthropic_response, parse_anthropic_stream_chunk,
        parse_openai_request, parse_openai_response, RequestSummary, ResponseSummary,
    },
    policy::{PolicyEngine, Trip},
    pricing::cost_usd,
};

#[derive(Clone)]
pub struct ProxyState {
    pub cfg: Arc<SentinelConfig>,
    pub http: reqwest::Client,
    pub shipper: Arc<SpanShipper>,
    pub policy: Arc<PolicyEngine>,
    pub control: Arc<ControlState>,
    pub sessions: Arc<SessionRegistry>,
}

/// Top-level handler: classifies the upstream by URL path, forwards the
/// request, parses tokens out, and emits a span.
pub async fn handle(State(s): State<Arc<ProxyState>>, req: Request) -> Response {
    let vendor = s.cfg.upstream.vendor.to_ascii_lowercase();
    match vendor.as_str() {
        "anthropic" => forward(s, req, Vendor::Anthropic).await,
        "openai" => forward(s, req, Vendor::OpenAI).await,
        other => {
            warn!("unknown upstream vendor: {other}");
            (StatusCode::BAD_GATEWAY, "unknown upstream vendor").into_response()
        }
    }
}

#[derive(Copy, Clone)]
enum Vendor {
    Anthropic,
    OpenAI,
}

async fn forward(s: Arc<ProxyState>, req: Request, vendor: Vendor) -> Response {
    let (parts, body) = req.into_parts();
    let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(b) => b,
        Err(e) => {
            warn!("failed to buffer request body: {e}");
            return (StatusCode::BAD_REQUEST, "bad request body").into_response();
        }
    };

    let req_summary = match vendor {
        Vendor::Anthropic => parse_anthropic_request(&body_bytes),
        Vendor::OpenAI => parse_openai_request(&body_bytes),
    };

    // ---- Pre-flight: kill switch ----
    if let Some(reason) = s.control.current_kill_reason() {
        return synthetic_kill_response(vendor, &reason);
    }

    // ---- Pre-flight: repetition detector ----
    for h in &req_summary.tool_input_hashes {
        if let Some(Trip::Repetition { count, args_hash }) = s.policy.observe_tool_call(h) {
            warn!(count, args_hash, "local repetition trip; killing");
            return synthetic_kill_response(
                vendor,
                &format!("repetition detected ({count}× {args_hash})"),
            );
        }
    }

    // ---- Compose upstream request ----
    let upstream_url = build_upstream_url(&s.cfg.upstream.url, &parts.uri);
    let mut headers = strip_hop_headers(&parts.headers);
    // Replace Host header (reqwest will set its own)
    headers.remove("host");
    // Inject hint as a system message if pending.
    let outgoing_body = if let Some(hint) = s.control.take_hint() {
        inject_hint(vendor, &body_bytes, &hint)
    } else {
        body_bytes.clone()
    };

    let started = Utc::now();
    let resp = match s
        .http
        .request(parts.method.clone(), &upstream_url)
        .headers(headers)
        .body(outgoing_body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("upstream request failed: {e}");
            return (StatusCode::BAD_GATEWAY, "upstream failure").into_response();
        }
    };
    let status = resp.status();
    let resp_headers = resp.headers().clone();

    // Streaming vs non-streaming branch.
    if req_summary.stream || is_event_stream(&resp_headers) {
        return stream_through(s, vendor, resp, status, resp_headers, req_summary, started).await;
    }

    // Buffer non-streaming response for token parsing.
    let resp_bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            warn!("upstream body read failed: {e}");
            return (StatusCode::BAD_GATEWAY, "upstream body read failed").into_response();
        }
    };
    let summary = match vendor {
        Vendor::Anthropic => parse_anthropic_response(&resp_bytes),
        Vendor::OpenAI => parse_openai_response(&resp_bytes),
    };
    emit_span(&s, vendor, &req_summary, &summary, started);

    let mut builder = Response::builder().status(status);
    for (k, v) in &resp_headers {
        if hop_header(k.as_str()) {
            continue;
        }
        // Force a plain content-length since we may have buffered.
        if k.as_str().eq_ignore_ascii_case("content-length") {
            continue;
        }
        builder = builder.header(k, v);
    }
    builder
        .header("content-length", resp_bytes.len())
        .body(Body::from(resp_bytes))
        .unwrap_or_else(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, "response build failed").into_response()
        })
}

async fn stream_through(
    s: Arc<ProxyState>,
    vendor: Vendor,
    resp: reqwest::Response,
    status: StatusCode,
    headers: HeaderMap,
    req_summary: RequestSummary,
    started: chrono::DateTime<Utc>,
) -> Response {
    use eventsource_stream::Eventsource;

    let mut builder = Response::builder().status(status);
    for (k, v) in &headers {
        if hop_header(k.as_str()) {
            continue;
        }
        builder = builder.header(k, v);
    }

    let upstream = resp.bytes_stream();
    let (tx_down, rx_down) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(32);
    let acc_shared = Arc::new(parking_lot::Mutex::new(ResponseSummary::default()));
    let acc_for_task = acc_shared.clone();
    let s_clone = s.clone();
    let req_summary_clone = req_summary.clone();
    tokio::spawn(async move {
        let mut stream = upstream.eventsource();
        while let Some(event) = stream.next().await {
            match event {
                Ok(ev) => {
                    if matches!(vendor, Vendor::Anthropic) {
                        let mut a = acc_for_task.lock();
                        parse_anthropic_stream_chunk(&ev.data, &mut a);
                    }
                    let mut chunk = String::new();
                    if !ev.event.is_empty() {
                        chunk.push_str("event: ");
                        chunk.push_str(&ev.event);
                        chunk.push('\n');
                    }
                    if !ev.id.is_empty() {
                        chunk.push_str("id: ");
                        chunk.push_str(&ev.id);
                        chunk.push('\n');
                    }
                    for line in ev.data.lines() {
                        chunk.push_str("data: ");
                        chunk.push_str(line);
                        chunk.push('\n');
                    }
                    chunk.push('\n');
                    if tx_down.send(Ok(Bytes::from(chunk))).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    warn!("SSE error: {e}");
                    break;
                }
            }
        }
        let summary = acc_for_task.lock().clone();
        emit_span(&s_clone, vendor, &req_summary_clone, &summary, started);
    });

    let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx_down));
    builder.body(body).unwrap_or_else(|_| {
        (StatusCode::INTERNAL_SERVER_ERROR, "stream build failed").into_response()
    })
}

fn build_upstream_url(base: &str, path_and_query: &Uri) -> String {
    let trimmed = base.trim_end_matches('/');
    let pq = path_and_query
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    format!("{trimmed}{pq}")
}

fn strip_hop_headers(in_h: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (k, v) in in_h.iter() {
        if !hop_header(k.as_str()) {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

fn hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
            | "host"
    )
}

fn is_event_stream(h: &HeaderMap) -> bool {
    h.get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.starts_with("text/event-stream"))
        .unwrap_or(false)
}

/// Build the synthesized terminal response the agent SDK will treat as
/// a clean stop. Anthropic clients see a `stop_reason: killed_by_firewall`
/// message; OpenAI clients see a single choice with `finish_reason: stop`.
fn synthetic_kill_response(vendor: Vendor, reason: &str) -> Response {
    let body = match vendor {
        Vendor::Anthropic => json!({
            "id": format!("msg_{}", ulid::Ulid::new()),
            "type": "message",
            "role": "assistant",
            "model": "sentinel-firewall",
            "content": [{
                "type": "text",
                "text": format!("Sentinel firewall: {reason}")
            }],
            "stop_reason": "killed_by_firewall",
            "stop_sequence": null,
            "usage": {"input_tokens": 0, "output_tokens": 0}
        }),
        Vendor::OpenAI => json!({
            "id": format!("chatcmpl-{}", ulid::Ulid::new()),
            "object": "chat.completion",
            "created": chrono::Utc::now().timestamp(),
            "model": "sentinel-firewall",
            "choices": [{
                "index": 0,
                "message": {"role":"assistant","content": format!("Sentinel firewall: {reason}")},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
        }),
    };
    let mut resp = (StatusCode::OK, axum::Json(body)).into_response();
    resp.headers_mut().insert(
        "x-sentinel-intervention",
        HeaderValue::from_static("killed_by_firewall"),
    );
    resp
}

/// Inject a hint as a system message at the head of the messages array.
fn inject_hint(vendor: Vendor, body: &[u8], hint: &str) -> Bytes {
    let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Bytes::copy_from_slice(body);
    };
    match vendor {
        Vendor::Anthropic => {
            // Anthropic uses a top-level `system` field. Prepend to it.
            let existing = v
                .get("system")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let new_sys = if existing.is_empty() {
                hint.to_string()
            } else {
                format!("{hint}\n\n{existing}")
            };
            v["system"] = serde_json::Value::String(new_sys);
        }
        Vendor::OpenAI => {
            if let Some(arr) = v.get_mut("messages").and_then(|m| m.as_array_mut()) {
                arr.insert(
                    0,
                    json!({"role": "system", "content": format!("Sentinel hint: {hint}")}),
                );
            }
        }
    }
    Bytes::from(serde_json::to_vec(&v).unwrap_or_default())
}

fn emit_span(
    s: &Arc<ProxyState>,
    vendor: Vendor,
    req: &RequestSummary,
    resp: &ResponseSummary,
    started: chrono::DateTime<Utc>,
) {
    let (agent_id, session_id) = s.sessions.get().unwrap_or_else(|| {
        (
            s.cfg
                .server
                .agent_id
                .clone()
                .unwrap_or_else(|| "default".into()),
            "default".into(),
        )
    });
    let model = if req.model.is_empty() {
        match vendor {
            Vendor::Anthropic => "unknown-anthropic".into(),
            Vendor::OpenAI => "unknown-openai".into(),
        }
    } else {
        req.model.clone()
    };
    let cost = cost_usd(
        &model,
        resp.input_tokens,
        resp.output_tokens,
        resp.cache_read_tokens,
        resp.cache_creation_tokens,
    );

    // Local cost / velocity heuristics.
    let now_ms = Utc::now().timestamp_millis() as u64;
    if let Some(trip) = s
        .policy
        .observe_request(&session_id, resp.output_tokens, cost, now_ms)
    {
        match trip {
            Trip::CostThreshold { usd } => {
                warn!(usd, "local cost cap trip; next request will be killed");
                *s.control.kill_until_resume.lock() = Some(format!("cost cap exceeded: ${usd:.2}"));
            }
            Trip::TokenVelocity { tokens_per_sec } => {
                warn!(tokens_per_sec, "local velocity trip");
                *s.control.kill_until_resume.lock() =
                    Some(format!("token velocity {tokens_per_sec:.0}/s exceeded"));
            }
            _ => {}
        }
    }

    let span = SpanRow {
        agent_id: agent_id.clone(),
        agent_session_id: session_id,
        parent_span_id: None,
        kind: if !req.tool_input_hashes.is_empty() {
            "tool_call".into()
        } else {
            "request".into()
        },
        model,
        started_at: started,
        ended_at: Utc::now(),
        input_tokens: resp.input_tokens,
        output_tokens: resp.output_tokens,
        cache_read_tokens: resp.cache_read_tokens,
        cache_creation_tokens: resp.cache_creation_tokens,
        cost_usd: cost,
        tool_name: resp.tool_calls.first().cloned(),
        tool_input_hash: req.tool_input_hashes.first().cloned(),
        stop_reason: resp.stop_reason.clone(),
        properties: None,
    };
    s.shipper.send(span);
}
