# Core Web Vitals / Performance Metrics

## Problem
Analytics tracks visitor behavior but nothing about the actual experience quality — page load speed, layout stability, input responsiveness. Slow pages drive bounce but this correlation is invisible.

## Goal
Capture LCP, CLS, and INP from the browser tracker. Report p50/p75/p95 per page and over time. Show alongside pageview data so performance regressions are visible.

## Metrics
| Metric | What it measures | Good threshold |
|--------|-----------------|---------------|
| LCP (Largest Contentful Paint) | Load speed | < 2.5s |
| CLS (Cumulative Layout Shift) | Visual stability | < 0.1 |
| INP (Interaction to Next Paint) | Responsiveness | < 200ms |

## Tracker Changes (`assets/tracker.js`)
Use the [Web Vitals JS library](https://github.com/GoogleChrome/web-vitals) or native `PerformanceObserver` APIs:

```js
// After page load, collect vitals and send as special event
import { onLCP, onCLS, onINP } from 'web-vitals';

function sendVital(name, value, rating) {
  sendEvent('__vital__', {
    metric: name,        // 'LCP' | 'CLS' | 'INP'
    value: value,        // numeric
    rating: rating,      // 'good' | 'needs-improvement' | 'poor'
    url: location.pathname
  });
}

onLCP(({ name, value, rating }) => sendVital(name, value, rating));
onCLS(({ name, value, rating }) => sendVital(name, value, rating));
onINP(({ name, value, rating }) => sendVital(name, value, rating));
```

Vitals stored as custom events with name `__vital__` — no schema change required.

## API Changes

### Aggregated vitals report
```
GET /api/v1/sites/{site}/vitals?range=30d
GET /api/v1/sites/{site}/vitals?range=30d&url=/pricing
```

Response:
```json
{
  "lcp": { "p50": 1.8, "p75": 2.4, "p95": 4.1, "good_pct": 72 },
  "cls": { "p50": 0.04, "p75": 0.09, "p95": 0.21, "good_pct": 85 },
  "inp": { "p50": 120, "p75": 185, "p95": 340, "good_pct": 68 }
}
```

### Per-page vitals breakdown
```
GET /api/v1/sites/{site}/vitals/pages?range=30d&metric=lcp&limit=20
```
Returns top pages ranked by worst p75 LCP (most impactful to fix).

## UI
- "Performance" tab in site dashboard
- Summary: three metric cards (LCP / CLS / INP) with p75 + color-coded rating (green/amber/red)
- Table: top pages by worst p75 LCP
- Timeseries: p75 per day over selected range (detect regressions after deploys)
- Filter by URL: click a page to see its vitals breakdown

## Edge Cases
- Vitals only available for browsers supporting Web Vitals APIs (modern browsers)
- CLS is reported after page unload — may miss some sessions
- Server-side rendered pages vs SPAs: INP may not fire on SSR-only pages
- `__vital__` events should NOT appear in regular custom events reports — filter by convention
- Vitals data may lag pageviews by minutes (observer callbacks fire late)
