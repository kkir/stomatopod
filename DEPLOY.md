# Deploying Stomatopod

## Data persistence (read this first)

With the default **embedded** storage backend, Stomatopod keeps *all* state under
`storage.data_dir` (default `/app/data` in the container): the SQLite metadata
database (organizations, users, sites, API keys, funnels), the write-ahead
logs, and the Parquet event files.

Inside a container that directory lives in the writable image layer **unless you
mount a volume over it**. Without a volume, every redeploy starts from an empty
data dir — all analytics history is lost.

To prevent silent data loss, the server **refuses to start** when it detects it is
running in a container with `data_dir` on ephemeral storage. Mount a persistent
volume (below), use the `postgres` backend, or explicitly opt into ephemeral
storage with `STOMATOPOD_STORAGE__ALLOW_EPHEMERAL=true`.

**What to back up / snapshot:** the entire `data_dir` tree (`/app/data`).

### Health probes

| Path | Purpose |
|------|---------|
| `GET /health` | Liveness - process is accepting HTTP |
| `GET /ready` | Readiness - meta store answers a cheap query |

Example for Kubernetes or Coolify:

```yaml
livenessProbe:
  httpGet: { path: /health, port: 8080 }
readinessProbe:
  httpGet: { path: /ready, port: 8080 }
```

### Retention

Optional. Set `storage.retention_days` (or `STOMATOPOD_STORAGE__RETENTION_DAYS`)
to drop events older than N days. `0` (default) keeps forever. The server prunes
once at boot and then daily.

## docker compose (recommended for self-hosting)

A ready-to-use [`docker-compose.yml`](./docker-compose.yml) ships in the repo with a
named volume:

```bash
export STOMATOPOD_AUTH__SECRET_KEY="$(openssl rand -hex 32)"
export STOMATOPOD_ADMIN_PASSWORD="$(openssl rand -base64 24)"
# optional: STOMATOPOD_ADMIN_EMAIL=you@example.com
docker compose up -d
```

Stomatopod is a **single-owner appliance**: one organization and one admin user
per instance (many sites under that org are fine).

Redeploy to a new image without losing data:

```bash
docker compose pull && docker compose up -d
```

`docker compose down` (without `-v`) stops the container but keeps the
`stomatopod_data` volume.

## docker run

Use a **named volume** (or a host bind mount) for `/app/data`:

```bash
docker run -d \
  -p 8080:8080 \
  -e STOMATOPOD_AUTH__SECRET_KEY="$(openssl rand -hex 32)" \
  -e STOMATOPOD_ADMIN_PASSWORD="$(openssl rand -base64 24)" \
  -v stomatopod_data:/app/data \
  ghcr.io/kkir/stomatopod:latest
```

Host bind mount instead of a named volume:

```bash
  -v /srv/stomatopod/data:/app/data
```

> An *anonymous* volume (`-v /app/data` with no name) is orphaned on
> `docker rm` + recreate, so it does **not** survive a redeploy. Always name the
> volume or bind-mount a host path.

## PaaS / Kubernetes (any platform)

Attach a persistent disk and either mount it at `/app/data`, or mount it elsewhere
and point Stomatopod at it with `STOMATOPOD_STORAGE__DATA_DIR`. The container
starts as root and `chown`s the mounted directory to its unprivileged user before
dropping privileges, so this works whether the platform gives you a Docker named
volume, a bind mount, or a plain host directory (e.g. Coolify's persistent
storage) — you don't need to pre-provision ownership yourself. The one exception:
if the platform forces a specific non-root UID/GID on the container (an explicit
`--user` or equivalent), the ownership fix-up is skipped and you're responsible
for provisioning correct ownership on that path.

- **Fly.io** — create a volume and mount it:
  ```toml
  [mounts]
  source = "stomatopod_data"
  destination = "/app/data"
  ```
- **Railway / Render** — add a persistent volume with the mount path `/app/data`.
- **Kubernetes** — back the Deployment with a `PersistentVolumeClaim` mounted at
  `/app/data`:
  ```yaml
  volumeMounts:
    - name: data
      mountPath: /app/data
  volumes:
    - name: data
      persistentVolumeClaim:
        claimName: stomatopod-data
  ```

## Alternative: external database

Set the storage backend to Postgres in `stomatopod.toml` (or via env) so durability
lives in a managed database — no volume required:

```toml
[storage]
backend = "postgres"
url = "postgresql://user:pass@host:5432/stomatopod"
```

## Required configuration

- `STOMATOPOD_AUTH__SECRET_KEY` — required; a long random string used to sign
  sessions. The server refuses to start without it.
- `STOMATOPOD_ADMIN_PASSWORD` — required on **first boot** (empty data dir);
  min 12 characters. Creates the single owner account. Optional
  `STOMATOPOD_ADMIN_EMAIL` (default `admin@localhost`).
- `STOMATOPOD_BASE_URL` — public URL for dashboard links in digests
  (e.g. `https://analytics.example.com`). Set this to an `https://` URL so the
  session cookie is marked `Secure`, or set `STOMATOPOD_AUTH__COOKIE_SECURE=true`.

### Optional security knobs

- `STOMATOPOD_AUTH__TRUST_FORWARDED_HEADERS` — default `true`. Set `false` if
  the process is exposed directly to the internet without a reverse proxy, so
  clients cannot spoof `X-Forwarded-For` / `X-Real-IP` for geo and session
  derivation.
- `STOMATOPOD_AUTH__COOKIE_SECURE` — force the session cookie `Secure` flag
  on/off instead of inferring from `base_url`.

See [`stomatopod.example.toml`](./stomatopod.example.toml) for the full reference.

## Escape hatch (ephemeral on purpose)

For throwaway demos or ephemeral test containers, bypass the persistence guard:

```bash
-e STOMATOPOD_STORAGE__ALLOW_EPHEMERAL=true
```

Data will not survive a redeploy when this is set.
