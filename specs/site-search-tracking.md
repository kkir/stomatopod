# Site Search Tracking

## Problem
Internal site search is a high-signal behavior — what users search for reveals what they want but can't find through navigation. This intent data is invisible in standard analytics.

## Goal
Track internal search queries and report top search terms, search volume over time, and zero-result searches.

## Tracking Approaches

### Option A: Auto-detect (recommended for v1)
Tracker monitors URL changes for common search param patterns:
```js
const SEARCH_PARAMS = ['q', 'query', 's', 'search', 'term', 'keyword'];

function extractSearchQuery(url) {
  const params = new URLSearchParams(new URL(url, location.origin).search);
  for (const p of SEARCH_PARAMS) {
    if (params.has(p)) return params.get(p);
  }
  return null;
}

// Check on each pageview
const query = extractSearchQuery(location.href);
if (query) {
  sendEvent('__search__', { query: query.slice(0, 200), url: location.pathname });
}
```

### Option B: Manual instrumentation
```js
stomatopod.track('site_search', { query: searchQuery, results_count: 0 });
```

Both approaches store as custom events — no schema change.

## API Changes

### Top search terms
```
GET /api/v1/sites/{site}/search?range=30d&limit=20
```
Response:
```json
{
  "total_searches": 3240,
  "rows": [
    { "query": "pricing", "count": 420, "pct": 13.0 },
    { "query": "api docs", "count": 280, "pct": 8.6 }
  ]
}
```

### Zero-result searches (requires manual instrumentation with `results_count`)
```
GET /api/v1/sites/{site}/search/zero-results?range=30d&limit=20
```

### Search volume timeseries
```
GET /api/v1/sites/{site}/search/timeseries?range=30d&granularity=day
```

## UI
- "Search" tab in site dashboard (only visible if search events detected)
- Top search terms table: query, count, % of searches
- Timeseries: search volume over time
- Zero-results tab (if `results_count` data available): queries with no results — high-priority content gaps
- Click query to filter top pages by sessions that searched that term

## Configuration
Allow sites to configure their search param names (for non-standard implementations):
```toml
[site.search]
params = ["q", "keyword", "custom_search"]
```

## CLI Changes
```
spq query search --site <id> [--range] [--limit]
spq query search-zero-results --site <id> [--range]
```

## Edge Cases
- PII in search queries: user might search for their own name or email. Consider: truncate at 200 chars, strip email-like patterns (`/\S+@\S+/`), document that queries are stored as-is
- Query normalization: lowercase + trim whitespace before storage
- Auto-detect false positives: filter out navigation params that match common search param names (e.g., `?s=` in WordPress can be a category slug)
- `__search__` events excluded from regular custom events reports
