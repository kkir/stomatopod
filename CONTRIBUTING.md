# Contributing

Thanks for helping improve Stomatopod.

## Prerequisites

1. Install [mise](https://mise.jdx.dev/getting-started.html).
2. From the repo root:

```bash
mise install
mise run config:init
```

Set `auth.secret_key` in `stomatopod.toml` (or `STOMATOPOD_AUTH__SECRET_KEY`),
a strong first-boot `STOMATOPOD_ADMIN_PASSWORD` (min 12 characters), and
`STOMATOPOD_ADMIN_EMAIL`:

```bash
export STOMATOPOD_AUTH__SECRET_KEY="$(openssl rand -hex 32)"
export STOMATOPOD_ADMIN_PASSWORD="$(openssl rand -base64 24)"
export STOMATOPOD_ADMIN_EMAIL=you@example.com
```

## Daily commands

```bash
mise run dev          # wasm client + server (dashboard on :8080)
mise run check        # fmt + clippy + tests
mise run test         # Rust tests
mise run e2e          # Playwright smoke tests
mise run seed         # demo traffic into a running dev server
```

## Pull requests

- Run `mise run check` before opening a PR.
- Prefer small, focused changes with clear commit messages.
- Match existing style (Rust 2021, workspace deps, no em dashes in prose).
- New storage backends should implement `StorageBackend` and `MetaStore` in
  `stomatopod-core` rather than bypassing those traits.

## License

By contributing, you agree that your contributions are licensed under the MIT
License (see `LICENSE`).
