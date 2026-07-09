# SPQ CLI Enhancements

## Problem
`spq` covers core queries but gaps exist: no filtering, no period comparison, no access to new report types (entry/exit pages, UTM, etc.), and no alert management from the CLI.

## Goal
Extend `spq` to expose all analytics features and make it fully usable as a first-class interface for power users, scripts, and AI agents.

## New Query Commands

### From new feature specs
```
spq query top-os              --site <id> [--range] [--limit]
spq query top-regions         --site <id> [--range] [--limit]
spq query top-entry-pages     --site <id> [--range] [--limit]
spq query top-exit-pages      --site <id> [--range] [--limit]
spq query utm                 --site <id> --dimension source|medium|campaign|term|content [--range] [--limit] [--utm-source <s>] [--utm-medium <m>]
spq query campaigns           --site <id> [--range] [--limit]
```

## Enhanced Existing Commands

### Filtering (`--filter`)
Add `--filter` flag to all query commands:
```
spq query top-pages --site <id> --filter "country:eq:US" --filter "device_type:eq:mobile"
```
Syntax: `field:op:value` — maps to `FilterField`/`FilterOp` types.
Multiple `--filter` flags = AND.

### Period comparison (`--compare`)
```
spq query pageviews --site <id> --range 30d --compare
spq query top-pages --site <id> --range 30d --compare
```
Adds `delta_pct` fields to output JSON/table.

### Custom date range (`--from` / `--to`)
```
spq query pageviews --site <id> --from 2025-01-01 --to 2025-01-31
```
Replaces `--range` preset when specified.

## New Management Commands

### Alerts
```
spq alerts list    --site <id>
spq alerts create  --site <id> --type traffic_spike --threshold 200 --window 60 --channel <channel_id>
spq alerts delete  --site <id> --alert <id>
spq alerts toggle  --site <id> --alert <id> --enabled true|false
```

### Share Links
```
spq share list    --site <id>
spq share create  --site <id> [--label "Client view"] [--expires 2025-12-31]
spq share revoke  --site <id> --link <id>
```

## Output Format Consistency
All commands follow existing convention:
- Default: JSON (machine-readable)
- `--human`: formatted table
- `--limit N`: row limit (default varies per command)

## `spq describe` Update
Update the machine-readable CLI manifest to include all new commands so AI agents (Claude, etc.) auto-discover them.

## Implementation Notes
All new commands are thin wrappers over new API endpoints. Pattern is consistent with existing query commands in `bin/spq/src/query.rs` — add match arm per command, build request params, print JSON response.
