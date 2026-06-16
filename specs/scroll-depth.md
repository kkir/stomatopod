# Scroll Depth & Engagement

## Problem
Pageviews count visits but not whether content was actually read. A user who bounced after 2 seconds counts the same as one who read to the bottom.

## Goal
Track how far users scroll on each page. Report scroll depth percentiles per page to identify where readers drop off.

## Approach
Track scroll depth milestones (25%, 50%, 75%, 100%) as custom events from the tracker. Per-page aggregation shows engagement quality.

## Tracker Changes (`assets/tracker.js`)
```js
const milestones = [25, 50, 75, 100];
const reached = new Set();

function getScrollPct() {
  const el = document.documentElement;
  return Math.round((window.scrollY + window.innerHeight) / el.scrollHeight * 100);
}

window.addEventListener('scroll', () => {
  const pct = getScrollPct();
  for (const m of milestones) {
    if (pct >= m && !reached.has(m)) {
      reached.add(m);
      sendEvent('__scroll__', { depth: m, url: location.pathname });
    }
  }
}, { passive: true });
```

Events stored as custom events with name `__scroll__` — no schema change.

## API Changes
```
GET /api/v1/sites/{site}/scroll?range=30d
GET /api/v1/sites/{site}/scroll?range=30d&url=/blog/my-post
```

Response:
```json
{
  "url": "/blog/my-post",
  "sessions_with_scroll_data": 840,
  "reached_25pct": 92.4,
  "reached_50pct": 71.2,
  "reached_75pct": 48.6,
  "reached_100pct": 31.1
}
```

### Per-page scroll rankings
```
GET /api/v1/sites/{site}/scroll/pages?range=30d&limit=20
```
Returns pages ranked by `reached_100pct` descending (most fully-read content).

## UI
- Scroll data shown on "Top Pages" detail view (click through from main dashboard)
- Visual: horizontal bar divided at 25/50/75/100 marks with fill % for each milestone
- Optional: "Most engaged pages" widget on main dashboard (pages where >50% of visitors reach 75%)

## Edge Cases
- Short pages: 100% scroll may be reached without meaningful reading — consider minimum height threshold (> 2x viewport)
- SPA navigation: reset milestones on route change
- `__scroll__` events excluded from custom events reports (same convention as `__vital__`)
- Page with no scroll events: show "No data" not 0% (could be a non-scrollable page)
