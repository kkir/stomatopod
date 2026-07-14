# stomatopod

Use [`mise` tasks](https://mise.jdx.dev/tasks/) for local development.

## Prerequisites

1. Install `mise` (see <https://mise.jdx.dev/getting-started.html>).
2. Install toolchain versions declared in `mise.toml`:

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

Stomatopod is a **single-owner** appliance (one org, one admin user, many sites).

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

The dashboard is a Dioxus 0.7 **fullstack** app that lives in the single
[`crates/web`](./crates/web) crate, styled with Tailwind CSS v4. The same code
compiles two ways: to wasm (the `web` feature — the hydrating client) and to
native (the `server` feature — the axum server that server-renders it). The
server serves the SSR'd dashboard at `/`, its hashed wasm/JS/CSS assets, and the
JSON API at `/api/v1`. `/login` is server-rendered HTML (no template engine).

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

## Deployment

See [`DEPLOY.md`](./DEPLOY.md) for running Stomatopod in production, including the
**data persistence** requirement (mount a volume at `/app/data`) and ready-to-use
[`docker-compose.yml`](./docker-compose.yml). The [`Dockerfile`](./Dockerfile)
installs `dx` and runs a single `dx build --platform web --release`, which
produces both the wasm client bundle and the native server binary, so
`docker build` needs no separate UI step.

## Notes

- `mise run dev` fails fast when auth config is missing.
- The `dx` CLI (`cargo:dioxus-cli`) and the `wasm32-unknown-unknown` target are
  required for the UI; `mise install` provisions `dx`, and `dx` installs the
  wasm target on first run.
