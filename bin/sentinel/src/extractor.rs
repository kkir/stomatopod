use serde::Deserialize;

/// What we recovered from a request body, before sending upstream.
#[derive(Debug, Default, Clone)]
pub struct RequestSummary {
    pub model: String,
    pub stream: bool,
    /// blake3 hex of the canonical-serialized tool_use inputs in the
    /// request. Empty if the request has no tool calls.
    pub tool_input_hashes: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct ResponseSummary {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
    pub stop_reason: Option<String>,
    /// Tool calls invoked by the *assistant* in this response.
    pub tool_calls: Vec<String>,
}

/// Parse an Anthropic `POST /v1/messages` request body.
pub fn parse_anthropic_request(body: &[u8]) -> RequestSummary {
    #[derive(Deserialize)]
    struct Req {
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        stream: Option<bool>,
        #[serde(default)]
        messages: Vec<Msg>,
    }
    #[derive(Deserialize)]
    struct Msg {
        #[serde(default)]
        content: serde_json::Value,
    }

    let Ok(req) = serde_json::from_slice::<Req>(body) else {
        return RequestSummary::default();
    };
    let mut hashes = Vec::new();
    for m in &req.messages {
        collect_tool_use_hashes(&m.content, &mut hashes);
    }
    RequestSummary {
        model: req.model.unwrap_or_default(),
        stream: req.stream.unwrap_or(false),
        tool_input_hashes: hashes,
    }
}

/// Parse an OpenAI `POST /v1/chat/completions` request body.
pub fn parse_openai_request(body: &[u8]) -> RequestSummary {
    #[derive(Deserialize)]
    struct Req {
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        stream: Option<bool>,
        #[serde(default)]
        messages: Vec<Msg>,
    }
    #[derive(Deserialize)]
    struct Msg {
        #[serde(default)]
        tool_calls: Option<Vec<ToolCall>>,
    }
    #[derive(Deserialize)]
    struct ToolCall {
        #[serde(default)]
        function: Option<Func>,
    }
    #[derive(Deserialize)]
    struct Func {
        #[serde(default)]
        arguments: Option<String>,
    }

    let Ok(req) = serde_json::from_slice::<Req>(body) else {
        return RequestSummary::default();
    };
    let mut hashes = Vec::new();
    for m in &req.messages {
        if let Some(calls) = &m.tool_calls {
            for c in calls {
                if let Some(f) = &c.function {
                    if let Some(args) = &f.arguments {
                        hashes.push(blake3_hex(args.as_bytes()));
                    }
                }
            }
        }
    }
    RequestSummary {
        model: req.model.unwrap_or_default(),
        stream: req.stream.unwrap_or(false),
        tool_input_hashes: hashes,
    }
}

/// Parse a non-streaming Anthropic `messages` response.
pub fn parse_anthropic_response(body: &[u8]) -> ResponseSummary {
    #[derive(Deserialize)]
    struct Resp {
        #[serde(default)]
        usage: Option<Usage>,
        #[serde(default)]
        stop_reason: Option<String>,
        #[serde(default)]
        content: serde_json::Value,
    }
    #[derive(Deserialize)]
    struct Usage {
        #[serde(default)]
        input_tokens: u32,
        #[serde(default)]
        output_tokens: u32,
        #[serde(default)]
        cache_read_input_tokens: u32,
        #[serde(default)]
        cache_creation_input_tokens: u32,
    }

    let Ok(r) = serde_json::from_slice::<Resp>(body) else {
        return ResponseSummary::default();
    };
    let usage = r.usage.unwrap_or(Usage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    });
    let mut tool_calls = Vec::new();
    collect_tool_use_names(&r.content, &mut tool_calls);
    ResponseSummary {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_read_tokens: usage.cache_read_input_tokens,
        cache_creation_tokens: usage.cache_creation_input_tokens,
        stop_reason: r.stop_reason,
        tool_calls,
    }
}

/// Parse a non-streaming OpenAI chat completion.
pub fn parse_openai_response(body: &[u8]) -> ResponseSummary {
    #[derive(Deserialize)]
    struct Resp {
        #[serde(default)]
        usage: Option<Usage>,
        #[serde(default)]
        choices: Vec<Choice>,
    }
    #[derive(Deserialize)]
    struct Usage {
        #[serde(default)]
        prompt_tokens: u32,
        #[serde(default)]
        completion_tokens: u32,
    }
    #[derive(Deserialize)]
    struct Choice {
        #[serde(default)]
        finish_reason: Option<String>,
        #[serde(default)]
        message: Option<Msg>,
    }
    #[derive(Deserialize)]
    struct Msg {
        #[serde(default)]
        tool_calls: Option<Vec<ToolCall>>,
    }
    #[derive(Deserialize)]
    struct ToolCall {
        #[serde(default)]
        function: Option<Func>,
    }
    #[derive(Deserialize)]
    struct Func {
        #[serde(default)]
        name: Option<String>,
    }

    let Ok(r) = serde_json::from_slice::<Resp>(body) else {
        return ResponseSummary::default();
    };
    let usage = r.usage.unwrap_or(Usage {
        prompt_tokens: 0,
        completion_tokens: 0,
    });
    let mut tool_calls = Vec::new();
    let mut stop_reason = None;
    for c in &r.choices {
        stop_reason = c.finish_reason.clone();
        if let Some(m) = &c.message {
            if let Some(calls) = &m.tool_calls {
                for tc in calls {
                    if let Some(f) = &tc.function {
                        if let Some(name) = &f.name {
                            tool_calls.push(name.clone());
                        }
                    }
                }
            }
        }
    }
    ResponseSummary {
        input_tokens: usage.prompt_tokens,
        output_tokens: usage.completion_tokens,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        stop_reason,
        tool_calls,
    }
}

