# Entry / Exit Pages

## Problem
Session derivation already computes `entry_url` and `exit_url` per session but there are no report endpoints or UI views for them.

## Goal
Surface top entry pages (where sessions start) and top exit pages (where sessions end) as dedicated reports.

## Data Already Computed
Session derivation logic (in `crates/core/src/query/pageviews.rs` or equivalent) derives:
- `entry_url` — first URL of the session
- `exit_url` — last URL of the session
- `bounce` — session with single pageview and < 30s duration

## API Changes

### New endpoints
```
GET /api/v1/sites/{site}/top-entry-pages?range=30d&limit=20
GET /api/v1/sites/{site}/top-exit-pages?range=30d&limit=20
```

Entry pages response:
```json
{
  "rows": [
    {
      "url": "/blog/getting-started",
      "sessions": 820,
      "pct": 18.4,
      "bounce_rate": 42.1   -- % of sessions starting here that bounced
    }
  ]
}
```

Exit pages response:
```json
{
  "rows": [
    {
      "url": "/pricing",
      "exits": 310,
      "pct": 12.7,
      "exit_rate": 28.4   -- exits / total pageviews on that page
    }
  ]
}
```

## UI
- Add tabs or toggle on the "Top Pages" panel: "All pages" | "Entry pages" | "Exit pages"
- Entry pages show sessions + bounce rate columns
- Exit pages show exits + exit rate columns
- Clicking a row adds URL filter (same as top-pages click behavior)

## CLI Changes
```
spq query top-entry-pages --site <id> [--range] [--limit]
spq query top-exit-pages --site <id> [--range] [--limit]
```

## Edge Cases
- Direct traffic / no-referrer sessions: entry page is the first page regardless
- Single-page sessions: entry_url == exit_url (counted in both reports)
- Exit rate vs bounce rate distinction: exit rate is per-page, bounce rate is per-session-entry
