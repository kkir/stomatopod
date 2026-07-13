//! A small, I/O-free representation of an HTTP request so command modules can
//! build requests as pure values (easy to unit-test) and dispatch them through
//! [`ApiClient`] in one place.

use serde_json::Value;

use crate::client::ApiClient;

#[derive(Debug, PartialEq)]
pub enum Req {
    Get(String),
    Post(String, Value),
    Patch(String, Value),
    Delete(String),
}

impl Req {
    /// Execute the request. Returns the decoded JSON body for methods that
    /// produce one; `DELETE` (204 No Content) yields `None`.
    pub async fn send(&self, client: &ApiClient) -> anyhow::Result<Option<Value>> {
        Ok(match self {
            Req::Get(path) => Some(client.get(path).await?),
            Req::Post(path, body) => Some(client.post(path, body).await?),
            Req::Patch(path, body) => Some(client.patch(path, body).await?),
            Req::Delete(path) => {
                client.delete(path).await?;
                None
            }
        })
    }
}
