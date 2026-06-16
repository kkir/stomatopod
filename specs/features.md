# Analytics Feature Brainstorm

## Context
Stomatopod already has: pageviews timeseries, top-N breakdowns (pages/referrers/countries/browsers/devices), cookieless sessions, custom events + funnels, AI firewall/sentinel. Filtering infrastructure exists in code (`FilterField`/`FilterOp`) but not exposed in UI. Alert dispatcher exists (webhook/Slack) but wired only to AI firewall incidents. This is a brainstorm — no implementation yet.

---

## Tier 1 — High impact, low effort (existing infra, needs surface)

### 1. UI Filtering / Segmentation
`FilterField`/`FilterOp` already exists in `crates/core/src/query/pageviews.rs`. Just needs UI exposure.
- Filter dashboard by: country, browser, OS, device type, UTM source/medium/campaign, referrer
- Chips/pills UI — click a row in top-N to add as filter
- Persist in URL query params

### 2. Custom Date Range Picker
Currently: 7d / 30d / 90d / 12m presets only. Add arbitrary from/to date range.

### 3. Period Comparison
Show current period vs prior period side-by-side. "+12% vs last period" on timeseries + top-N rows.
- Requires no new data; two parallel queries.

### 4. Top OS + Top Regions
Data already captured (`os`, `region` columns). Not surfaced in top-N reports. Trivial addition.

---

## Tier 2 — High impact, moderate effort

### 5. Analytics Alerts
Alert dispatcher + webhook/Slack sinks already exist (`crates/web/src/alerts/`). Extend to analytics:
- Traffic spike / drop (% change vs prior period)
- Goal event threshold crossed (e.g., "100 signups today")
- New country/referrer spike
- 404 spike

### 6. Goals / Conversion Tracking
Mark a custom event as a "goal". Track goal completions + conversion rate over time on dashboard. Funnel steps already use event matching — goals are a simplified single-step version.

### 7. Entry / Exit Pages
Sessions already have `entry_url` / `exit_url` in session derivation. Just needs a dedicated report endpoint + UI tab.

### 8. Real-Time View
Last 30 minutes: active sessions, current page distribution, live event stream. Needs a short-window query + polling/SSE push.

### 9. Data Export
CSV/JSON download of any report. API already returns JSON — just wire a download button + Content-Disposition header.

---

## Tier 3 — Medium impact, higher effort

### 10. Retention / Cohort Analysis
Users (sessions) who came in week N and returned in week N+K. Classic retention grid. Needs cohort grouping query.

### 11. UTM Campaign Report
Dedicated view for UTM breakdowns: source → medium → campaign → content → term drill-down tree. Data already captured.

### 12. User Paths / Flow
Top N-step sequences (page A → page B → page C). Sankey or table. Needs windowed sequence query.

### 13. Annotations
Mark dates with a note (deployed X, launched campaign Y). Shown as vertical line on timeseries chart. Needs a simple `annotations` table.

### 14. Sparklines on Top-N
Mini 7-day trend chart next to each row in top-pages/referrers. Needs per-dimension timeseries sub-query.

### 15. Multi-Site Comparison
Compare traffic/events across sites in same org. Useful for teams running staging + prod or multiple products.

---

## Tier 4 — Nice to have, larger scope

### 16. Performance Metrics (Core Web Vitals)
Capture LCP, CLS, INP from browser tracker. Show p50/p75/p95 per page. Tracker already runs in browser — needs Web Vitals API calls + new event kind.

### 17. Scroll Depth / Engagement
% of page scrolled, time on page (not just session duration). Needs tracker-side JS + aggregation.

### 18. Public Dashboard Share Links
Shareable read-only dashboard URL (no auth). Token-scoped to site + read-only. Useful for clients / stakeholders.

### 19. Email Digest
Weekly/monthly report email. Needs job scheduler + email sender. Could reuse alert infrastructure.

### 20. A/B Test Tracking
Custom event property `variant=A|B` → compare conversion rates per variant. Funnels already support property filters — this is mostly a UI affordance.

### 21. Revenue / E-commerce Events
Tag events with `revenue` property. Track total revenue, AOV, LTV over time. Needs special aggregation + reporting.

### 22. Heatmaps
Click + scroll heatmaps per page. Requires canvas overlay renderer in dashboard. Significant frontend effort.

### 23. Site Search Tracking
Auto-detect or manually tag internal search queries. Report top search terms.

---

## SPQ CLI Enhancements

- `spq query entry-pages` / `exit-pages`
- `spq query retention`
- `spq query goals`
- `--compare-prev` flag on all queries (period-over-period delta)
- `--filter` flag (expose FilterField/FilterOp)
- `spq alerts create/list/delete`
