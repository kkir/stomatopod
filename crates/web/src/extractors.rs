use axum::{
    async_trait,
    extract::{FromRequestParts, Path, Query},
    http::request::Parts,
};
use serde::Deserialize;
use stomatopod_core::query::pageviews::TimeRange;
use ulid::Ulid;

use crate::error::AppError;

/// Path extractor that decodes a single `:site_id` segment as a ULID.
///
/// Replaces the `let site_id = match Ulid::from_string(&site_id_str) { … }`
/// block that every dashboard/partial handler repeated. Returns
/// `AppError::BadRequest` for non-ULID inputs so the response is a clean 400.
pub struct SiteId(pub Ulid);

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for SiteId {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(s): Path<String> = Path::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::BadRequest("missing site id"))?;
        Ulid::from_string(&s)
            .map(SiteId)
            .map_err(|_| AppError::BadRequest("invalid site id"))
    }
}

#[derive(Deserialize)]
struct RangeParam {
    #[serde(default = "default_range_label")]
    range: String,
}

fn default_range_label() -> String {
    "30d".into()
}

/// Query extractor that reads `?range=` and turns it into a `TimeRange` via
/// `TimeRange::from_label`. Centralises the label table so the web routes
/// and the CLI agree on what `"7d"` means.
pub struct Range(pub TimeRange);

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for Range {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(p): Query<RangeParam> = Query::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::BadRequest("invalid range query"))?;
        Ok(Range(TimeRange::from_label(&p.range)))
    }
}
