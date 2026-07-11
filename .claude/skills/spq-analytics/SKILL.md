---
name: spq-analytics
description: Query Stomatopod web analytics and create funnels from the command line via the `spq` CLI. Use when asked to inspect a site's traffic (pageviews, top pages/referrers, custom events), run or list funnels, or define a new conversion funnel. Read-only by default; the one write is funnel creation.
---

# Stomatopod analytics via `spq`

`spq` is a command-line client for the Stomatopod analytics JSON API, built for
LLM-agent use. It prints JSON on stdout by default (pass `--human` for a table).
It is a separate binary from the `stomatopod` server.

## Setup

Authenticate with a **read** API key (`rk_…`), minted in the dashboard under
**API Keys**. The same key can both query and create funnels.

```bash
export STOMATOPOD_TOKEN=rk_xxxxxxxx           # read API key
export STOMATOPOD_SERVER=https://your-host    # defaults to http://localhost:8080
```

`STOMATOPOD_TOKEN` may instead live in `~/.config/stomatopod/credentials`.

A key is confined to its organization (and to a single site if created
site-bound). Out-of-scope access returns `403`.

## Discover the surface

Don't guess flags — ask the CLI:

```bash
spq describe      # machine-readable JSON manifest of every command + args
```

## Conventions

- `--site` — a site **ULID or its domain** (e.g. `example.com`).
- `--range` — `7d`, `30d` (default), `90d`, `12m`. Match the window to the
  question: `7d` for recent trends, `12m` for year-over-year.
- `--from` / `--to` — explicit `YYYY-MM-DD` window; overrides `--range`.
- `--granularity` (pageviews/retention) — `hour`, `day`, `week`, `month`.
- `--limit` — rows for top-N commands (default `20`).
- `--filter` — repeatable `field:op:value` (e.g. `country:eq:US`); multiple
  filters are AND-combined. Fields: url, referrer, country, region, browser,
  os, device_type, utm_source, utm_medium, utm_campaign, utm_term, utm_content,
  event_name. Ops: eq, not_eq, contains, starts_with.
- `--compare` — flag that attaches prior-period comparison (`delta_pct`).
- Output is JSON unless `--human` is passed.

## Recommended workflow

1. `spq sites` — list sites in scope, get their ids/domains.
2. Run the query commands below for the metrics you need.
3. To measure a multi-step conversion, create a funnel then run it.

## Query commands

```bash
spq sites                                                       # sites in scope
spq query pageviews         --site <id|domain> [--range 30d] [--granularity day] [--compare] [--filter f:op:v]
spq query top-pages         --site <id|domain> [--range 30d] [--limit 20] [--compare] [--filter f:op:v]
spq query top-referrers     --site <id|domain> [--range 30d] [--limit 20]
spq query top-os            --site <id|domain> [--range 30d] [--limit 20]
spq query top-regions       --site <id|domain> [--range 30d] [--limit 20]
spq query top-entry-pages   --site <id|domain> [--range 30d] [--limit 20]
spq query top-exit-pages    --site <id|domain> [--range 30d] [--limit 20]
spq query events            --site <id|domain> [--name signup] [--range 30d] [--filter f:op:v]
spq query campaigns         --site <id|domain> [--range 30d] [--limit 20]
spq query utm               --site <id|domain> --dimension source|medium|campaign|term|content [--utm-source s] [--utm-medium m]
spq query paths             --site <id|domain> [--steps 3] [--start-url /pricing] [--limit 25]
spq query retention         --site <id|domain> [--granularity week|month] [--range 90d]
spq query realtime          --site <id|domain>
spq query vitals            --site <id|domain> [--url /home] [--range 30d]
spq query scroll            --site <id|domain> [--url /home] [--range 30d]
spq query search            --site <id|domain> [--range 30d] [--limit 20]
spq query revenue           --site <id|domain> [--range 30d]
spq query revenue-breakdown --site <id|domain> --dimension referrer|country|utm_source [--range 30d]
spq query experiments       --site <id|domain> [--range 30d]
spq query experiment        --site <id|domain> --experiment <name> [--goal <goal_id>] [--range 30d]
spq query funnels           --site <id|domain>                  # list funnels
spq query funnel            --site <id|domain> --funnel <id> [--range 30d]   # run one
```

## Management commands (require a write-capable key)

```bash
spq goals list          --site <id|domain>
spq goals create        --site <id|domain> --name "Signup" --event user_signed_up [--filter plan:eq:pro]
spq goals delete        --site <id|domain> --goal <id>

spq alerts list         --site <id|domain>
spq alerts create       --site <id|domain> --type traffic_spike --threshold 200 --window 60 --channel <channel_id>
spq alerts delete       --site <id|domain> --alert <id>
spq alerts toggle       --site <id|domain> --alert <id> --enabled true|false

spq annotations list    --site <id|domain> [--range 90d]
spq annotations create  --site <id|domain> --date 2025-06-03 --label "Launched v2.0" [--note "HN post + email"]
spq annotations delete  --site <id|domain> --id <id>
```

## Creating a funnel (the one write)

A funnel is a named, ordered list of steps (≥ 2). Each step:

| Field        | Meaning                                            |
|--------------|----------------------------------------------------|
| `name`       | Step label, e.g. `"Signup"`.                       |
| `event_name` | Event satisfying the step, e.g. `"pageview"`.      |
| `filters`    | Property filters; `[]` for none.                   |

```bash
spq query funnel-create --site example.com --name "Signup flow" \
  --steps '[{"name":"Landing","event_name":"pageview","filters":[]},
            {"name":"Signup","event_name":"signup","filters":[]}]'
```

`--steps` is a JSON array string. The command prints the created funnel as JSON,
including its `id` — use that id with `spq query funnel --funnel <id>` to see
conversion/drop-off per step.

Equivalent raw API call:

```bash
curl -X POST $STOMATOPOD_SERVER/api/v1/sites/example.com/funnels \
  -H "Authorization: Bearer $STOMATOPOD_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"Signup flow","steps":[
        {"name":"Landing","event_name":"pageview","filters":[]},
        {"name":"Signup","event_name":"signup","filters":[]}]}'
```

## Errors

Standard HTTP status codes surface as CLI errors: `401` unauthenticated,
`403` out of scope, `404` unknown site/funnel, `400` bad input (e.g. invalid
`--steps` JSON or fewer than 2 steps), `429` rate-limited.
