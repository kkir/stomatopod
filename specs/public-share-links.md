# Public Dashboard Share Links

## Problem
Sharing analytics with stakeholders, clients, or the public requires giving them an account or reading numbers aloud. No shareable read-only view exists.

## Goal
Generate a token-scoped public URL that renders a read-only dashboard for a specific site without requiring authentication.

## Data Model

New table: `share_links`
```sql
CREATE TABLE share_links (
    id          TEXT PRIMARY KEY,    -- ULID
    site_id     TEXT NOT NULL REFERENCES sites(id),
    token       TEXT NOT NULL UNIQUE, -- random 32-byte URL-safe token
    label       TEXT,                -- optional name, e.g. "Client view"
    expires_at  TIMESTAMPTZ,         -- NULL = no expiry
    created_by  TEXT NOT NULL REFERENCES users(id),
    created_at  TIMESTAMPTZ NOT NULL
);
```

## URL Format
```
https://{host}/share/{token}
```

No auth required to access. Rate-limited to 60 req/min per token.

## Scope
Share links expose:
- Pageviews timeseries
- Top pages, referrers, countries, browsers, devices
- Custom events summary
- Goals (completions + conversion rate)

Share links do NOT expose:
- Raw event data
- Session details
- API keys or settings
- Funnels with sensitive step data (configurable per link)

## API Changes
```
POST   /api/v1/sites/{site}/share-links           -- create
GET    /api/v1/sites/{site}/share-links           -- list
DELETE /api/v1/sites/{site}/share-links/{id}      -- revoke
PATCH  /api/v1/sites/{site}/share-links/{id}      -- update label/expiry
```

Public access endpoint (no auth):
```
GET /share/{token}            -- renders dashboard HTML
GET /share/{token}/api/*      -- mirrors analytics API endpoints, token-scoped
```

## UI — Share Link Management
- "Share" button in site dashboard header
- Dialog: existing links list + "Create new link" form
- Create form: label (optional), expiry date (optional)
- Copy link button
- Revoke button per link

## UI — Public View
- Same dashboard layout as authenticated view
- Stomatopod branding + "Powered by Stomatopod" footer
- Site domain shown but no org/account info
- No settings, no API keys, no navigation to other sites
- Time range picker: enabled (visitors can explore)
- Filters: enabled (read-only exploration)

## Edge Cases
- Revoked token: 404, not 403 (don't leak that the link existed)
- Expired token: render expiry message rather than redirect to login
- Password protection: out of scope for v1
- Embedding via iframe: add appropriate CSP headers to allow
