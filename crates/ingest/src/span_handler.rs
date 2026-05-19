use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::warn;
use ulid::Ulid;

use stomatopod_core::{
    domain::{
        agent::Agent,
        agent_span::{AgentSpan, SpanKind},
    },
    traits::MetaStore,
};

/// JSON body posted by the Sentinel sidecar. Multiple spans per request
/// to amortise the HTTP round-trip.
#[derive(Debug, Deserialize)]
pub struct SpanIngestPayload {
    pub spans: Vec<SpanIngestRow>,
}

#[derive(Debug, Deserialize)]
pub struct SpanIngestRow {
    /// Optional — server-mints a ULID if absent.
    #[serde(default)]
    pub id: Option<String>,
    pub agent_id: String,
    pub agent_session_id: String,
    #[serde(default)]
    pub parent_span_id: Option<String>,
    pub kind: SpanKind,
    pub model: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_read_tokens: u32,
    #[serde(default)]
    pub cache_creation_tokens: u32,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input_hash: Option<String>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub properties: Option<serde_json::Value>,
}

pub struct SpanIngestContext {
    pub meta: Arc<dyn MetaStore>,
    pub tx: mpsc::Sender<AgentSpan>,
    /// Cache: token_hash → site_id. Avoids a SQLite hit on every span POST.
    pub token_cache: Arc<DashMap<String, Ulid>>,
    /// Object keys whose values should be stripped from `properties`
    /// before storage. Defends against the sidecar accidentally
    /// shipping secrets in tool args.
    pub redact_keys: Arc<Vec<String>>,
}

/// Handle one span batch. `bearer_token` is the raw token from the
/// `Authorization: Bearer ...` header; the function hashes it and looks
/// it up in `sentinel_tokens`.
pub async fn handle_span_ingest(
    ctx: &SpanIngestContext,
    bearer_token: &str,
    payload: SpanIngestPayload,
) -> StatusCode {
    let token_hash = hex_blake3(bearer_token);

    // Hot-path token check via the cache.
    let site_id = if let Some(id) = ctx.token_cache.get(&token_hash) {
        *id
    } else {
        match ctx.meta.get_sentinel_token_by_hash(&token_hash).await {
            Ok(Some(tok)) => {
                ctx.token_cache.insert(token_hash.clone(), tok.site_id);
                // Fire-and-forget last-used update.
                let meta = ctx.meta.clone();
                let id = tok.id;
                tokio::spawn(async move {
                    if let Err(e) = meta.touch_sentinel_token(id).await {
                        warn!("touch_sentinel_token failed: {e}");
                    }
                });
                tok.site_id
            }
            Ok(None) => return StatusCode::UNAUTHORIZED,
            Err(e) => {
                warn!("sentinel token lookup error: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
        }
    };

    if payload.spans.is_empty() {
        return StatusCode::NO_CONTENT;
    }

    // Upsert the agent row so the dashboard can list known agents even
    // before the parquet flush makes spans queryable. Best-effort; never
    // blocks ingest on failure.
    if let Some(first) = payload.spans.first() {
        let agent = Agent {
            id: Ulid::new(),
            site_id,
            agent_id: first.agent_id.clone(),
            name: first.agent_id.clone(),
            policy_id: None,
            created_at: Utc::now(),
            last_seen_at: Utc::now(),
        };
        let meta = ctx.meta.clone();
        tokio::spawn(async move {
            if let Err(e) = meta.upsert_agent(&agent).await {
                warn!("upsert_agent failed: {e}");
            }
        });
    }

    let received_at = Utc::now();
    for row in payload.spans {
        let span = match row_to_span(row, site_id, &ctx.redact_keys, received_at) {
            Ok(s) => s,
            Err(e) => {
                warn!("malformed span row: {e}");
                return StatusCode::BAD_REQUEST;
            }
        };
        match ctx.tx.try_send(span) {
            Ok(_) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                return StatusCode::TOO_MANY_REQUESTS
            }
            Err(_) => return StatusCode::SERVICE_UNAVAILABLE,
        }
    }
    StatusCode::NO_CONTENT
}

fn row_to_span(
    row: SpanIngestRow,
    site_id: Ulid,
    redact_keys: &[String],
    _received_at: DateTime<Utc>,
) -> anyhow::Result<AgentSpan> {
    let id = row
        .id
        .as_deref()
        .map(Ulid::from_string)
        .transpose()?
        .unwrap_or_else(Ulid::new);
    let parent_span_id = row
        .parent_span_id
        .as_deref()
        .map(Ulid::from_string)
        .transpose()?;
    let properties = row.properties.map(|mut v| {
        redact(&mut v, redact_keys);
        v
    });
    Ok(AgentSpan {
        id,
        site_id,
        agent_id: row.agent_id,
        agent_session_id: row.agent_session_id,
        parent_span_id,
        kind: row.kind,
        model: row.model,
        started_at: row.started_at,
        ended_at: row.ended_at,
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        cache_read_tokens: row.cache_read_tokens,
        cache_creation_tokens: row.cache_creation_tokens,
        cost_usd: row.cost_usd,
        tool_name: row.tool_name,
        tool_input_hash: row.tool_input_hash,
        stop_reason: row.stop_reason,
        properties,
    })
}

/// Strip values for any object key in `keys`, recursively. Replaces the
/// value with the string `"[redacted]"` so the redaction is visible in
/// the dashboard rather than silently dropped.
fn redact(v: &mut serde_json::Value, keys: &[String]) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                if keys.iter().any(|rk| rk.eq_ignore_ascii_case(k)) {
                    *val = serde_json::Value::String("[redacted]".into());
                } else {
                    redact(val, keys);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                redact(item, keys);
            }
        }
        _ => {}
    }
}

fn hex_blake3(s: &str) -> String {
    let h = blake3::hash(s.as_bytes());
    h.to_hex().to_string()
}

/// Helper for callers (sentinel binary, tests) that need to compute the
/// hash they'd POST to `/api/v1/spans` with.
pub fn token_hash(s: &str) -> String {
    hex_blake3(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_top_level_secret() {
        let mut v: serde_json::Value = serde_json::json!({
            "tool": "search",
            "api_key": "sk-abcd1234",
            "query": "hello"
        });
        redact(&mut v, &["api_key".into()]);
        assert_eq!(v["api_key"], serde_json::Value::String("[redacted]".into()));
        assert_eq!(v["query"], serde_json::json!("hello"));
    }

    #[test]
    fn redact_nested() {
        let mut v: serde_json::Value = serde_json::json!({
            "inputs": {
                "headers": { "authorization": "Bearer xxx" },
                "body": "ok",
            }
        });
        redact(&mut v, &["authorization".into()]);
        assert_eq!(
            v["inputs"]["headers"]["authorization"],
            serde_json::Value::String("[redacted]".into())
        );
        assert_eq!(v["inputs"]["body"], serde_json::json!("ok"));
    }

    #[test]
    fn redact_case_insensitive() {
        let mut v: serde_json::Value = serde_json::json!({"API_KEY": "x"});
        redact(&mut v, &["api_key".into()]);
        assert_eq!(v["API_KEY"], serde_json::Value::String("[redacted]".into()));
    }
}
