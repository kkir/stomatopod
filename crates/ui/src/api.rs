//! Fetch helpers over the same-origin JSON API (`/api/v1/*`). Calls are
//! credentialed same-origin so the existing dashboard session cookie
//! authenticates them via `require_api_auth` (see crates/web) with no new
//! auth machinery on the client.
//!
//! No page calls these yet: real data wiring (`use_resource` per page)
//! starts in Phase 3, so this module is otherwise dead code until then.
#![allow(dead_code)]

use gloo_net::http::{Request, Response};
use serde::de::DeserializeOwned;
use serde::Serialize;
use web_sys::RequestCredentials;

/// Errors surfaced to callers. A 401 is handled centrally below (full
/// navigation to `/login`) before returning, so callers mostly see this
/// for other failure modes.
#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    /// The request could not be sent, or the transport itself failed.
    Network(String),
    /// A non-2xx, non-401 HTTP response.
    Status { code: u16, message: String },
    /// The response body was not valid JSON for the expected type.
    Deserialize(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Network(msg) => write!(f, "network error: {msg}"),
            ApiError::Status { code, message } => write!(f, "HTTP {code}: {message}"),
            ApiError::Deserialize(msg) => write!(f, "could not read response: {msg}"),
        }
    }
}

/// A 401 means the session cookie expired or was never set. `/login` is
/// server-rendered SSR, so this is a full navigation, not a router push.
fn session_expired() -> ApiError {
    if let Some(window) = web_sys::window() {
        let _ = window.location().set_href("/login");
    }
    ApiError::Status {
        code: 401,
        message: "session expired".to_string(),
    }
}

async fn read_json<T: DeserializeOwned>(resp: Response) -> Result<T, ApiError> {
    if resp.status() == 401 {
        return Err(session_expired());
    }
    if !resp.ok() {
        let message = resp.text().await.unwrap_or_default();
        return Err(ApiError::Status {
            code: resp.status(),
            message,
        });
    }
    resp.json::<T>()
        .await
        .map_err(|e| ApiError::Deserialize(e.to_string()))
}

async fn read_empty(resp: Response) -> Result<(), ApiError> {
    if resp.status() == 401 {
        return Err(session_expired());
    }
    if !resp.ok() {
        let message = resp.text().await.unwrap_or_default();
        return Err(ApiError::Status {
            code: resp.status(),
            message,
        });
    }
    Ok(())
}

pub async fn get_json<T: DeserializeOwned>(path: &str) -> Result<T, ApiError> {
    let resp = Request::get(path)
        .credentials(RequestCredentials::SameOrigin)
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    read_json(resp).await
}

pub async fn post_json<B: Serialize + ?Sized, T: DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<T, ApiError> {
    let resp = Request::post(path)
        .credentials(RequestCredentials::SameOrigin)
        .json(body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    read_json(resp).await
}

pub async fn patch_json<B: Serialize + ?Sized, T: DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<T, ApiError> {
    let resp = Request::patch(path)
        .credentials(RequestCredentials::SameOrigin)
        .json(body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    read_json(resp).await
}

pub async fn put_json<B: Serialize + ?Sized, T: DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<T, ApiError> {
    let resp = Request::put(path)
        .credentials(RequestCredentials::SameOrigin)
        .json(body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    read_json(resp).await
}

/// Fire-and-forget delete; the API returns no body worth decoding.
pub async fn delete(path: &str) -> Result<(), ApiError> {
    let resp = Request::delete(path)
        .credentials(RequestCredentials::SameOrigin)
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    read_empty(resp).await
}
