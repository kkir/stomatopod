# UI Filtering / Segmentation

## Problem
`FilterField`/`FilterOp` types exist in `crates/core/src/query/pageviews.rs` but are not exposed in the dashboard UI. Users can't narrow analytics to a specific country, device, UTM source, etc.

## Goal
Let users build filters from the dashboard by clicking top-N rows or manually entering values. All reports (pageviews timeseries + all top-N breakdowns) re-run with active filters applied.

## Filter Model (already in core)
```rust
pub enum FilterField {
    Url, Referrer, Country, Browser, Os, DeviceType,
    UtmSource, UtmMedium, UtmCampaign, UtmTerm, UtmContent,
    EventName,
}
pub enum FilterOp { Eq, NotEq, Contains, StartsWith }
pub struct Filter { pub field: FilterField, pub op: FilterOp, pub value: String }
```

## API Changes
All existing query endpoints already accept `filters[]` in query params (verify). If not, add:
```
GET /api/v1/sites/{site}/top-pages?filters[0][field]=country&filters[0][op]=eq&filters[0][value]=US
```
Filters are ANDed. Max 10 filters per request.

## URL Params
Active filters serialized into URL so links are shareable:
```
/app/sites/{id}?f=country:eq:US&f=device_type:eq:mobile
```
HTMX partials inherit filters from page URL on re-fetch.

## UI — Filter Bar
- Strip below time-range picker, above charts
- Each active filter shown as a pill: `country = US  ×`
- "Add filter" button opens popover: field dropdown → op dropdown → value input → Add
- Clicking any row in top-N appends `field = value` filter (Eq op, auto-detected field)
- Remove filter → all partials refresh

## UI — Top-N Row Click Behavior
Clicking a row:
1. Country row → adds `country:eq:{code}`
2. Browser row → adds `browser:eq:{name}`
3. Page row → adds `url:eq:{url}`
4. Referrer row → adds `referrer:eq:{domain}`

## Edge Cases
- Filter on EventName only meaningful on events endpoint; hide from page/referrer filter add
- `Contains` / `StartsWith` on URL allow path-prefix filtering (e.g., `/blog/*`)
- Conflict detection not required — duplicate filters are additive

## Open Questions
- Should `NotEq` be exposed in UI or API-only initially?
- Max filter count: 10 seems reasonable, revisit if query performance degrades
