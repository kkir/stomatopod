# Deploying Stomatopod

## Data persistence (read this first)

With the default **embedded** storage backend, Stomatopod keeps *all* state under
`storage.data_dir` (default `/app/data` in the container): the SQLite metadata
database (organizations, users, sites, API keys, funnels, goals), the write-ahead
logs, and the Parquet event/span files.

Inside a container that directory lives in the writable image layer **unless you
mount a volume over it**. Without a volume, every redeploy starts from an empty
data dir — all analytics history is lost.

To prevent silent data loss, the server **refuses to start** when it detects it is
running in a container with `data_dir` on ephemeral storage. Mount a persistent
volume (below), use the `postgres` backend, or explicitly opt into ephemeral
storage with `STOMATOPOD_STORAGE__ALLOW_EPHEMERAL=true`.

**What to back up / snapshot:** the entire `data_dir` tree (`/app/data`).

## docker compose (recommended for self-hosting)

A ready-to-use [`docker-compose.yml`](./docker-compose.yml) ships in the repo with a
named volume:

```bash
export STOMATOPOD_AUTH__SECRET_KEY="$(openssl rand -hex 32)"
docker compose up -d
```

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
and point Stomatopod at it with `STOMATOPOD_STORAGE__DATA_DIR`.

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
- `STOMATOPOD_BASE_URL` — public URL for share links and email digests
  (e.g. `https://analytics.example.com`).

See [`stomatopod.example.toml`](./stomatopod.example.toml) for the full reference.

## Escape hatch (ephemeral on purpose)

For throwaway demos or ephemeral test containers, bypass the persistence guard:

```bash
-e STOMATOPOD_STORAGE__ALLOW_EPHEMERAL=true
```

Data will not survive a redeploy when this is set.
