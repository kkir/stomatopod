# Real-Time View

## Problem
Dashboard shows historical data. No way to see what's happening on the site right now — who's visiting, which pages, live event stream.

## Goal
A live view showing the last 30 minutes of activity: active session count, current top pages, recent events. Auto-updates without page reload.

## Metrics (30-minute rolling window)

| Metric | Definition |
|--------|-----------|
| Active sessions | Distinct session_ids with event in last 30 min |
| Active pages | Current page distribution (last pageview per session) |
| Pageviews/min | Rolling 1-min bucket count |
| Recent events | Latest 50 custom events, newest first |

## API Changes

### Polling endpoint (simple)
```
GET /api/v1/sites/{site}/realtime
```
Response:
```json
{
  "active_sessions": 42,
  "pageviews_per_minute": 8.3,
  "top_pages": [
    { "url": "/pricing", "active_sessions": 12, "pct": 28.6 }
  ],
  "recent_events": [
    { "name": "button_click", "url": "/pricing", "seconds_ago": 4, "properties": {} }
  ]
}
```

### SSE stream (optional enhancement)
```
GET /api/v1/sites/{site}/realtime/stream
```
Server-sent events pushing delta updates as events arrive. Useful for live event feed.

## Update Strategy
- Poll `/realtime` every 10 seconds via HTMX or JS `setInterval`
- SSE for live event feed row prepend (lower priority)

## UI
- Separate "Real-time" link in site nav (not the default view)
- Top stat cards: active sessions, pageviews/min
- Mini bar chart: pageviews per minute, last 30 buckets
- "Active pages" table: URL, active sessions count
- "Recent events" live log: scrolling list, auto-prepend new rows
- Indicator badge in nav: "● 42 active" (green dot)

## Performance Considerations
- Real-time query window is always 30 minutes — no index scan on full history
- Add index on `received_at` if not already present
- Rate-limit: 1 request per 5 seconds per site per API key
- SSE: cap at 100 concurrent connections per site

## Edge Cases
- Low-traffic sites: "0 active sessions" is a valid state, not an error
- Disabled for sites with no events in last 24h (avoid keeping connection open for dormant sites)
