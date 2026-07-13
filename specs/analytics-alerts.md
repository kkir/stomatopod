# Analytics Alerts

## Problem
Alert dispatcher + webhook/Slack sinks already exist (`crates/web/src/alerts/`) but are wired exclusively to AI firewall incidents. Analytics has no alerting — users discover traffic spikes or drops manually.

## Goal
Reuse alert infrastructure to fire on analytics conditions: traffic spikes/drops
and referrer anomalies.

## Alert Types

| Type | Trigger condition |
|------|------------------|
| `traffic_spike` | Pageviews in last N minutes > X% above rolling baseline |
| `traffic_drop` | Pageviews in last N minutes < X% below rolling baseline |
| `new_referrer_spike` | Single referrer > X% of traffic in last hour (viral spike detection) |
| `daily_summary` | Fixed-time daily digest (optional, see email-digest spec for channel digests) |

## Data Model

New table: `analytics_alerts`
```sql
CREATE TABLE analytics_alerts (
    id          TEXT PRIMARY KEY,  -- ULID
    site_id     TEXT NOT NULL REFERENCES sites(id),
    type        TEXT NOT NULL,     -- traffic_spike | traffic_drop | new_referrer_spike
    config      JSONB NOT NULL,    -- type-specific params (threshold, window_minutes)
    channel_id  TEXT NOT NULL REFERENCES alert_channels(id),  -- legacy FK; fires fan out to all site channels
    enabled     BOOLEAN NOT NULL DEFAULT TRUE,
    created_at  TIMESTAMPTZ NOT NULL
);
```

New table: `analytics_alert_fires`
```sql
CREATE TABLE analytics_alert_fires (
    id          TEXT PRIMARY KEY,
    alert_id    TEXT NOT NULL REFERENCES analytics_alerts(id),
    fired_at    TIMESTAMPTZ NOT NULL,
    payload     JSONB NOT NULL    -- snapshot of values that triggered
);
```

Cooldown: don't re-fire same alert within 1 hour.

## Evaluation
Background worker (similar to AI firewall incident checker) runs every minute:
1. Fetch enabled analytics alerts
2. For each: run the relevant aggregate query against recent data
3. Compare to threshold
4. If triggered and cooldown elapsed: insert fire record + dispatch to every notification channel on the site

## API Changes
```
POST   /api/v1/sites/{site}/analytics-alerts        -- create (no channel_id; site must have ≥1 destination)
GET    /api/v1/sites/{site}/analytics-alerts        -- list
DELETE /api/v1/sites/{site}/analytics-alerts/{id}   -- delete
PATCH  /api/v1/sites/{site}/analytics-alerts/{id}   -- enable/disable
```

## UI
- New "Alerts" tab in site settings (not on main dashboard)
- List alerts with status, last fired time
- Create form: type selector → config fields (dynamic per type); destinations managed under Settings
- `create_site` writes three starter rules as real `analytics_alerts` rows (traffic spike 100%/60m, traffic drop 50%/60m, referrer spike 35%/60m) so users can disable or delete any they do not need; a one-time meta migration backfills empty existing sites
- When the site has no notification destinations, show a callout with a link to Settings; custom create stays disabled until a destination exists

## CLI Changes
```
spq alerts list --site <id>
spq alerts create --site <id> --type traffic_spike --threshold 200 --window 60
spq alerts delete --site <id> --alert <id>
```

## Edge Cases
- Baseline for traffic_spike/drop: rolling 7-day same-hour average
- New sites (< 7 days): disable spike/drop alerts or use absolute threshold only
- Site must have at least one notification channel before create (API rejects otherwise)
