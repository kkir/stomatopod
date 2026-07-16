# Resource benchmarks

Stomatopod is meant to be **light enough to co-host** with the apps it measures:
one process, embedded storage, no external database. This document defines how
we measure process memory (RSS) and ingest throughput, and records a sample run.

Re-run on your machine:

```bash
mise run bench:memory
```

Artifacts land in `target/bench/memory/<run-id>/` (`report.md`, `report.json`,
`samples.csv`).

## Methodology

| Item | Detail |
|------|--------|
| Binary | Release fullstack server from `mise run ui:bundle` (`target/dx/stomatopod/release/web/server`) |
| RSS | `ps -o rss=` on the server PID every 1s (KiB, reported as MiB). Median and max over the scenario window |
| Loadgen | `stomatopod-seed` (`--rps` / `--duration` for sustained pageview POSTs to `/api/v1/event`) |
| Warm corpus | ~25k historical pageviews over 30 days before the RPS ladder |
| Config | Production-like flush defaults (see harness `stomatopod.toml`); isolated temp `data_dir` |

### Scenarios

1. **idle** - server past `/ready`, no traffic (~20s)
2. **warm** - bulk seed of historical events, then a short settle
3. **moderate (~10 RPS)** - sustained ingest at 10 req/s plus light dashboard queries every ~2s (`pageviews`, `top-pages`, `top-referrers`, `top-browsers`)
4. **100 RPS** - ingest-only sustained load
5. **1000 RPS** - ingest-only sustained load
6. Short **settle** samples after each RPS tier

Tunables (env):

| Variable | Default | Meaning |
|----------|---------|---------|
| `BENCH_DURATION_S` | `60` | Seconds per RPS tier |
| `BENCH_IDLE_S` | `20` | Idle sample window |
| `BENCH_SETTLE_S` | `15` | Post-tier settle |
| `BENCH_WARM_EVENTS` | `25000` | Historical pageviews before the ladder |
| `BENCH_RPS_TIERS` | `10,100,1000` | Comma-separated ingest targets |
| `BENCH_REBUILD` | `0` | Set `1` to force `ui:bundle` |

Sustained load alone:

```bash
cargo run -p stomatopod-seed --release -- \
  --public-key pk_… --rps 100 --duration 60 --no-custom --stats
```

## Sample results

Host and knobs for the checked-in sample (re-run for your hardware):

- **Host:** Darwin arm64 / Apple M5 Pro
- **Binary:** ~49 MiB on disk (release server)
- **Duration per RPS tier:** 30s
- **Warm seed:** 25000 pageviews / 30 days
- **Date:** 2026-07-16

| Scenario | Target RPS | Achieved RPS | OK | Fail | p50 ms | p99 ms | RSS med (MiB) | RSS max (MiB) | Data dir (MiB) |
|----------|------------|--------------|----|------|--------|--------|---------------|---------------|----------------|
| idle | - | - | - | - | - | - | 39.7 | 39.7 | 0.4 |
| warm | - | (bulk) | 25006 | 0 | 0.12 | 0.21 | 69.5 | 79.6 | 1.7 |
| moderate (~10 RPS + queries) | 10 | 10.0 | 299 | 0 | 0.30 | 0.67 | 82.5 | 93.4 | 1.8 |
| settle after moderate | - | - | - | - | - | - | 83.6 | 83.6 | 1.8 |
| 100 RPS ingest | 100 | 100.0 | 2999 | 0 | 0.17 | 0.45 | 83.1 | 83.6 | 2.1 |
| settle after 100 | - | - | - | - | - | - | 83.1 | 86.5 | 2.0 |
| 1000 RPS ingest | 1000 | 999.4 | 29990 | 0 | 0.10 | 0.88 | 84.5 | 123.1 | 4.0 |
| settle after 1000 | - | - | - | - | - | - | 99.1 | 99.1 | 4.0 |

### Takeaways

- **Idle RSS ~40 MiB** on this host - comfortable on a small VPS or the same box as a product API.
- **With history + light dashboard use, expect on the order of ~80-100 MiB** median RSS, not hundreds of MiB.
- **~100 and ~1000 pageview RPS** were sustained with zero failed requests on this laptop; peak RSS under 1000 RPS stayed around **~120 MiB**.
- Numbers are **illustrative**. Cold start, OS, CPU, GeoIP MMDB, and query mix all move RSS. Always re-run the harness for capacity planning.

## What this does not measure

- Multi-tenant SaaS deployments (this repo is the single-owner appliance)
- Query-only QPS ladders (ingest is the co-host path; add a query ladder later if needed)
- Heap profiles (`dhat` / `heaptrack`) - use those when optimizing, not for sizing claims
- Docker image RSS (optional follow-up: `docker stats` under the same scenarios)

## Related

- Deploy / volumes: [`DEPLOY.md`](./DEPLOY.md)
- Microbenches (CPU hot paths only):

  ```bash
  cargo bench -p stomatopod-ingest --bench hot_paths
  cargo bench -p stomatopod-store --bench record_batch
  ```
