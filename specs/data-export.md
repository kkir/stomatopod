# Data Export

## Problem
All analytics data is locked in the dashboard or API JSON responses. No way to pull data into a spreadsheet, data warehouse, or custom BI tool.

## Goal
Allow CSV and JSON bulk export of any report or raw event data via API and dashboard download button.

## Export Types

| Export | Description |
|--------|------------|
| `pageviews` | Timeseries buckets (date, pageviews, sessions) |
| `top-pages` | Full ranked list, no limit cap |
| `top-referrers` | Full ranked list |
| `top-countries` | Full ranked list |
| `top-browsers` | Full ranked list |
| `top-devices` | Full ranked list |
| `top-os` | Full ranked list |
| `top-regions` | Full ranked list |
| `events` | Raw custom events (name, url, properties, timestamp) |
| `sessions` | Derived session rows (entry, exit, duration, bounce, device, referrer, UTM) |

## API Changes
Add `format=csv` param to all existing report endpoints:
```
GET /api/v1/sites/{site}/top-pages?range=30d&format=csv
```
Response: `Content-Type: text/csv`, `Content-Disposition: attachment; filename="top-pages-30d.csv"`

For raw events/sessions, add dedicated export endpoints with cursor-based pagination (events can be large):
```
GET /api/v1/sites/{site}/export/events?range=30d&format=csv&cursor=<token>
GET /api/v1/sites/{site}/export/sessions?range=30d&format=csv&cursor=<token>
```

Pagination response headers:
```
X-Export-Next-Cursor: <token>
X-Export-Total-Rows: 142000
```

## Rate Limiting
- Max export range: 1 year
- Max rows per request: 100,000 (paginate beyond)
- Rate limit: 10 export requests per site per hour

## UI
- "Export" button (download icon) on every top-N panel
- Downloads CSV of that panel's full data (no row limit)
- "Export raw events" / "Export sessions" option in site settings or a dedicated Export page
- Show range + row count before confirming large exports (> 50k rows)

## CLI Changes
```
spq export pageviews --site <id> --range 30d --format csv > pageviews.csv
spq export events --site <id> --range 30d --format json > events.jsonl
spq export sessions --site <id> --range 30d --format csv > sessions.csv
```

## Edge Cases
- Empty range: return CSV with headers only, no rows
- Properties column in events CSV: JSON-encoded string
- Very large exports: stream response rather than buffer in memory
