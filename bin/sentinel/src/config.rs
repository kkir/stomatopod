use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentinelConfig {
    #[serde(default)]
    pub listen: ListenConfig,
    pub upstream: UpstreamConfig,
    pub server: ServerConfig,
    #[serde(default)]
    pub policy: PolicyConfig,
    #[serde(default)]
    pub redact: RedactConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for ListenConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

fn default_host() -> String {
    "127.0.0.1".into()
}
fn default_port() -> u16 {
    9999
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamConfig {
    /// e.g. https://api.anthropic.com
    pub url: String,
    /// One of "anthropic" or "openai" — selects the extractor.
    #[serde(default = "default_vendor")]
    pub vendor: String,
}

fn default_vendor() -> String {
    "anthropic".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Stomatopod base URL, e.g. http://localhost:8080
    pub url: String,
    /// Bearer token minted via `stomatopod sites add-sentinel-token`.
    pub token: String,
    /// Stable agent identifier; defaults to the hostname.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Site ID this sentinel reports to.
    pub site_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyConfig {
    /// Trip if the same tool_input_hash appears this many times.
    pub repetition_max: Option<u32>,
    /// Trip if output tokens/sec exceeds this value.
    pub velocity_max_tps: Option<f64>,
    /// Trip if a session burns more than this USD.
    pub cost_cap_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RedactConfig {
    /// Object keys in tool_use inputs whose values should be stripped
    /// before sending the span upstream.
    #[serde(default)]
    pub keys: Vec<String>,
}
