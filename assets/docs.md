# Stomatopod Documentation

Stomatopod is a privacy-friendly, cookieless web analytics product. It ingests
pageviews and custom events, and exposes analytics over a JSON API and a CLI.

This document is the canonical usage guide. It is published in two forms:

- **Humans:** rendered at `/app/docs` in the dashboard.
- **Machines / LLM agents:** served as Markdown at `/llms.txt`.

Both are generated from the same source, so they never drift.

## Concepts

- **Organization** — the top-level tenant. Owns sites and API keys.
- **Site** — one website/app you track. Identified by a ULID and a domain.
- **Event** — a pageview or a named custom event, attached to a site.
- **API key** — a credential for programmatic access. Two scopes:
  - **Ingest** (`sk_live_…`) — write custom events from a backend.
  - **Read** (`rk_…`) — read-only analytics queries (CLI / LLM agents).

Create and revoke keys in the dashboard under **API Keys** (`/app/keys`, or a
site's **API Keys** tab). The full key is shown **once** at creation — store it
securely. Keys are held only as hashes; a revoked key stops working immediately.

## Authentication

All programmatic requests authenticate with a bearer token:

```
Authorization: Bearer <key>
```

- Ingest endpoints require an **ingest** key bound to the target site.
- Read endpoints require a **read** key (or a logged-in dashboard session).
  A read key is confined to its organization, and to a single site if it was
  created site-bound. Cross-organization access returns `403`.

## Emitting custom events

Send server-side custom events with an ingest key.

```
POST /api/v1/ingest
Authorization: Bearer sk_live_xxxxxxxx
Content-Type: application/json
```

Body fields:

| Field        | Type            | Required | Notes                                            |
|--------------|-----------------|----------|--------------------------------------------------|
| `name`       | string          | yes      | Event name, e.g. `"signup"`.                     |
| `properties` | object          | no       | Arbitrary JSON metadata.                         |
| `url`        | string          | no       | Originating URL; UTM params are parsed from it.  |
| `referrer`   | string          | no       | Referrer URL.                                    |
| `timestamp`  | integer         | no       | Unix milliseconds. Defaults to server time.      |
| `session_id` | string          | no       | Stable id to group events into a session.        |

The site is resolved from the key — you do not pass a site id. Returns `204 No
Content` on success.

Example:

```bash
curl -X POST https://your-host/api/v1/ingest \
  -H "Authorization: Bearer sk_live_xxxxxxxx" \
  -H "Content-Type: application/json" \
  -d '{"name":"signup","properties":{"plan":"pro"},"session_id":"user-42"}'
```

Browser pageviews use a separate public endpoint, `POST /api/v1/event`, keyed by
the site's public tracker key. That path is for the bundled tracker script, not
for backend use.

## Querying analytics

Read endpoints return JSON. Authenticate with a read key.

Common query parameters:

- `range` — `7d`, `30d` (default), `90d`, `12m`.
- `granularity` — `hour`, `day` (default), `week`, `month` (pageviews only).
- `limit` — max rows for top-N endpoints (default `20`).
- `:site` — a site ULID **or** its domain.

| Method & path                                  | Returns                              |
|------------------------------------------------|--------------------------------------|
| `GET /api/v1/sites`                            | Sites visible to the key.            |
| `GET /api/v1/sites/:site/pageviews`            | Pageview/session timeseries.         |
| `GET /api/v1/sites/:site/top-pages`            | Top pages by traffic.                |
| `GET /api/v1/sites/:site/top-referrers`        | Top referrers.                       |
| `GET /api/v1/sites/:site/events`               | Custom event breakdown (`name=`).    |
| `GET /api/v1/sites/:site/funnels`              | Funnels defined for the site.        |
| `GET /api/v1/sites/:site/funnels/:funnel_id`   | Funnel conversion result.            |

Example:

```bash
curl -H "Authorization: Bearer rk_xxxxxxxx" \
  "https://your-host/api/v1/sites/example.com/top-pages?range=7d&limit=10"
```

## CLI

The `stomatopod` binary wraps the read API and is designed for LLM-agent use —
it emits JSON by default (`--human` for a table).

Set the credential once:

```bash
export STOMATOPOD_TOKEN=rk_xxxxxxxx       # a read API key
export STOMATOPOD_SERVER=https://your-host  # defaults to http://localhost:8080
```

Commands:

```bash
stomatopod query pageviews    --site <id|domain> [--range 30d] [--granularity day]
stomatopod query top-pages    --site <id|domain> [--range 30d] [--limit 20]
stomatopod query top-referrers --site <id|domain> [--range 30d] [--limit 20]
stomatopod query events       --site <id|domain> [--name signup] [--range 30d]
stomatopod query funnels      --site <id|domain>
stomatopod query funnel       --site <id|domain> --funnel <funnel_id> [--range 30d]
```

## For LLM agents

To analyze a site's traffic:

1. Obtain a read key (`rk_…`) from the operator.
2. `GET /api/v1/sites` to discover site ids/domains in scope.
3. Query the endpoints above for the metrics you need; prefer `--range` windows
   that match the question (`7d` for recent trends, `12m` for year-over-year).
4. Custom events are queried via `…/events?name=<event>`; emit them via the
   ingest endpoint with an ingest key.

All responses are JSON; errors use standard HTTP status codes (`401`
unauthenticated, `403` out of scope, `404` unknown site, `429` rate-limited).
