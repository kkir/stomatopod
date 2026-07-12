# Analytics Digest

## Problem
Users only see analytics when they log in. There's no passive awareness of how a site is doing - no weekly summary arriving on their notification channel.

## Goal
Send opt-in weekly and/or monthly digests summarizing key site metrics: pageviews, sessions, top pages, top referrers, and period-over-period change. Delivery uses the site's configured notification medium (Slack, Telegram, or webhook) - the same channels as analytics alerts.

## Content

### Weekly digest (sent Monday morning)
- Title: "Your [domain] analytics - last 7 days"
- Last 7 days: total pageviews, sessions, bounce rate
- vs prior 7 days: delta (+12% / -8%)
- Top 3 pages (with pageview counts)
- Top 3 referrers
- Top country
- Link to full dashboard

### Monthly digest (sent 1st of month)
- Same structure but for last 30 days

## Data Model

Table: `digest_subscriptions`
```sql
CREATE TABLE digest_subscriptions (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id),
    site_id     TEXT NOT NULL REFERENCES sites(id),
    frequency   TEXT NOT NULL CHECK (frequency IN ('weekly', 'monthly', 'both')),
    enabled     BOOLEAN NOT NULL DEFAULT TRUE,
    created_at  TIMESTAMPTZ NOT NULL,
    UNIQUE (user_id, site_id)
);
```

## Job Scheduler
Background job runs:
- Weekly: every Monday at 08:00 user's timezone (fallback: UTC)
- Monthly: 1st of month at 08:00

Job fetches enabled subscriptions, dedupes by site, computes stats, and
dispatches a plain-text message to every alert channel on that site.

## Delivery
Reuses alert channel sinks:
- **Slack**: Block Kit header + mrkdwn section
- **Telegram**: Bot API `sendMessage` (plain text)
- **Webhook**: JSON `{ "type": "digest", "title", "text" }` with optional HMAC

No separate email provider. If a site has no channels, the digest is skipped.

## API Changes
```
GET    /api/v1/sites/{site}/digest-subscription    -- get current user's subscription
PUT    /api/v1/sites/{site}/digest-subscription    -- create or update
DELETE /api/v1/sites/{site}/digest-subscription    -- unsubscribe
POST   /api/v1/sites/{site}/digest-subscription/test -- send now (requires a channel)
```

## UI
- "Analytics digest" toggle in site settings
- Frequency selector: Weekly / Monthly / Both
- Copy clarifies delivery via notification destinations below

## Edge Cases
- Site with zero traffic in the period: still send digest but note "No activity this period"
- Multiple users subscribed for one site: one digest per site per cadence (deduped)
- No channels configured: skip delivery; test-send returns 400
- Channel delivery failures: increment bounce_count; disable subscription after 3
