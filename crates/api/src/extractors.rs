use axum::{
    extract::{FromRequestParts, Path, Query},
    http::request::Parts,
};
use axum_extra::extract::Query as FormQuery;
use serde::Deserialize;
use stomatopod_core::query::pageviews::{canonical_label, Filter, Granularity, TimeRange};
use ulid::Ulid;

use crate::error::AppError;

/// Most filters a single dashboard query honours. Caps query cost and URL
/// length; extra `filter=` params beyond this are dropped.
const MAX_FILTERS: usize = 10;

/// Path extractor that decodes a single `:site_id` segment as a ULID.
///
/// Replaces the `let site_id = match Ulid::from_string(&site_id_str) { … }`
/// block that every dashboard/partial handler repeated. Returns
/// `AppError::BadRequest` for non-ULID inputs so the response is a clean 400.
pub struct SiteId(pub Ulid);

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
///
/// Holds the canonicalised label alongside the parsed `TimeRange` so
/// handlers can pass a known-good value back into templates and link URLs
/// without propagating arbitrary user input from `?range=`.
pub struct Range {
    pub range: TimeRange,
    pub label: &'static str,
}

impl<S: Send + Sync> FromRequestParts<S> for Range {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(p): Query<RangeParam> = Query::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::BadRequest("invalid range query"))?;
        Ok(Range {
            range: TimeRange::from_label(&p.range),
            label: canonical_label(&p.range),
        })
    }
}

/// Raw query params for the analytics dashboard. Uses `axum_extra`'s `Query`
/// so repeated `filter=` keys deserialize into a `Vec`.
#[derive(Deserialize, Default)]
struct DashParams {
    range: Option<String>,
    from: Option<String>,
    to: Option<String>,
    /// Presence-based: any of `1`/`true`/`on` enables comparison.
    compare: Option<String>,
    #[serde(default)]
    filter: Vec<String>,
}

/// The full set of dashboard query controls: time range (preset *or* custom
/// `from`/`to`), active filters, and the period-comparison toggle. Replaces
/// the bare [`Range`] extractor on the overview, its partials, and the
/// analytics API so all three honour the same URL contract.
pub struct DashQuery {
    pub range: TimeRange,
    pub granularity: Granularity,
    pub filters: Vec<Filter>,
    /// Canonical preset label (`"7d"`/`"30d"`/…). Used for tab highlighting
    /// and for links on sibling pages that only understand presets.
    pub label: &'static str,
    /// True when a custom `from`/`to` range is in effect.
    pub custom: bool,
    pub from: Option<String>,
    pub to: Option<String>,
    pub compare: bool,
}

impl DashQuery {
    fn from_params(p: DashParams) -> Self {
        let filters: Vec<Filter> = p
            .filter
            .iter()
            .filter_map(|s| Filter::parse(s))
            .take(MAX_FILTERS)
            .collect();

        let custom_range = match (p.from.as_deref(), p.to.as_deref()) {
            (Some(f), Some(t)) => TimeRange::parse_dates(f, t),
            _ => None,
        };

        let label_str = p.range.as_deref().unwrap_or("30d");
        let compare = matches!(p.compare.as_deref(), Some("1" | "true" | "on"));

        match custom_range {
            Some(range) => {
                let granularity = Granularity::auto_for_range(&range);
                DashQuery {
                    range,
                    granularity,
                    filters,
                    label: canonical_label(label_str),
                    custom: true,
                    from: p.from,
                    to: p.to,
                    compare,
                }
            }
            None => DashQuery {
                range: TimeRange::from_label(label_str),
                granularity: Granularity::Day,
                filters,
                label: canonical_label(label_str),
                custom: false,
                from: None,
                to: None,
                compare,
            },
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for DashQuery {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let FormQuery(p): FormQuery<DashParams> = FormQuery::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::BadRequest("invalid dashboard query"))?;
        Ok(DashQuery::from_params(p))
    }
}
