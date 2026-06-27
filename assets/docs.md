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
- A read key may also **create funnels** (see below). This is the one write a
  read key can perform; it is confined to the same org/site scope as queries.

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

## Tier-4 analytics

These read endpoints surface the data captured by the bundled tracker's
auto-emitted events (see [Auto-captured browser events](#auto-captured-browser-events)).
All accept the common query parameters above and authenticate with a read key.

| Method & path                                       | Returns                                                        |
|-----------------------------------------------------|----------------------------------------------------------------|
| `GET /api/v1/sites/:site/vitals`                    | Core Web Vitals (LCP/CLS/INP) p50/p75/p95. `url=` narrows.     |
| `GET /api/v1/sites/:site/vitals/pages`              | Per-page vitals (`metric=lcp\|cls\|inp`).                      |
| `GET /api/v1/sites/:site/scroll`                    | Scroll-depth reach (25/50/75/100%). `url=` narrows.           |
| `GET /api/v1/sites/:site/scroll/pages`              | Per-page scroll engagement.                                    |
| `GET /api/v1/sites/:site/search`                    | Top internal site-search terms.                                |
| `GET /api/v1/sites/:site/search/zero-results`       | Searches that returned no results.                             |
| `GET /api/v1/sites/:site/search/timeseries`         | Search volume over time.                                       |
| `GET /api/v1/sites/:site/revenue`                   | Revenue totals, orders, AOV, revenue-per-session.              |
| `GET /api/v1/sites/:site/revenue/timeseries`        | Revenue over time.                                             |
| `GET /api/v1/sites/:site/revenue/pages`             | Revenue attributed by page.                                    |
| `GET /api/v1/sites/:site/revenue/breakdown`         | Revenue by `dimension=` (page/referrer/country/...).           |
| `GET /api/v1/sites/:site/experiments`               | A/B experiments seen (variant `properties`).                   |
| `GET /api/v1/sites/:site/experiments/:experiment`   | Per-variant conversion comparison (`goal=`).                   |
| `GET /api/v1/sites/:site/heatmaps/clicks`           | Click heatmap points for a page (`url=` required).             |
| `GET /api/v1/sites/:site/heatmaps/scroll`           | Scroll heatmap buckets for a page (`url=` required).           |

## Auto-captured browser events

The bundled tracker (`/tracker.js`) automatically emits reserved custom events
in addition to pageviews. Each can be disabled per-site with a `data-no-*`
attribute on the script tag (`data-no-vitals`, `data-no-scroll`,
`data-no-clicks`, `data-no-search`) without affecting pageview analytics.

| Event        | Trigger                          | Key properties                                   |
|--------------|----------------------------------|--------------------------------------------------|
| `__vital__`  | Core Web Vital measured          | `metric` (LCP/CLS/INP), `value`, `rating`        |
| `__scroll__` | A scroll-depth milestone reached | `depth` (25/50/75/100), `url`                    |
| `__click__`  | A non-form-field click           | `x`/`y` (% of viewport/page), `url`, `element`   |
| `__search__` | A search query param detected    | `query`                                          |

Revenue and A/B testing reuse ordinary custom events: tag any event with a
`revenue` property (a number) to feed the revenue reports, and with a
`variant` (plus an `experiment` name) property to feed A/B comparisons.

These names are reserved — sending them yourself via the ingest API is
rejected so the tracker remains the single source.

## Public share links

Mint a token-scoped, read-only public dashboard for a single site — no login
required. Manage links with a read key:

| Method & path                                      | Action                          |
|----------------------------------------------------|---------------------------------|
| `POST /api/v1/sites/:site/share-links`             | Create (body: `label?`, `expires_at?` RFC3339). |
| `GET /api/v1/sites/:site/share-links`              | List links (with public `url`). |
| `PATCH /api/v1/sites/:site/share-links/:id`        | Update `label`/`expires_at`.    |
| `DELETE /api/v1/sites/:site/share-links/:id`       | Revoke.                         |

The public surface needs no auth:

- `GET /share/:token` — read-only HTML dashboard shell.
- `GET /share/:token/api/pageviews` — pageview timeseries.
- `GET /share/:token/api/top/:dimension` — `pages`, `referrers`, `countries`, `browsers`, `devices`, `os`, `regions`.
- `GET /share/:token/api/events` — custom event summary.
- `GET /share/:token/api/goals` — goal completions + conversion rate.

Share links expose only aggregate reports — never raw events, sessions, API
keys, or settings. A revoked token returns `404` (existence is never leaked);
an expired token returns `410 Gone`.

## Email digests

Opt in to weekly and/or monthly summary emails per site. Subscriptions are
per-user; manage the current user's subscription with a user-scoped token:

| Method & path                                          | Action                                  |
|--------------------------------------------------------|-----------------------------------------|
| `GET /api/v1/sites/:site/digest-subscription`          | Current subscription (or `null`).       |
| `PUT /api/v1/sites/:site/digest-subscription`          | Create/update (`frequency`: `weekly`/`monthly`/`both`, `enabled?`). |
| `DELETE /api/v1/sites/:site/digest-subscription`       | Unsubscribe.                            |
| `POST /api/v1/sites/:site/digest-subscription/test`    | Send a digest immediately.              |

Weekly digests are sent Monday 08:00 (UTC fallback); monthly on the 1st at
08:00. Every email carries a one-click `GET /digest/unsubscribe/:token` link
that needs no login. Configure delivery under `[email]` in `stomatopod.toml`
(`provider`, `api_key`, `from`) and the public link host via `base_url`; with
no provider the scheduler renders digests but does not send.

## Creating funnels

A funnel is a named, ordered list of steps used to measure conversion. Create
one with a **read** key:

```
POST /api/v1/sites/:site/funnels
Authorization: Bearer rk_xxxxxxxx
Content-Type: application/json
```

Body:

| Field   | Type   | Required | Notes                                              |
|---------|--------|----------|----------------------------------------------------|
| `name`  | string | yes      | Display name for the funnel.                       |
| `steps` | array  | yes      | Ordered step objects (≥ 2).                        |

Each step object:

| Field        | Type   | Required | Notes                                              |
|--------------|--------|----------|----------------------------------------------------|
| `name`       | string | yes      | Step label, e.g. `"Signup"`.                       |
| `event_name` | string | yes      | Event that satisfies the step, e.g. `"pageview"`.  |
| `filters`    | array  | yes      | Property filters; `[]` for none.                   |

Returns `201 Created` with the stored funnel (including its `id`), which you can
then run via `GET /api/v1/sites/:site/funnels/:funnel_id`.

Example:

```bash
curl -X POST https://your-host/api/v1/sites/example.com/funnels \
  -H "Authorization: Bearer rk_xxxxxxxx" \
  -H "Content-Type: application/json" \
  -d '{
        "name": "Signup flow",
        "steps": [
          {"name": "Landing", "event_name": "pageview", "filters": []},
          {"name": "Signup",  "event_name": "signup",   "filters": []}
        ]
      }'
```

## CLI

The `spq` binary wraps the read API and is designed for LLM-agent use — it
emits JSON by default (`--human` for a table). It is a separate binary from the
`stomatopod` server.

Set the credential once:

```bash
export STOMATOPOD_TOKEN=rk_xxxxxxxx       # a read API key
export STOMATOPOD_SERVER=https://your-host  # defaults to http://localhost:8080
```

Commands:

```bash
spq sites
spq query pageviews     --site <id|domain> [--range 30d] [--granularity day]
spq query top-pages     --site <id|domain> [--range 30d] [--limit 20]
spq query top-referrers --site <id|domain> [--range 30d] [--limit 20]
spq query events        --site <id|domain> [--name signup] [--range 30d]
spq query funnels       --site <id|domain>
spq query funnel        --site <id|domain> --funnel <funnel_id> [--range 30d]
spq query funnel-create --site <id|domain> --name <name> --steps '<json-array>'
```

`funnel-create` is the one write command: it posts a new funnel and prints the
created record (with its `id`) as JSON. `--steps` is a JSON array of step
objects with at least two entries:

```bash
spq query funnel-create --site example.com --name "Signup flow" \
  --steps '[{"name":"Landing","event_name":"pageview","filters":[]},
            {"name":"Signup","event_name":"signup","filters":[]}]'
```

Run `spq describe` for a machine-readable JSON manifest of every command and
argument — useful for wiring `spq` into an LLM agent or MCP server.

### Claude Code skill

Install the bundled Claude Code skill globally so `spq` commands are available
in any project session:

```bash
spq skills install          # installs to ~/.claude/skills/spq-analytics/
spq skills install --force  # overwrite an existing installation
```

Claude Code picks up the skill on next session start.

## For LLM agents

To analyze a site's traffic:

1. Obtain a read key (`rk_…`) from the operator.
2. `GET /api/v1/sites` to discover site ids/domains in scope.
3. Query the endpoints above for the metrics you need; prefer `--range` windows
   that match the question (`7d` for recent trends, `12m` for year-over-year).
4. Custom events are queried via `…/events?name=<event>`; emit them via the
   ingest endpoint with an ingest key.
5. To define a new funnel, `POST …/funnels` (or `spq query funnel-create`) with
   a read key — list its results afterward via the funnel endpoints.

All responses are JSON; errors use standard HTTP status codes (`401`
unauthenticated, `403` out of scope, `404` unknown site, `429` rate-limited).
