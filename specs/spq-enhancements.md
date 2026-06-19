# SPQ CLI Enhancements

## Problem
`spq` covers core queries but gaps exist: no filtering, no period comparison, no access to new report types (entry/exit pages, UTM, paths, vitals, etc.), and no alert management from the CLI.

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
spq query paths               --site <id> --steps 2|3 [--range] [--limit] [--start-url <url>]
spq query retention           --site <id> [--granularity week|month] [--range]
spq query realtime            --site <id>
spq query vitals              --site <id> [--range] [--url <path>]
spq query scroll              --site <id> [--range] [--url <path>]
spq query search              --site <id> [--range] [--limit]
spq query revenue             --site <id> [--range]
spq query revenue-breakdown   --site <id> --dimension referrer|country|utm_source [--range]
spq query experiments         --site <id> [--range]
spq query experiment          --site <id> --experiment <name> [--goal <goal_id>] [--range]
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

### Goals
```
spq goals list     --site <id>
spq goals create   --site <id> --name "Signup" --event user_signed_up [--filter "plan:eq:pro"]
spq goals delete   --site <id> --goal <id>
```

### Alerts
```
spq alerts list    --site <id>
spq alerts create  --site <id> --type traffic_spike --threshold 200 --window 60 --channel <channel_id>
spq alerts delete  --site <id> --alert <id>
spq alerts toggle  --site <id> --alert <id> --enabled true|false
```

### Annotations
```
spq annotations list    --site <id> [--range]
spq annotations create  --site <id> --date 2025-06-03 --label "Launched v2.0" [--note "HN post + email"]
spq annotations delete  --site <id> --annotation <id>
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
