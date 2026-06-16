# Retention / Cohort Analysis

## Problem
No way to measure whether visitors return. Aggregate traffic numbers hide whether growth comes from new visitors or loyal returning ones.

## Goal
Show a retention grid: cohorts of first-visit sessions grouped by week/month, with columns showing what % returned in subsequent periods.

## Approach
Cookieless sessions make true user-level retention impossible. Instead: session-based cohort — group sessions by `first_seen` period, track how many period-over-period session sequences exist from same derived session_id (BLAKE3 hash of `site_id || ip_anon || ua || utc_day`).

Note: session_id changes daily by design (privacy). Cohort retention here means "browser fingerprint approximate return" — should be labeled as "returning visitor approximation" in UI.

## Retention Grid Format

|  | Week 0 | Week 1 | Week 2 | Week 3 | Week 4 |
|--|--------|--------|--------|--------|--------|
| Jun 1 cohort | 100% | 22% | 15% | 11% | 9% |
| Jun 8 cohort | 100% | 19% | 14% | — | — |

- Rows: cohort period (week or month of first visit)
- Columns: period offset (0 = acquisition week, 1 = 1 week later, etc.)
- Value: % of cohort that had at least one session in that offset period

## API Changes
```
GET /api/v1/sites/{site}/retention?granularity=week&range=12w
GET /api/v1/sites/{site}/retention?granularity=month&range=6m
```

Response:
```json
{
  "granularity": "week",
  "cohorts": [
    {
      "period": "2025-06-01",
      "size": 840,
      "retention": [100.0, 22.1, 15.4, 11.2, 9.0]
    }
  ]
}
```

## Query Strategy
1. Assign each session to its first-occurrence cohort period (min(date) for session_id)
2. For each cohort × offset: count distinct session_ids active in that offset window
3. Divide by cohort size

This is a heavier query — cache results with 1-hour TTL.

## UI
- "Retention" tab in site dashboard
- Color-coded grid (heatmap: darker = higher retention)
- Toggle: weekly / monthly granularity
- Tooltip on cell: "X out of Y cohort members returned"
- Disclaimer: "Approximate — cookieless sessions use daily fingerprint hashing"

## CLI Changes
```
spq query retention --site <id> [--granularity week|month] [--range 12w]
```

## Edge Cases
- Incomplete current cohort: rightmost cohort row has fewer offset columns — show as partial
- Very new sites: < 2 cohort periods of data → show message rather than empty grid
- High-traffic sites: cohort query may be slow — enforce range limit (max 52 weeks / 24 months)
