# Stomatopod

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE)

Privacy-friendly, cookieless **web analytics** you run yourself. One binary (plus
a small wasm dashboard), embedded storage on a local volume, JSON API and CLI
for humans and agents.

Stomatopod is a **single-owner appliance** (one org, one admin user, many sites).
Licensed under **MIT**.

## Features

- Browser tracker (`/tracker.js`) and server-side ingest
- Dashboard (Dioxus fullstack) with pageviews, funnels, alerts, digests
- REST API + OpenAPI (`/openapi.json`) and `stoma` CLI
- Embedded storage: SQLite metadata, WAL, Parquet partitions (no external DB)

See [`ARCHITECTURE.md`](./ARCHITECTURE.md) for the crate map and how library
crates can plug into a separate multi-tenant product. Production SaaS storage
is intentionally **not** part of this repository.

## Prerequisites

1. Install [`mise`](https://mise.jdx.dev/getting-started.html).
2. Install toolchain versions from `mise.toml`:

```bash
mise install
```

## First-time setup

```bash
mise run config:init
```

Then set `auth.secret_key` in `stomatopod.toml`, or export:

```bash
export STOMATOPOD_AUTH__SECRET_KEY="replace-with-a-long-random-secret"
# First boot (empty data dir) also needs a strong owner password (min 12 chars):
export STOMATOPOD_ADMIN_PASSWORD="$(openssl rand -base64 24)"
```

## Daily commands

```bash
mise run dev          # build the wasm client + run the server (SSR dashboard at /, JSON at /api/v1)
mise run seed         # post demo traffic into a running dev server (bin/seed)
mise run check        # fmt + clippy + tests
mise run test         # Rust tests only
mise run e2e          # Playwright end-to-end tests
```

### Seeding demo data

With `mise run dev` already running, seed ~30 days of pageviews (and a few custom events):

```bash
export STOMATOPOD_ADMIN_PASSWORD='your-admin-password'   # same password used on first boot
mise run seed
```

Or point at an existing site without logging in:

```bash
STOMATOPOD_PUBLIC_KEY='pk_…' mise run seed
```

Useful knobs: `SEED_DAYS` / `--days` (default 30), `SEED_EVENTS` / `--events`
(default 2500), `STOMATOPOD_SERVER` / `--server` (default `http://localhost:8080`).
See `cargo run -p stomatopod-seed -- --help` for the full list.

## Dashboard UI (Dioxus fullstack)

The dashboard is a Dioxus 0.7 **fullstack** app in [`crates/web`](./crates/web),
styled with Tailwind CSS v4. The same code compiles two ways: to wasm (the `web`
feature - the hydrating client) and to native (the `server` feature - the axum
server that server-renders it). The server serves the SSR'd dashboard at `/`,
hashed wasm/JS/CSS assets, and the JSON API at `/api/v1`. `/login` is
server-rendered HTML.

```bash
# Build the wasm client, then run the server against it (SSR + hydration):
mise run ui:build     # dx build --platform web → target/dx/stomatopod/debug/web
mise run dev          # server on :8080, DIOXUS_PUBLIC_PATH set to the bundle above
```

`mise run dev` builds the client first (via `ui:build`) and points the server
at it with `DIOXUS_PUBLIC_PATH`. For production/CI, `mise run ui:bundle`
(`dx build --platform web --release`) emits `target/dx/stomatopod/release/web/`
containing the `server` binary next to its `public/` bundle; the Docker build
copies both.

## Marketing site (Dioxus SSG)

The public marketing site lives in [`crates/www`](./crates/www) and shares brand
tokens and presentational components with the dashboard via
[`crates/ui`](./crates/ui) (`stomatopod-ui`). It is pre-rendered with Dioxus SSG
and deployed to **GitHub Pages** (site root / custom-domain ready).

```bash
mise run www:serve     # local dev with hot reload
mise run www:bundle    # release SSG (dx build --ssg) → target/dx/stomatopod-www/release/web/public
```

CI deploys on push to `main` when `crates/www` or `crates/ui` change
(`.github/workflows/pages.yml`). Enable Pages with **Source: GitHub Actions**
in the repository settings.

## Workspace crates

| Crate | Path | Role |
|-------|------|------|
| `stomatopod-core` | `crates/core` | Domain, traits, config |
| `stomatopod-store` | `crates/store` | Embedded analytics storage |
| `stomatopod-ingest` | `crates/ingest` | Event ingest pipeline |
| `stomatopod-alerts` | `crates/alerts` | Alert evaluation and delivery |
| `stomatopod-api` | `crates/api` | REST API (no UI) |
| `stomatopod-ui` | `crates/ui` | Shared design system (components + theme CSS) |
| `stomatopod-web` | `crates/web` | Dashboard + server binary |
| `stomatopod-www` | `crates/www` | Marketing site (SSG / GitHub Pages) |
| `stomatopod-cli` | `bin/stoma` | `stoma` query CLI |

## Deployment

See [`DEPLOY.md`](./DEPLOY.md) for production, including the **data persistence**
requirement (mount a volume at `/app/data`) and
[`docker-compose.yml`](./docker-compose.yml). The [`Dockerfile`](./Dockerfile)
installs `dx` and runs a single `dx build --platform web --release`.

## Contributing

See [`CONTRIBUTING.md`](./CONTRIBUTING.md) and [`SECURITY.md`](./SECURITY.md).

## License

[MIT](./LICENSE) - Copyright (c) 2026 Kirill Kayer.
