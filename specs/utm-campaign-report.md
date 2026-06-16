# UTM Campaign Report

## Problem
UTM parameters are captured on every session but the only report is `top-referrers`. There's no way to analyze campaign performance across the source → medium → campaign → content → term hierarchy.

## Goal
Dedicated UTM analytics view with drill-down from source → medium → campaign → term/content, showing sessions, pageviews, and goal conversion rates per campaign dimension.

## Data Already Captured
`utm_source`, `utm_medium`, `utm_campaign`, `utm_term`, `utm_content` on every session that arrives with UTM params.

## API Changes

### Top-level UTM breakdown
```
GET /api/v1/sites/{site}/utm?dimension=source&range=30d&limit=20
GET /api/v1/sites/{site}/utm?dimension=medium&range=30d
GET /api/v1/sites/{site}/utm?dimension=campaign&range=30d
GET /api/v1/sites/{site}/utm?dimension=term&range=30d
GET /api/v1/sites/{site}/utm?dimension=content&range=30d
```

### Filtered drill-down
```
GET /api/v1/sites/{site}/utm?dimension=medium&utm_source=google&range=30d
GET /api/v1/sites/{site}/utm?dimension=campaign&utm_source=google&utm_medium=cpc&range=30d
```

Response:
```json
{
  "dimension": "campaign",
  "rows": [
    {
      "value": "summer_launch",
      "sessions": 1240,
      "pageviews": 4100,
      "pct": 18.2,
      "goal_conversions": 42,     -- if goals defined
      "conversion_rate": 3.4
    }
  ]
}
```

## UI
- "Campaigns" tab in site dashboard nav
- Default view: top sources table
- Breadcrumb drill-down: Sources → `google` → Mediums → `cpc` → Campaigns
- Each row clickable to narrow one level deeper
- Time range selector (same as main dashboard)
- Optional: conversion rate column if goals are defined

## CLI Changes
```
spq query utm --site <id> --dimension source [--range] [--limit]
spq query utm --site <id> --dimension campaign --utm-source google --utm-medium cpc [--range]
```

## Edge Cases
- Sessions without UTM: excluded from UTM report (not counted as "unknown" to avoid inflating data)
- UTM values are case-sensitive in storage; normalize to lowercase on ingest or at query time (decision needed)
- `utm_term` / `utm_content` often empty — don't show empty-value rows
