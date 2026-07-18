#!/usr/bin/env bash
# Release-mode RSS + ingest RPS ladder for Stomatopod.
#
# Scenarios: idle → warm seed → moderate (~10 RPS + light queries) →
# 100 RPS → settle → 1000 RPS → settle.
#
# Usage:
#   mise run bench:memory
#   BENCH_DURATION_S=30 bash scripts/bench-memory.sh
#   BENCH_RPS_TIERS="10,100" bash scripts/bench-memory.sh   # skip 1000
#
# Artifacts: target/bench/memory/<run-id>/{report.md,report.json,samples.csv,...}

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# ---------------------------------------------------------------------------
# Tunables
# ---------------------------------------------------------------------------
BENCH_DURATION_S="${BENCH_DURATION_S:-60}"
BENCH_IDLE_S="${BENCH_IDLE_S:-20}"
BENCH_SETTLE_S="${BENCH_SETTLE_S:-15}"
BENCH_WARM_EVENTS="${BENCH_WARM_EVENTS:-25000}"
BENCH_WARM_DAYS="${BENCH_WARM_DAYS:-30}"
# Comma-separated ingest RPS tiers after warm (moderate is first if present).
BENCH_RPS_TIERS="${BENCH_RPS_TIERS:-10,100,1000}"
BENCH_MODERATE_QUERIES="${BENCH_MODERATE_QUERIES:-1}" # 1 = query mix during first tier
BENCH_PORT="${BENCH_PORT:-}" # empty = pick free port

ADMIN_EMAIL="${STOMATOPOD_ADMIN_EMAIL:-admin@bench.test}"
ADMIN_PASSWORD="${STOMATOPOD_ADMIN_PASSWORD:-bench-admin-pw-ok}"
AUTH_SECRET="${STOMATOPOD_AUTH__SECRET_KEY:-bench-memory-secret-key-32b}"

SERVER_BIN="${STOMATOPOD_BIN:-$ROOT/target/dx/stomatopod/release/web/server}"
PUBLIC_PATH="${DIOXUS_PUBLIC_PATH:-$ROOT/target/dx/stomatopod/release/web/public}"
SEED_BIN="${SEED_BIN:-}"

RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
RUN_DIR="${BENCH_OUT_DIR:-$ROOT/target/bench/memory/$RUN_ID}"
mkdir -p "$RUN_DIR"
SAMPLES="$RUN_DIR/samples.csv"
LOG_SERVER="$RUN_DIR/server.log"
LOG_SEED="$RUN_DIR/seed.log"
SCENARIO_FILE="$RUN_DIR/scenario"
COOKIE_JAR="$RUN_DIR/cookies.txt"
REPORT_MD="$RUN_DIR/report.md"
REPORT_JSON="$RUN_DIR/report.json"
RESULTS_TSV="$RUN_DIR/results.tsv"

HOST_OS="$(uname -s)"
HOST_ARCH="$(uname -m)"
HOST_CPU="$(sysctl -n machdep.cpu.brand_string 2>/dev/null || true)"
if [[ -z "$HOST_CPU" ]]; then
  HOST_CPU="$(grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2 | sed 's/^ //' || echo unknown)"
fi

echo "==> bench-memory run $RUN_ID"
echo "    out: $RUN_DIR"
echo "    duration/tier: ${BENCH_DURATION_S}s  idle: ${BENCH_IDLE_S}s  warm events: $BENCH_WARM_EVENTS"
echo "    RPS tiers: $BENCH_RPS_TIERS"
echo "    host: $HOST_OS $HOST_ARCH / $HOST_CPU"

# ---------------------------------------------------------------------------
# Ensure release server + seed binary
# ---------------------------------------------------------------------------
ensure_server() {
  if [[ -x "$SERVER_BIN" && -d "$PUBLIC_PATH" && "${BENCH_REBUILD:-0}" != "1" ]]; then
    echo "    using server: $SERVER_BIN"
    return
  fi
  echo "    building release fullstack app (ui:bundle)…"
  mise run ui:bundle
  SERVER_BIN="$ROOT/target/dx/stomatopod/release/web/server"
  PUBLIC_PATH="$ROOT/target/dx/stomatopod/release/web/public"
  if [[ ! -x "$SERVER_BIN" ]]; then
    echo "error: release server not found at $SERVER_BIN after ui:bundle" >&2
    exit 1
  fi
}

