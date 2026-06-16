# Goals / Conversion Tracking

## Problem
Custom events are trackable but there's no concept of a "goal" — a target event that represents a meaningful conversion. Users can't see conversion rates over time or set targets.

## Goal
Let users designate any custom event (with optional property filters) as a goal. Dashboard shows goal completions + conversion rate (completions / sessions) over time.

## Relationship to Funnels
Goals are single-step funnels. Implementation can reuse funnel step matching logic but goals have dedicated storage and a simplified UI.

## Data Model

New table: `goals`
```sql
CREATE TABLE goals (
    id          TEXT PRIMARY KEY,   -- ULID
    site_id     TEXT NOT NULL REFERENCES sites(id),
    name        TEXT NOT NULL,      -- display name, e.g. "Signup"
    event_name  TEXT NOT NULL,      -- matches custom event name
    filters     JSONB,              -- optional property filters (same Filter type as funnels)
    created_at  TIMESTAMPTZ NOT NULL
);
```

No separate completions table — completions derived by querying events with goal's event_name + filters.

## Metrics
For a given time range:
- **Completions**: COUNT of matching events
- **Unique completions**: COUNT DISTINCT session_id
- **Conversion rate**: unique_completions / total_sessions * 100
- **Timeseries**: completions per day/week/month

## API Changes
```
POST   /api/v1/sites/{site}/goals             -- create
GET    /api/v1/sites/{site}/goals             -- list
DELETE /api/v1/sites/{site}/goals/{id}        -- delete
GET    /api/v1/sites/{site}/goals/{id}/stats?range=30d  -- completions + conversion rate timeseries
```

Stats response:
```json
{
  "goal_id": "...",
  "name": "Signup",
  "completions": 142,
  "unique_completions": 138,
  "conversion_rate": 4.7,
  "timeseries": [
    { "date": "2025-06-01", "completions": 12, "conversion_rate": 3.9 }
  ]
}
```

## UI
- "Goals" tab in site dashboard (alongside Funnels)
- Goal list: name, total completions (30d), conversion rate, sparkline
- Goal detail: timeseries chart + completion count + conversion rate card
- Create goal form: name, event name (autocomplete from known events), optional filters
- Goals summary widget on main dashboard overview (top 3 goals by completions)

## CLI Changes
```
spq query goals --site <id> [--range]
spq query goal --site <id> --goal <id> [--range]
spq goals create --site <id> --name "Signup" --event user_signed_up
```

## Edge Cases
- Deleting a goal does not delete event data
- Same event can be used by multiple goals with different filter conditions
- Conversion rate = 0 if total_sessions = 0 (new site, no division by zero)
