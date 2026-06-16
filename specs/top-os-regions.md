# Top OS + Top Regions

## Problem
`os` and `region` are captured on every event but never surfaced in any report or dashboard view.

## Goal
Add `top-os` and `top-regions` report endpoints and surface them in the dashboard alongside existing top-N breakdowns.

## Data Already Captured
- `os` — e.g., "Windows", "macOS", "Android", "iOS", "Linux"
- `region` — ISO 3166-2 subdivision code, e.g., "US-CA", "GB-ENG"

## API Changes

### New endpoints
```
GET /api/v1/sites/{site}/top-os?range=30d&limit=20
GET /api/v1/sites/{site}/top-regions?range=30d&limit=20
```

Response shape identical to existing top-N endpoints:
```json
{
  "rows": [
    { "value": "macOS", "pageviews": 5200, "sessions": 1400, "pct": 43.2 }
  ]
}
```

### Dashboard report
Add `top_os` and `top_regions` to the parallel dashboard fetch in `crates/query/src/reports.rs`.

## UI Changes
- Add two new partial templates: `top-os.jinja`, `top-regions.jinja`
- Add HTMX partial routes: `/app/sites/{id}/partials/top-os`, `/app/sites/{id}/partials/top-regions`
- Place in dashboard grid — suggest replacing or tabbing alongside top-browsers or top-devices

## CLI Changes
```
spq query top-os --site <id> [--range] [--limit]
spq query top-regions --site <id> [--range] [--limit]
```

## Edge Cases
- Region may be empty string if IP geolocation doesn't resolve to subdivision — group as "Unknown"
- OS version not included (separate `top-os-versions` could follow)
