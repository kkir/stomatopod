# Period Comparison

## Problem
No way to tell if traffic is trending up or down relative to a prior period. Users must manually note numbers across different date selections.

## Goal
Show current period metrics alongside the equivalent prior period. Render delta badges ("+12%" / "-8%") on the timeseries chart and top-N row counts.

## Prior Period Calculation
| Current range | Prior period |
|--------------|-------------|
| 7d | previous 7d |
| 30d | previous 30d |
| 90d | previous 90d |
| 12m | previous 12m |
| Custom from/to | same-length window ending at `from - 1 day` |

## API Changes
Add `compare=true` (or `compare=prev_period`) query param to all report endpoints.

Response shape adds a `comparison` field:
```json
{
  "current": { "pageviews": 1200, "sessions": 340 },
  "comparison": { "pageviews": 1050, "sessions": 290 },
  "delta_pct": { "pageviews": 14.3, "sessions": 17.2 }
}
```

For timeseries: return both series aligned by relative offset (day 1 of current vs day 1 of prior), not absolute timestamp.

## UI
- Toggle: "Compare to prior period" switch in time-range controls
- Timeseries: dashed line for prior period, solid for current. Tooltip shows both values.
- Top-N rows: small badge next to count — `↑14%` (green) / `↓8%` (red) / `—` (< 1% change)
- Summary stat cards: show current value + delta badge

## Edge Cases
- Comparison disabled for real-time view
- If prior period has zero data, show "—" not divide-by-zero
- Very new sites (< 14 days old): prior period may be empty; show gracefully
