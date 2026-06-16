# Heatmaps

## Problem
Analytics shows which pages get traffic but not what visitors do on those pages — where they click, how far they scroll, what they interact with.

## Goal
Click heatmaps and scroll heatmaps overlaid on a live preview of any page. Shows hot zones (many clicks) and cold zones (ignored areas).

## Heatmap Types
- **Click heatmap**: density of click/tap events on the page, shown as colored overlay
- **Scroll heatmap**: % of sessions that scrolled to each vertical position

## Tracker Changes (`assets/tracker.js`)
```js
// Click tracking
document.addEventListener('click', (e) => {
  sendEvent('__click__', {
    x: Math.round(e.clientX / window.innerWidth * 100),    // % of viewport width
    y: Math.round((e.clientY + window.scrollY) / document.documentElement.scrollHeight * 100),  // % of page height
    url: location.pathname,
    element: e.target.tagName + (e.target.id ? '#' + e.target.id : '')
  });
}, { passive: true });
```

Scroll depth already handled in scroll-depth.md — reuse `__scroll__` events for scroll heatmap.

## API Changes

### Click data for a page
```
GET /api/v1/sites/{site}/heatmaps/clicks?url=/pricing&range=30d
```
Response:
```json
{
  "url": "/pricing",
  "total_clicks": 2840,
  "points": [
    { "x": 52, "y": 34, "count": 48 }
  ]
}
```
Points are bucketed into a grid (e.g., 50×100 cells covering 0–100% x/y) before return to reduce payload.

### Scroll heatmap for a page
```
GET /api/v1/sites/{site}/heatmaps/scroll?url=/pricing&range=30d
```
Response:
```json
{
  "url": "/pricing",
  "sessions": 840,
  "scroll_distribution": [
    { "depth_pct": 0, "reached_pct": 100 },
    { "depth_pct": 25, "reached_pct": 92 },
    { "depth_pct": 50, "reached_pct": 71 },
    { "depth_pct": 75, "reached_pct": 48 },
    { "depth_pct": 100, "reached_pct": 31 }
  ]
}
```

## UI (Significant Frontend Work)

### Page selector
- "Heatmaps" tab in site dashboard
- URL search/select from known pages

### Heatmap viewer
- Embed target page in iframe (same-origin pages) or render screenshot
- Overlay canvas with WebGL or Canvas 2D heatmap rendering
- Toggle: Click heatmap / Scroll heatmap
- Color scale: blue (cold) → green → yellow → red (hot)
- Session count + date range shown

### Fallback (MVP)
If iframe embedding is blocked (cross-origin, CSP), show click data as a table of top-clicked elements by selector, without visual overlay.

## Edge Cases
- Cross-origin pages: cannot iframe — only table view available
- Dynamic pages (React SPAs): x/y coordinates may not match re-rendered layout
- Click coordinates are % of page height at time of click — page height changes (dynamic content) skew results
- Privacy: don't capture text input click positions — exclude `INPUT`, `TEXTAREA`, `SELECT` elements
- High-traffic pages: click point storage grows fast — bucket on ingest, not at query time

## Implementation Notes
Heatmaps require significant frontend investment (canvas rendering, iframe embedding) and have data volume concerns. Consider:
1. **MVP**: table of top-clicked elements (tagname + id/class) without visual overlay
2. **v2**: visual overlay for same-origin pages
3. **v3**: screenshot-based overlay for cross-origin pages
