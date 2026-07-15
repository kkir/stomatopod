# stoma

Command-line client for the Stomatopod analytics API. Emits JSON by default
(`--human` for tables). Separate from the `stomatopod` server binary.

## Install

Requires [Rust](https://rustup.rs) and [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall):

```bash
# one-time: install cargo-binstall
curl -L --proto '=https' --tlsv1.2 -sSf \
  https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash

# install stoma (prebuilt when a matching GitHub release exists; otherwise builds from source)
cargo binstall --git https://github.com/kkir/stomatopod stomatopod-cli
```

From a local checkout of this monorepo:

```bash
cargo binstall --manifest-path bin/stoma --locked stomatopod-cli
# or: cargo install --path bin/stoma
```

## Configure

```bash
export STOMATOPOD_TOKEN=rk_xxxxxxxx            # read API key from the dashboard
export STOMATOPOD_SERVER=https://your-host     # default: http://localhost:8080
```

`STOMATOPOD_TOKEN` may instead live in `~/.config/stomatopod/credentials`.

## Quick start

```bash
stoma sites
stoma describe
stoma skills install   # optional: agent skill for Claude / Grok / Cursor
```

Full command reference: repo-root `assets/docs.md` (also `/docs` and `/llms.txt`
on a running server).
