# Annotations

## Problem
Traffic changes on the timeseries chart have no context. A spike on June 3rd might be a product launch, a viral post, or a bug — but there's nothing in the dashboard to say which.

## Goal
Let users pin dated notes to the timeseries chart. Annotations appear as vertical markers with labels so traffic changes can be correlated with real events.

## Data Model

New table: `annotations`
```sql
CREATE TABLE annotations (
    id          TEXT PRIMARY KEY,   -- ULID
    site_id     TEXT NOT NULL REFERENCES sites(id),
    date        DATE NOT NULL,
    label       TEXT NOT NULL,      -- short display label, max 80 chars
    note        TEXT,               -- optional longer description
    created_by  TEXT NOT NULL REFERENCES users(id),
    created_at  TIMESTAMPTZ NOT NULL
);
```

Index: `(site_id, date)` — queries filter by site + date range.

## API Changes
```
POST   /api/v1/sites/{site}/annotations           -- create
GET    /api/v1/sites/{site}/annotations?from=&to= -- list (date range)
PATCH  /api/v1/sites/{site}/annotations/{id}      -- update label/note
DELETE /api/v1/sites/{site}/annotations/{id}      -- delete
```

Create body:
```json
{ "date": "2025-06-03", "label": "Launched v2.0", "note": "Blog post + HN submission" }
```

Annotations are returned alongside timeseries data when `include_annotations=true`:
```
GET /api/v1/sites/{site}/pageviews?range=30d&include_annotations=true
```

## UI
- Annotations shown as vertical dashed lines on the timeseries chart
- Label appears as small tag above the line
- Hover/click: show full note in tooltip
- Click empty area on chart to open "Add annotation" form (date pre-filled)
- Manage annotations: list in site settings with edit/delete

## CLI Changes
```
spq annotations list --site <id> [--range]
spq annotations create --site <id> --date 2025-06-03 --label "Launched v2.0"
spq annotations delete --site <id> --annotation <id>
```

## Edge Cases
- Multiple annotations on the same date: stack markers or show count badge with expand on hover
- Annotations outside the current chart range: not rendered (fetched only for visible range)
- Label truncation: max 80 chars; truncate with ellipsis at 40 chars in chart marker
