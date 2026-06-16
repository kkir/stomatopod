# Sparklines on Top-N Rows

## Problem
Top-N tables show aggregate counts for the selected period but no trend direction. A page with 1,000 views could be growing fast or dying — the table doesn't say.

## Goal
Add a small 7-day trend sparkline next to each row's count in every top-N table. No axes, no labels — just shape of the trend.

## Approach
Each sparkline is a 7-point series (one value per day) for that specific dimension value. Rendered as a tiny SVG path inline in the table cell.

## API Changes

Add `sparklines=true` param to top-N endpoints:
```
GET /api/v1/sites/{site}/top-pages?range=30d&sparklines=true
```

When present, each row includes a `trend` array:
```json
{
  "rows": [
    {
      "value": "/pricing",
      "pageviews": 5200,
      "sessions": 1400,
      "pct": 18.2,
      "trend": [120, 145, 130, 190, 210, 185, 220]  -- last 7 days, oldest first
    }
  ]
}
```

The `trend` array always covers the last 7 calendar days regardless of the main report's range. This provides consistent trend context.

## Implementation
Trend data requires a sub-query per dimension value — expensive for N=20 rows. Options:
1. **Single query**: window function or UNION across all rows in one query (preferred)
2. **N queries in parallel**: simple but 20 DB round-trips per dashboard load

Prefer option 1. Cache sparkline data with 1-hour TTL since it changes slowly.

## UI
- Sparkline SVG rendered in a 60×20px cell after the count column
- Color: green if last value > first value, red if declining, gray if flat (< 5% change)
- No tooltip needed — clicking the row filters dashboard to show full chart for that dimension
- Lazy-load: fetch sparklines after initial table renders (separate HTMX request) to not block page

## Edge Cases
- New pages/referrers (< 7 days old): pad missing days with 0
- Zero-traffic days in range: render as 0 point, not gap
- Performance: sparklines=true is opt-in — default dashboard can skip them to keep initial load fast
