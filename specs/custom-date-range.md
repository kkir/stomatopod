# Custom Date Range

## Problem
Dashboard only supports 4 presets: 7d, 30d, 90d, 12m. Users can't analyze arbitrary date windows (e.g., a specific campaign run, a quarter, a launch week).

## Goal
Add a date range picker that lets users set any `from`/`to` date. Presets remain as quick shortcuts.

## API Changes
All query endpoints already accept `range` param. Extend to accept ISO 8601 dates:
```
GET /api/v1/sites/{site}/pageviews?from=2025-01-01&to=2025-01-31
```
- `range` preset param (7d/30d/90d/12m) takes precedence when provided
- `from`+`to` used when `range` absent
- `to` defaults to today if only `from` provided
- Max range: 2 years (query guard)

Server-side: parse both formats in the same handler; convert to `(DateTime, DateTime)` pair before querying.

## URL Params
```
/app/sites/{id}?from=2025-01-01&to=2025-01-31
```
Replaces `range` param in URL when custom range selected.

## UI — Picker
- Replace/extend range dropdown with: preset tabs + "Custom" option
- "Custom" opens a two-input date picker (from / to)
- Show selected range label: "Jan 1 – Jan 31, 2025"
- On mobile: collapse to single dropdown with text inputs

## Granularity Auto-Detection
When range > 90 days → default granularity to `week`.
When range > 365 days → default granularity to `month`.
User can still override manually.

## Edge Cases
- `from` after `to`: swap silently or show validation error
- Future `to` date: clamp to today
- Very short ranges (< 24h): force `hour` granularity