ensure_seed() {
  if [[ -n "$SEED_BIN" && -x "$SEED_BIN" ]]; then
    return
  fi
  echo "    building stomatopod-seed (release)…"
  cargo build -q -p stomatopod-seed --release
  SEED_BIN="$ROOT/target/release/stomatopod-seed"
  if [[ ! -x "$SEED_BIN" ]]; then
    # Some workspaces put release bins under target/<triple>/release
    SEED_BIN="$(find "$ROOT/target" -path '*/release/stomatopod-seed' -type f -perm -111 2>/dev/null | head -1 || true)"
  fi
  if [[ -z "${SEED_BIN:-}" || ! -x "$SEED_BIN" ]]; then
    echo "error: stomatopod-seed binary not found after build" >&2
    exit 1
  fi
  echo "    using seed: $SEED_BIN"
}

free_port() {
  if [[ -n "$BENCH_PORT" ]]; then
    echo "$BENCH_PORT"
    return
  fi
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

# ---------------------------------------------------------------------------
# RSS sampler (background)
# ---------------------------------------------------------------------------
# samples.csv: unix_ts,scenario,rss_kib
echo "unix_ts,scenario,rss_kib" > "$SAMPLES"
echo "idle" > "$SCENARIO_FILE"

SERVER_PID=""
SAMPLER_PID=""
QUERY_PID=""

cleanup() {
  local code=$?
  set +e
  if [[ -n "${QUERY_PID:-}" ]] && kill -0 "$QUERY_PID" 2>/dev/null; then
    kill "$QUERY_PID" 2>/dev/null || true
    wait "$QUERY_PID" 2>/dev/null || true
  fi
  if [[ -n "${SAMPLER_PID:-}" ]] && kill -0 "$SAMPLER_PID" 2>/dev/null; then
    kill "$SAMPLER_PID" 2>/dev/null || true
    wait "$SAMPLER_PID" 2>/dev/null || true
  fi
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    # Give it a moment, then force.
    sleep 1
    kill -9 "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  exit "$code"
}
trap cleanup EXIT INT TERM

start_sampler() {
  local pid="$1"
  (
    while kill -0 "$pid" 2>/dev/null; do
      local scen rss
      scen="$(cat "$SCENARIO_FILE" 2>/dev/null || echo unknown)"
      # macOS and Linux `ps -o rss=` report KiB.
      rss="$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ' || echo 0)"
      if [[ -n "$rss" && "$rss" != "0" ]]; then
        printf '%s,%s,%s\n' "$(date +%s)" "$scen" "$rss" >> "$SAMPLES"
      fi
      sleep 1
    done
  ) &
  SAMPLER_PID=$!
}

set_scenario() {
  echo "$1" > "$SCENARIO_FILE"
  echo "    scenario: $1"
}

# ---------------------------------------------------------------------------
# Config + server
# ---------------------------------------------------------------------------
ensure_server
ensure_seed

PORT="$(free_port)"
BASE="http://127.0.0.1:${PORT}"
DATA_DIR="$RUN_DIR/data"
mkdir -p "$DATA_DIR"

CONFIG="$RUN_DIR/stomatopod.toml"
cat > "$CONFIG" <<EOF
[auth]
secret_key = "${AUTH_SECRET}"
session_ttl_s = 2592000

[listen]
host = "127.0.0.1"
port = ${PORT}

[storage]
data_dir = "${DATA_DIR}"
allow_ephemeral = true
wal_fsync_interval_ms = 200
parquet_flush_rows = 50000
parquet_flush_interval_s = 30

[limits]
ingest_channel_size = 8192
ingest_batch_size = 1000
ingest_flush_interval_ms = 100
max_events_per_request = 10
EOF

echo "    starting server on $BASE …"
export DIOXUS_PUBLIC_PATH="$PUBLIC_PATH"
export STOMATOPOD_ADMIN_EMAIL="$ADMIN_EMAIL"
export STOMATOPOD_ADMIN_PASSWORD="$ADMIN_PASSWORD"
# First-boot bootstrap reads these env vars.
"$SERVER_BIN" --config "$CONFIG" serve >"$LOG_SERVER" 2>&1 &
SERVER_PID=$!

# Wait for readiness
ready=0
for _ in $(seq 1 90); do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "error: server exited early; see $LOG_SERVER" >&2
    tail -n 40 "$LOG_SERVER" >&2 || true
    exit 1
  fi
  if curl -sf "$BASE/ready" >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.5
done
if [[ "$ready" != "1" ]]; then
  echo "error: server never became ready; see $LOG_SERVER" >&2
  tail -n 40 "$LOG_SERVER" >&2 || true
  exit 1
fi
echo "    server ready (pid $SERVER_PID)"

start_sampler "$SERVER_PID"

# ---------------------------------------------------------------------------
# Bootstrap site + keys (session cookie)
# ---------------------------------------------------------------------------
curl -sf -c "$COOKIE_JAR" -b "$COOKIE_JAR" \
  -X POST "$BASE/login" \
  -H 'Content-Type: application/x-www-form-urlencoded' \
  --data-urlencode "email=${ADMIN_EMAIL}" \
  --data-urlencode "password=${ADMIN_PASSWORD}" \
  -o /dev/null -w '' || true

# Confirm session
if ! curl -sf -b "$COOKIE_JAR" "$BASE/api/v1/me" >/dev/null; then
  echo "error: login failed (check admin password / first-boot)" >&2
  tail -n 40 "$LOG_SERVER" >&2 || true
  exit 1
fi

SITE_JSON="$(curl -sf -b "$COOKIE_JAR" -X POST "$BASE/api/v1/sites" \
  -H 'Content-Type: application/json' \
  -d '{"domain":"bench.localhost","name":"Bench Site"}')"
PUBLIC_KEY="$(echo "$SITE_JSON" | python3 -c 'import sys,json; print(json.load(sys.stdin)["public_key"])')"
SITE_ID="$(echo "$SITE_JSON" | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])')"
echo "    site $SITE_ID public_key=${PUBLIC_KEY:0:12}…"

READ_JSON="$(curl -sf -b "$COOKIE_JAR" -X POST "$BASE/api/v1/keys" \
  -H 'Content-Type: application/json' \
  -d "{\"name\":\"bench-read\",\"scope\":\"read\",\"site_id\":\"${SITE_ID}\"}")"
READ_KEY="$(echo "$READ_JSON" | python3 -c 'import sys,json; print(json.load(sys.stdin)["secret"])')"

BINARY_BYTES="$(wc -c <"$SERVER_BIN" | tr -d ' ')"
echo "unix_ts	scenario	target_rps	ok	fail	achieved_rps	p50_ms	p99_ms	rss_med_mib	rss_max_mib	data_dir_mib" > "$RESULTS_TSV"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
rss_stats_for_scenario() {
  local scen="$1"
  python3 - "$SAMPLES" "$scen" <<'PY'
import sys
path, scen = sys.argv[1], sys.argv[2]
vals = []
with open(path) as f:
    next(f, None)
    for line in f:
        parts = line.strip().split(",")
        if len(parts) != 3:
            continue
        if parts[1] == scen:
            try:
                vals.append(int(parts[2]))
            except ValueError:
                pass
if not vals:
    print("0 0")
    sys.exit(0)
vals.sort()
med = vals[len(vals)//2]
mx = vals[-1]
# KiB → MiB
print(f"{med/1024:.2f} {mx/1024:.2f}")
PY
}

data_dir_mib() {
  python3 - "$DATA_DIR" <<'PY'
import os, sys
root = sys.argv[1]
total = 0
for dirpath, _, files in os.walk(root):
    for name in files:
        try:
            total += os.path.getsize(os.path.join(dirpath, name))
        except OSError:
            pass
print(f"{total/1024/1024:.2f}")
PY
}

parse_seed_stats() {
  # arg: path to seed log; prints: ok fail achieved_rps p50_ms p99_ms
  local logf="$1"
  python3 - "$logf" <<'PY'
import sys
path = sys.argv[1]
m = None
with open(path) as f:
    for line in f:
        if line.startswith("SEED_STATS "):
            m = line.strip()
kv = {}
if m:
    for part in m.split()[1:]:
        if "=" in part:
            k, v = part.split("=", 1)
            kv[k] = v
print(
    kv.get("ok", "0"),
    kv.get("fail", "0"),
    kv.get("achieved_rps", "0"),
    kv.get("p50_ms", "0"),
    kv.get("p99_ms", "0"),
)
PY
}

record_result() {
  local scen="$1" target_rps="$2" ok="$3" fail="$4" ach="$5" p50="$6" p99="$7"
  local rss
  rss="$(rss_stats_for_scenario "$scen")"
  local med max dd
  med="$(echo "$rss" | awk '{print $1}')"
  max="$(echo "$rss" | awk '{print $2}')"
  dd="$(data_dir_mib)"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$(date +%s)" "$scen" "$target_rps" "$ok" "$fail" "$ach" "$p50" "$p99" "$med" "$max" "$dd" \
    >> "$RESULTS_TSV"
  echo "    result $scen: rss_med=${med}MiB rss_max=${max}MiB ach_rps=${ach} ok=${ok} fail=${fail} data=${dd}MiB"
}

start_query_loop() {
  (
    local paths=(
      "pageviews?range=30d"
      "top-pages?range=30d&limit=10"
      "top-referrers?range=30d&limit=10"
      "top-browsers?range=30d&limit=10"
    )
    local i=0
    while true; do
      local p="${paths[$((i % ${#paths[@]}))]}"
      curl -sf -H "Authorization: Bearer ${READ_KEY}" \
        "$BASE/api/v1/sites/${SITE_ID}/${p}" -o /dev/null || true
      i=$((i + 1))
      sleep 2
    done
  ) &
  QUERY_PID=$!
}

stop_query_loop() {
  if [[ -n "${QUERY_PID:-}" ]] && kill -0 "$QUERY_PID" 2>/dev/null; then
    kill "$QUERY_PID" 2>/dev/null || true
    wait "$QUERY_PID" 2>/dev/null || true
  fi
  QUERY_PID=""
}

run_rps_tier() {
  local scen="$1"
  local rps="$2"
  local with_queries="${3:-0}"
  set_scenario "$scen"
  if [[ "$with_queries" == "1" ]]; then
    start_query_loop
  fi
  local logf="$RUN_DIR/seed-${scen}.log"
  set +e
  "$SEED_BIN" \
    --server "$BASE" \
    --public-key "$PUBLIC_KEY" \
    --rps "$rps" \
    --duration "$BENCH_DURATION_S" \
    --no-custom \
    --stats \
    >"$logf" 2>&1
  local rc=$?
  set -e
  cat "$logf" >> "$LOG_SEED"
  if [[ "$with_queries" == "1" ]]; then
    stop_query_loop
  fi
  local parsed
  parsed="$(parse_seed_stats "$logf")"
  # shellcheck disable=SC2086
  set -- $parsed
  record_result "$scen" "$rps" "$1" "$2" "$3" "$4" "$5"
  if [[ $rc -ne 0 ]]; then
    echo "    warning: seed exit $rc for $scen (see $logf)" >&2
  fi
}

# ---------------------------------------------------------------------------
# Ladder
# ---------------------------------------------------------------------------

# 1) Idle
set_scenario "idle"
sleep "$BENCH_IDLE_S"
record_result "idle" "-" "0" "0" "0" "0" "0"

# 2) Warm historical seed (keep scenario label through settle so RSS samples match)
set_scenario "warm"
echo "    warm seed: $BENCH_WARM_EVENTS events / $BENCH_WARM_DAYS days…"
set +e
"$SEED_BIN" \
  --server "$BASE" \
  --public-key "$PUBLIC_KEY" \
  --events "$BENCH_WARM_EVENTS" \
  --days "$BENCH_WARM_DAYS" \
  --no-custom \
  --stats \
  >"$RUN_DIR/seed-warm.log" 2>&1
set -e
cat "$RUN_DIR/seed-warm.log" >> "$LOG_SEED"
# Brief settle so parquet flush can progress (still labeled "warm")
sleep "$BENCH_SETTLE_S"
parsed="$(parse_seed_stats "$RUN_DIR/seed-warm.log")"
# shellcheck disable=SC2086
set -- $parsed
record_result "warm" "-" "$1" "$2" "$3" "$4" "$5"

# 3) RPS tiers
IFS=',' read -r -a TIERS <<< "$BENCH_RPS_TIERS"
first_tier=1
for rps in "${TIERS[@]}"; do
  rps="$(echo "$rps" | tr -d ' ')"
  [[ -z "$rps" ]] && continue
  scen="rps_${rps}"
  queries=0
  if [[ "$first_tier" == "1" && "$BENCH_MODERATE_QUERIES" == "1" ]]; then
    queries=1
    scen="moderate_rps_${rps}"
  fi
  first_tier=0
  run_rps_tier "$scen" "$rps" "$queries"
  set_scenario "settle_after_${scen}"
  sleep "$BENCH_SETTLE_S"
  record_result "settle_after_${scen}" "-" "0" "0" "0" "0" "0"
done

# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------
python3 - "$RUN_DIR" "$HOST_OS" "$HOST_ARCH" "$HOST_CPU" "$SERVER_BIN" "$BINARY_BYTES" \
  "$BENCH_DURATION_S" "$BENCH_WARM_EVENTS" "$BENCH_RPS_TIERS" "$BASE" <<'PY'
import csv, json, os, sys, datetime
from pathlib import Path

run_dir = Path(sys.argv[1])
host_os, host_arch, host_cpu = sys.argv[2], sys.argv[3], sys.argv[4]
server_bin, binary_bytes = sys.argv[5], int(sys.argv[6])
duration_s, warm_events, tiers, base = sys.argv[7], sys.argv[8], sys.argv[9], sys.argv[10]

results_path = run_dir / "results.tsv"
rows = []
with open(results_path) as f:
    r = csv.DictReader(f, delimiter="\t")
    for row in r:
        rows.append(row)

report = {
    "generated_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "host": {"os": host_os, "arch": host_arch, "cpu": host_cpu},
    "server_binary": server_bin,
    "binary_bytes": binary_bytes,
    "binary_mib": round(binary_bytes / 1024 / 1024, 2),
    "config": {
        "duration_s": int(duration_s),
        "warm_events": int(warm_events),
        "rps_tiers": tiers,
        "base_url": base,
    },
    "scenarios": rows,
}

(run_dir / "report.json").write_text(json.dumps(report, indent=2) + "\n")

def cell(row, k, default="-"):
    v = row.get(k, default)
    return v if v not in (None, "") else default

lines = []
lines.append(f"# Memory / RPS benchmark — {run_dir.name}")
lines.append("")
lines.append(f"- **Host:** {host_os} {host_arch} / {host_cpu}")
lines.append(f"- **Binary:** `{server_bin}` ({report['binary_mib']} MiB on disk)")
lines.append(f"- **Duration per RPS tier:** {duration_s}s")
lines.append(f"- **Warm seed:** {warm_events} pageviews")
lines.append(f"- **RPS tiers:** {tiers}")
lines.append(f"- **Base URL:** {base}")
lines.append("")
lines.append("| Scenario | Target RPS | Achieved RPS | OK | Fail | p50 ms | p99 ms | RSS med (MiB) | RSS max (MiB) | Data dir (MiB) |")
lines.append("|----------|------------|--------------|----|------|--------|--------|---------------|---------------|----------------|")
for row in rows:
    lines.append(
        "| {scen} | {tgt} | {ach} | {ok} | {fail} | {p50} | {p99} | {med} | {mx} | {dd} |".format(
            scen=cell(row, "scenario"),
            tgt=cell(row, "target_rps"),
            ach=cell(row, "achieved_rps"),
            ok=cell(row, "ok"),
            fail=cell(row, "fail"),
            p50=cell(row, "p50_ms"),
            p99=cell(row, "p99_ms"),
            med=cell(row, "rss_med_mib"),
            mx=cell(row, "rss_max_mib"),
            dd=cell(row, "data_dir_mib"),
        )
    )
lines.append("")
lines.append("RSS is process resident set size from `ps -o rss=` (KiB → MiB).")
lines.append("Re-run with `mise run bench:memory`. Artifacts in this directory.")
lines.append("")
(run_dir / "report.md").write_text("\n".join(lines))
print((run_dir / "report.md").read_text())
PY

echo ""
echo "==> done. report: $REPORT_MD"
echo "    json:   $REPORT_JSON"
echo "    samples: $SAMPLES"