/// Parse the terminal `message_delta` event from a streaming Anthropic
/// response. Anthropic sends usage as part of message_delta; we
/// reassemble across chunks.
pub fn parse_anthropic_stream_chunk(event_data: &str, acc: &mut ResponseSummary) {
    #[derive(Deserialize)]
    struct Evt {
        #[serde(rename = "type", default)]
        ty: Option<String>,
        #[serde(default)]
        usage: Option<Usage>,
        #[serde(default)]
        delta: Option<Delta>,
        #[serde(default)]
        message: Option<MsgRef>,
    }
    #[derive(Deserialize)]
    struct Usage {
        #[serde(default)]
        input_tokens: u32,
        #[serde(default)]
        output_tokens: u32,
        #[serde(default)]
        cache_read_input_tokens: u32,
        #[serde(default)]
        cache_creation_input_tokens: u32,
    }
    #[derive(Deserialize)]
    struct Delta {
        #[serde(default)]
        stop_reason: Option<String>,
    }
    #[derive(Deserialize)]
    struct MsgRef {
        #[serde(default)]
        usage: Option<Usage>,
    }

    let Ok(evt) = serde_json::from_str::<Evt>(event_data) else {
        return;
    };
    // `message_start` carries the prompt-side usage and zero output.
    if let Some(mref) = evt.message {
        if let Some(u) = mref.usage {
            acc.input_tokens = u.input_tokens;
            acc.cache_read_tokens = u.cache_read_input_tokens;
            acc.cache_creation_tokens = u.cache_creation_input_tokens;
        }
    }
    // `message_delta` carries the final output_tokens + stop_reason.
    if let Some(u) = evt.usage {
        if u.output_tokens > 0 {
            acc.output_tokens = u.output_tokens;
        }
    }
    if let Some(d) = evt.delta {
        if let Some(sr) = d.stop_reason {
            acc.stop_reason = Some(sr);
        }
    }
    if evt.ty.as_deref() == Some("error") {
        acc.stop_reason = Some("error".into());
    }
}

fn collect_tool_use_hashes(content: &serde_json::Value, out: &mut Vec<String>) {
    if let serde_json::Value::Array(items) = content {
        for item in items {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                if let Some(input) = item.get("input") {
                    out.push(blake3_hex(canonical_json(input).as_bytes()));
                }
            }
        }
    }
}

fn collect_tool_use_names(content: &serde_json::Value, out: &mut Vec<String>) {
    if let serde_json::Value::Array(items) = content {
        for item in items {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    out.push(name.to_string());
                }
            }
        }
    }
}

fn blake3_hex(b: &[u8]) -> String {
    blake3::hash(b).to_hex().to_string()
}

/// Stable JSON serialization: objects sorted by key, no whitespace.
/// Required so the repetition hash is reproducible regardless of how
/// the SDK orders keys in the wire payload.
pub fn canonical_json(v: &serde_json::Value) -> String {
    fn write(v: &serde_json::Value, out: &mut String) {
        match v {
            serde_json::Value::Object(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                out.push('{');
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(k).unwrap_or_default());
                    out.push(':');
                    write(&m[*k], out);
                }
                out.push('}');
            }
            serde_json::Value::Array(arr) => {
                out.push('[');
                for (i, item) in arr.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
        }
    }
    let mut s = String::new();
    write(v, &mut s);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn anthropic_request_extracts_tool_input_hash() {
        let body = serde_json::to_vec(&json!({
            "model": "claude-opus-4-7",
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": "1", "name": "bash", "input": {"cmd": "ls"}}
                ]
            }]
        }))
        .unwrap();
        let s = parse_anthropic_request(&body);
        assert_eq!(s.model, "claude-opus-4-7");
        assert_eq!(s.tool_input_hashes.len(), 1);
        assert!(!s.tool_input_hashes[0].is_empty());
    }

    #[test]
    fn anthropic_response_extracts_usage() {
        let body = serde_json::to_vec(&json!({
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 20},
            "content": []
        }))
        .unwrap();
        let s = parse_anthropic_response(&body);
        assert_eq!(s.input_tokens, 10);
        assert_eq!(s.output_tokens, 20);
        assert_eq!(s.stop_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn anthropic_stream_reassembly() {
        let mut acc = ResponseSummary::default();
        parse_anthropic_stream_chunk(
            &json!({"type":"message_start","message":{"usage":{"input_tokens":42}}}).to_string(),
            &mut acc,
        );
        parse_anthropic_stream_chunk(
            &json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":99}}).to_string(),
            &mut acc,
        );
        assert_eq!(acc.input_tokens, 42);
        assert_eq!(acc.output_tokens, 99);
        assert_eq!(acc.stop_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn canonical_json_is_stable_on_key_order() {
        let a = json!({"b": 1, "a": 2});
        let b = json!({"a": 2, "b": 1});
        assert_eq!(canonical_json(&a), canonical_json(&b));
    }
}
