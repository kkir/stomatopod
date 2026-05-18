use anyhow::Result;
use reqwest::Client;
use serde::de::DeserializeOwned;

pub struct ApiClient {
    pub base_url: String,
    client: Client,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(base_url: String) -> Self {
        let token = std::env::var("STOMATOPOD_TOKEN").ok().or_else(|| {
            let path = dirs_path();
            std::fs::read_to_string(path).ok()?.trim().to_string().into()
        });
        Self {
            base_url,
            client: Client::new(),
            token,
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.get(&url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("API error {}: {}", resp.status(), resp.text().await?);
        }
        Ok(resp.json().await?)
    }
}

fn dirs_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(home)
        .join(".config")
        .join("stomatopod")
        .join("credentials")
}
