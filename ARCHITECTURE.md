# Architecture

Stomatopod is a **self-hosted, single-owner** web analytics appliance. Storage
is **embedded only** (SQLite metadata + WAL + Parquet on a local volume). The
workspace is split so library crates can be reused from a separate product
(for example a multi-tenant SaaS) without pulling in the dashboard or appliance
bootstrap.

## Crate map

```
stomatopod-core              domain, traits, query DTOs, errors, config
       │
       ├─ stomatopod-store           EmbeddedBackend (Parquet/WAL/SQLite/DataFusion)
       │
       └─ stomatopod-ingest          beacon handler, batcher, geo, UA
              │
              └─ stomatopod-alerts   evaluator, webhook/slack sinks, SSRF guards
                     │
                     └─ stomatopod-api    axum REST, auth middleware, state, digest, openapi
                            │
                            └─ stomatopod-web   Dioxus dashboard + `stomatopod` serve binary
                                   │
                                   └─ stomatopod-ui   shared components + theme CSS
                                          ▲
stomatopod-www  (SSG marketing / GitHub Pages) ──┘

bin/stoma, bin/seed          CLI and demo seeder
```

| Crate | Role |
|-------|------|
| `stomatopod-core` | `StorageBackend` / `MetaStore` traits, domain types, query DTOs |
| `stomatopod-store` | Self-host embedded engine only |
| `stomatopod-ingest` | Browser/server event ingest pipeline |
| `stomatopod-alerts` | Analytics alert evaluation and delivery |
| `stomatopod-api` | HTTP API and appliance auth (no Dioxus) |
| `stomatopod-ui` | Shared presentational Dioxus components and brand CSS |
| `stomatopod-web` | Dashboard UI and process wiring |
| `stomatopod-www` | Static marketing site (Dioxus SSG) |

## Trait boundary

All query and metadata access goes through:

- `stomatopod_core::traits::StorageBackend` - event write + analytics queries
- `stomatopod_core::traits::MetaStore` - sites, users, funnels, alerts, keys, digests

Ingest and API code hold `Arc<dyn StorageBackend>` / `Arc<dyn MetaStore>`. Concrete
backends are constructed only at process start (appliance: embedded store).

## Multi-tenant SaaS plug-in model

The **data model is org-aware** (`Organization`, `Site.org_id`, `User.org_id`,
API keys scoped by org/site). Analytics events are keyed by `site_id`. The
**product shell is single-owner**: first-boot creates one org and one admin;
handlers often resolve "the" org as the first listed org.

A private multi-tenant SaaS can:

1. Depend on `stomatopod-core` (and optionally `ingest` / `alerts`).
2. Implement `StorageBackend` + `MetaStore` against production storage.
3. Provide its own auth, org routing, billing, and UI.
4. Skip `stomatopod-store`, appliance bootstrap, and (usually) `stomatopod-web`.

```
  OSS (MIT)           core + ingest + alerts
                           │
  Private SaaS        implements StorageBackend + MetaStore
                      multi-tenant auth / API / UI

  OSS appliance       store (embedded) + api + web
                      single-owner bootstrap, cookie auth, Dioxus
```

**Isolation rule:** traits do not enforce tenancy. Every analytics call must
authorize `site_id` (and org membership) in the caller before using
`StorageBackend`. The appliance does this via `Principal` checks; SaaS should
do the same more strictly across many orgs.

## Deployment note

Self-host durability requires a **mounted data directory**. There is no
Postgres (or other remote) backend in this open-source tree.
