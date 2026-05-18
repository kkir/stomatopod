use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,

    #[error("already exists")]
    AlreadyExists,

    #[error("database error: {0}")]
    Database(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("query error: {0}")]
    Query(String),

    #[error("backend not available: {0}")]
    Unavailable(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl StoreError {
    pub fn db(e: impl std::fmt::Display) -> Self {
        Self::Database(e.to_string())
    }

    pub fn query(e: impl std::fmt::Display) -> Self {
        Self::Query(e.to_string())
    }
}
