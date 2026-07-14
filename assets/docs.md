# Stomatopod Documentation

Stomatopod is a privacy-friendly, cookieless web analytics product. It ingests
pageviews and custom events, and exposes analytics over a JSON API and a CLI.

This document is the canonical usage guide. It is published in three forms:

- **Humans:** rendered at `/docs` in the dashboard.
- **Machines / LLM agents:** served as Markdown at `/llms.txt`.
- **Typed clients / codegen:** OpenAPI 3 at `/openapi.json` (derived from the Rust API types).

The human and Markdown forms share this source. The OpenAPI document is generated
from the same handlers and request types so path and schema details stay aligned
with the running server.

## Concepts

- **Organization** — the top-level tenant. Owns sites and API keys.
- **Site** — one website/app you track. Identified by a ULID and a domain.
- **Event** — a pageview or a named custom event, attached to a site.
- **API key** — a credential for programmatic access. Two scopes:
  - **Ingest** (`sk_live_…`) — write custom events from a backend.
  - **Read** (`rk_…`) — read-only analytics queries (CLI / LLM agents).

Create and revoke keys in the dashboard under **API Keys** (`/keys`, or a
site's **API Keys** tab). The full key is shown **once** at creation - store it
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
the site's public tracker key. That path is for the bundled tracker script (see
[Installing the browser tracker](#installing-the-browser-tracker)), not for
backend use.

## Querying analytics

Read endpoints return JSON. Authenticate with a read key.

Common query parameters:

- `range` — `7d`, `30d` (default), `90d`, `12m`.
- `granularity` — `hour`, `day` (default), `week`, `month` (pageviews only).
- `limit` — max rows for top-N endpoints (default `20`).
- `:site` — a site ULID **or** its domain.

Common filters (repeatable `filter=field:op:value`): fields `url`, `referrer`,
`country`, `region`, `browser`, `os`, `device_type`, `utm_source`, `utm_medium`,
`utm_campaign`, `utm_term`, `utm_content`, `event_name`; ops `eq`, `not_eq`,
`contains`, `starts_with`. Custom windows: `from`/`to` as `YYYY-MM-DD`.

| Method & path                                  | Returns                              |
|------------------------------------------------|--------------------------------------|
| `GET /api/v1/sites`                            | Sites visible to the key.            |
| `GET /api/v1/sites/:site/pageviews`            | Pageview/session timeseries.         |
| `GET /api/v1/sites/:site/top-pages`            | Top pages by traffic.                |
| `GET /api/v1/sites/:site/top-referrers`        | Top referrers.                       |
| `GET /api/v1/sites/:site/top-os`               | Top operating systems.               |
| `GET /api/v1/sites/:site/top-regions`          | Top regions.                         |
| `GET /api/v1/sites/:site/top-countries`        | Top countries.                       |
| `GET /api/v1/sites/:site/top-browsers`         | Top browsers.                        |
| `GET /api/v1/sites/:site/top-devices`          | Top device types.                    |
| `GET /api/v1/sites/:site/top-entry-pages`      | Top entry (landing) pages.           |
| `GET /api/v1/sites/:site/top-exit-pages`       | Top exit pages.                      |
| `GET /api/v1/sites/:site/events`               | Custom event breakdown (`name=`).    |
| `GET /api/v1/sites/:site/campaigns`            | UTM campaign breakdown.              |
| `GET /api/v1/sites/:site/export/events`        | Export events.                       |
| `GET /api/v1/sites/:site/export/sessions`      | Export sessions.                     |
| `GET /api/v1/sites/:site/funnels`              | Funnels defined for the site.        |
| `GET /api/v1/sites/:site/funnels/:funnel_id`   | Funnel conversion result.            |
| `DELETE /api/v1/sites/:site/funnels/:funnel_id`| Delete a funnel definition.          |
| `GET /api/v1/sites/:site/utm`                  | Single UTM dimension top-list (`dimension=source|medium|campaign|term|content`). |
| `GET /api/v1/sites/:site/analytics-alerts`     | List analytics alerts for the site.  |
| `POST /api/v1/sites/:site/analytics-alerts`    | Create an analytics alert (session). |
| `GET /health`                                  | Liveness probe (public).             |
| `GET /ready`                                   | Readiness probe (public).            |
| `GET /openapi.json`                            | OpenAPI 3 contract (public).         |
| `POST /api/v1/me/password`                     | Change owner password (session).     |

Example:

```bash
curl -H "Authorization: Bearer rk_xxxxxxxx" \
  "https://your-host/api/v1/sites/example.com/top-pages?range=7d&limit=10"
```

## Installing the browser tracker

The bundled tracker is a tiny, dependency-free script served at `/tracker.js`.
Add a single tag to every page you want to measure; it records a pageview on
load and on client-side route changes. No cookies are set and no cross-site
identifier is used.

```html
<script defer src="https://your-host/tracker.js" data-site="YOUR_PUBLIC_KEY"></script>
```

`data-site` is the site's **public key** (the `Public Key` shown on the site's
**Settings** tab, or via `GET /api/v1/sites`). Unlike the `sk_live_…`/`rk_…`
keys, it is meant to be embedded in client HTML: it only authorizes writing
pageviews and events to that one site through the public `POST /api/v1/event`
endpoint, and grants no read or cross-site access. The tracker resolves the
site from this key, so you never pass a site id.

By default the tracker posts to `{script origin}/api/v1/event`, so loading
`tracker.js` from your Stomatopod host is enough. Set `data-api` only when the
event endpoint lives on a different origin (for example a CDN for the script
and a separate ingest host).

The script is served with a one-day immutable cache, so reference it directly
from your host rather than copying its contents.

### Script-tag attributes

| Attribute        | Effect                                                                                      |
|------------------|---------------------------------------------------------------------------------------------|
| `data-site`      | **Required.** Site public key. The tracker no-ops if it is missing.                         |
| `data-api`       | **Optional.** Full event endpoint URL. If omitted, the tracker uses `{script origin}/api/v1/event` from the script `src` (not the page origin). |
| `data-exclude`   | Disable the tracker for this page load entirely (handy for staging/admin pages).            |

### Behavior

- A `pageview` is sent on initial load (or when the tab becomes visible if the
  page was prerendered / opened in the background) and on SPA navigations the
  tracker intercepts (`history.pushState` / `replaceState` and `popstate`).
- SPA pageviews only fire when `pathname` or `search` changes. Hash-only
  updates (common for scroll-spy section links) and other same-URL
  `replaceState` calls are ignored, so scrolling does not spam pageviews.
- Consecutive navigations to the same path+query are deduplicated.
- Restores from the back/forward cache (`pageshow` with `persisted`) count as
  a fresh pageview.
- Every beacon carries the current URL, referrer, screen size, and browser
  language; UTM parameters are parsed from the URL server-side.
- Beacons use `navigator.sendBeacon` when available (falling back to
  `fetch(..., {keepalive:true})`), so they survive page unload.
- **Do Not Track is honored:** if the browser reports
  `navigator.doNotTrack === "1"`, the tracker sends nothing.
- Loading the script twice is a no-op after the first init.

### Manual events from the browser

Once loaded, the tracker exposes a global helper for your own custom events
(the same events you later query via `…/events?name=<event>`):

```js
stomatopod("event", "signup", { plan: "pro" });
```

You can also queue calls before the script loads:

```js
window.stomatopod = window.stomatopod || [];
stomatopod.push(["event", "signup", { plan: "pro" }]);
```

The first argument is always `"event"`, followed by the event name and an
optional properties object. Properties are stored as JSON and shown in custom
event breakdowns; use them for anything your app needs (plan name, feature
flag, etc.).

## Analytics digests

Opt in to weekly and/or monthly summary digests per site. Digests are delivered
through the site's configured notification channels (Slack, Telegram, or
webhook) - the same destinations used for analytics alerts. Subscriptions are
per-user; manage the current user's subscription with a dashboard session:

