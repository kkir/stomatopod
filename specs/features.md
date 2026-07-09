# Analytics Feature Brainstorm (historical)

Many items below were implemented after this brainstorm was written. Several
have since been removed to keep the product focused on self-hosted privacy
analytics (dashboard + API + CLI). Treat this file as historical context, not
a roadmap of what ships today.

## What remains product surface

- Pageviews timeseries, top-N breakdowns, filters, period comparison
- Custom events, goals, funnels
- Realtime, campaigns (UTM), paths
- Analytics alerts + channels, share links, email digests
- Annotations, data export, entry/exit pages
- `spq` CLI and `/llms.txt`

## Intentionally removed

- Sentinel / AI firewall (sidecar, spans, agents/incidents UI)
- Retention / cohort analysis (meaningless under daily cookieless session IDs)
- ClickHouse backend (unfinished, never routable)
- Tier-4 auto collectors and reports (vitals, scroll, heatmaps, site search, revenue, A/B)
- Global nav duplicates (site-centric IA only)
- SaaS Plan tiers (SelfHosted only)

## Specs still useful as design notes

See sibling files for goals, alerts, annotations, digests, share links, paths,
campaigns, filtering, period comparison, etc. Specs for removed features were
deleted with the code.
