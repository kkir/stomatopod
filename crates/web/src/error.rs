use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use stomatopod_core::error::StoreError;
use tracing::error;

/// Error type produced by dashboard handlers. Wraps the categories of failure
/// a route can encounter and renders them as HTML responses with the
/// appropriate status code.
///
/// Previously every handler swallowed minijinja errors as 200-OK HTML and
/// `StoreError`s as 200-OK error fragments — both invisible to monitoring.
/// `AppError` propagates with `?` and maps to the right status at the edge.
#[derive(Debug)]
pub enum AppError {
    BadRequest(&'static str),
    NotFound(&'static str),
    Store(StoreError),
    Template(minijinja::Error),
}

impl AppError {
    fn status_and_message(&self) -> (StatusCode, String) {
        match self {
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, (*msg).to_string()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, (*msg).to_string()),
            AppError::Store(e) => {
                error!("store error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Storage error".into())
            }
            AppError::Template(e) => {
                error!("template error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Template error".into())
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = self.status_and_message();
        (status, Html(format!("<p>{message}</p>"))).into_response()
    }
}

impl From<StoreError> for AppError {
    fn from(e: StoreError) -> Self {
        AppError::Store(e)
    }
}

impl From<minijinja::Error> for AppError {
    fn from(e: minijinja::Error) -> Self {
        AppError::Template(e)
    }
}