| Method & path                                          | Action                                  |
|--------------------------------------------------------|-----------------------------------------|
| `GET /api/v1/sites/:site/digest-subscription`          | Current subscription (or `null`).       |
| `PUT /api/v1/sites/:site/digest-subscription`          | Create/update (`frequency`: `weekly`/`monthly`/`both`, `enabled?`). |
| `DELETE /api/v1/sites/:site/digest-subscription`       | Unsubscribe.                            |
| `POST /api/v1/sites/:site/digest-subscription/test`    | Send a digest immediately.              |

Weekly digests are sent Monday 08:00 (UTC fallback); monthly on the 1st at
08:00. Configure at least one notification destination under Site Settings.
With no channels configured the scheduler skips delivery. Dashboard links in
the message use `base_url` from `stomatopod.toml`.

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

The `stoma` binary wraps the read API and is designed for LLM-agent use — it
emits JSON by default (`--human` for a table). It is a separate binary from the
`stomatopod` server.

### Install

Install with [`cargo binstall`](https://github.com/cargo-bins/cargo-binstall)
(prebuilt binary when a matching GitHub release exists; otherwise compiles from
source):

```bash
# one-time: install cargo-binstall
curl -L --proto '=https' --tlsv1.2 -sSf \
  https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash

# install the stoma binary into ~/.cargo/bin
cargo binstall --git https://github.com/kkir/stomatopod stomatopod-cli
```

Private repo: log in with `gh`, or set `GITHUB_TOKEN` / `GH_TOKEN`.

From a monorepo checkout:

```bash
cargo binstall --manifest-path bin/stoma --locked stomatopod-cli
# equivalent: cargo install --path bin/stoma
```

### Credentials

Set the credential once:

```bash
export STOMATOPOD_TOKEN=rk_xxxxxxxx       # a read API key
export STOMATOPOD_SERVER=https://your-host  # defaults to http://localhost:8080
```

### Commands

```bash
stoma sites
stoma query pageviews       --site <id|domain> [--range 30d] [--granularity day]
stoma query top-pages       --site <id|domain> [--range 30d] [--limit 20]
stoma query top-referrers   --site <id|domain> [--range 30d] [--limit 20]
stoma query top-countries   --site <id|domain>
stoma query top-browsers    --site <id|domain>
stoma query top-devices     --site <id|domain>
stoma query top-os          --site <id|domain>
stoma query top-regions     --site <id|domain>
stoma query events          --site <id|domain> [--name signup] [--range 30d]
stoma query campaigns       --site <id|domain>
stoma query export-events   --site <id|domain> [--limit 1000]
stoma query export-sessions --site <id|domain> [--limit 1000]
stoma query funnels         --site <id|domain>
stoma query funnel          --site <id|domain> --funnel <funnel_id> [--range 30d]
stoma query funnel-create   --site <id|domain> --name <name> --steps '<json-array>'
```

`funnel-create` is the one write command available to **read API keys**: it
posts a new funnel and prints the created record (with its `id`) as JSON.
`--steps` is a JSON array of step objects with at least two entries.

```bash
stoma query funnel-create --site example.com --name "Signup flow" \
  --steps '[{"name":"Landing","event_name":"pageview","filters":[]},
            {"name":"Signup","event_name":"signup","filters":[]}]'
```

Run `stoma describe` for a machine-readable JSON manifest of every command and
argument — useful for wiring `stoma` into an LLM agent or MCP server.

### Agent skill

Install the bundled `stoma-analytics` skill globally so coding agents that support
`SKILL.md` can run `stoma` in any project session. By default this writes into the
common skill directories used by Claude Code, Grok, Cursor, and the generic
Agent Skills path:

```bash
stoma skills install                     # all known providers
stoma skills install --force             # overwrite existing installs
stoma skills install --provider claude   # one provider: claude|grok|cursor|agents
```

| Provider | Install path |
|----------|----------------|
| `claude` | `~/.claude/skills/stoma-analytics/` |
| `grok`   | `~/.grok/skills/stoma-analytics/` |
| `cursor` | `~/.cursor/skills/stoma-analytics/` |
| `agents` | `~/.agents/skills/stoma-analytics/` |

Restart the agent session so it reloads skills.

## For LLM agents

To analyze a site's traffic:

1. Obtain a read key (`rk_…`) from the operator.
2. `GET /api/v1/sites` to discover site ids/domains in scope.
3. Query the endpoints above for the metrics you need; prefer `--range` windows
   that match the question (`7d` for recent trends, `12m` for year-over-year).
4. Custom events are queried via `…/events?name=<event>`; emit them via the
   ingest endpoint with an ingest key.
5. To define a new funnel, `POST …/funnels` (or `stoma query funnel-create`) with
   a read key — list its results afterward via the funnel endpoints.

All responses are JSON; errors use standard HTTP status codes (`401`
unauthenticated, `403` out of scope, `404` unknown site, `429` rate-limited).
