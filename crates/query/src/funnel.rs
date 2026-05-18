use std::sync::Arc;

use stomatopod_core::{
    error::StoreError,
    query::funnel::{FunnelQuery, FunnelResult},
    traits::StorageBackend,
};

/// Execute a funnel query against the given backend.
pub async fn compute_funnel(
    backend: &Arc<dyn StorageBackend>,
    q: &FunnelQuery,
) -> Result<FunnelResult, StoreError> {
    backend.query_funnel(q).await
}
