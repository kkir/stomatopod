# Email Digest

## Problem
Users only see analytics when they log in. There's no passive awareness of how a site is doing — no weekly summary arriving in their inbox.

## Goal
Send opt-in weekly and/or monthly email digests summarizing key site metrics: pageviews, sessions, top pages, top referrers, and period-over-period change.

## Email Content

### Weekly digest (sent Monday morning)
- Subject: "Your [domain] analytics — week of [date]"
- Last 7 days: total pageviews, sessions, bounce rate
- vs prior 7 days: delta badges (+12% / -8%)
- Top 3 pages (with pageview counts)
- Top 3 referrers
- Top country
- Link to full dashboard

### Monthly digest (sent 1st of month)
- Same structure but for last 30 days
- Additional: best day of the month, sparkline summary (text-based)

## Data Model

New table: `digest_subscriptions`
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

Job fetches all enabled subscriptions, computes stats, renders email template, enqueues send.

## Email Template
Plain HTML email (no heavy design framework). Structure:
- Header: site domain + date range
- Stats cards: 3 numbers in a row (pageviews / sessions / bounce rate) with delta
- Top pages: simple table (5 rows max)
- Top referrers: simple table (5 rows max)
- Footer: unsubscribe link, "View full dashboard" CTA

## API Changes
```
GET    /api/v1/sites/{site}/digest-subscription    -- get current user's subscription
PUT    /api/v1/sites/{site}/digest-subscription    -- create or update
DELETE /api/v1/sites/{site}/digest-subscription    -- unsubscribe
```

One-click unsubscribe link in every email (token-based, no login required).

## UI
- "Email digest" toggle in site settings
- Frequency selector: Weekly / Monthly / Both
- "Send test digest" button (triggers immediate send)

## Email Sending
Use SMTP or transactional email provider (Postmark / Resend). Config in `stomatopod.toml`:
```toml
[email]
provider = "postmark"
api_key = "..."
from = "analytics@yourdomain.com"
```

## Edge Cases
- Site with zero traffic in the period: still send digest but note "No activity this week"
- User with access to 5 sites: one email per site per frequency (not batched into one)
- User changes timezone: apply on next digest cycle
- Email bounces: disable subscription after 3 consecutive bounces, notify in UI
