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
mise run setup
mise run config:init
```

Then set `auth.secret_key` in `stomatopod.toml`, or export:

```bash
export STOMATOPOD_AUTH__SECRET_KEY="replace-with-a-long-random-secret"
```

## Daily commands

```bash
mise run dev          # build assets + run local server
mise run assets:watch # Tailwind watch mode
mise run check        # fmt + clippy + tests
mise run test         # Rust tests only
mise run e2e          # Playwright smoke tests
```

## Deployment

See [`DEPLOY.md`](./DEPLOY.md) for running Stomatopod in production, including the
**data persistence** requirement (mount a volume at `/app/data`) and ready-to-use
[`docker-compose.yml`](./docker-compose.yml).

## Notes

- `mise run dev` fails fast when auth config is missing.
- Rust build/run requires generated `assets/dist/marketing.css` and `assets/vendor/anime.min.js`; `assets` is wired into task dependencies for `dev`, `lint`, `test`, and `e2e`.
