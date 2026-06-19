# Revenue / E-commerce Tracking

## Problem
No way to connect analytics to business outcomes. Traffic and sessions are measured but revenue, orders, and customer value are invisible.

## Goal
Track revenue-bearing events and report on total revenue, order count, average order value (AOV), and revenue per session over time.

## Tracking Pattern
Teams emit a special event with a `revenue` property:
```js
// On purchase completion
stomatopod.track('purchase', {
  revenue: 49.99,        // required: numeric, USD or configured currency
  currency: 'USD',       // optional, defaults to site currency setting
  order_id: 'ord_123',   // optional, for deduplication
  product: 'Pro Plan'    // optional, additional context
});
```

No schema changes — `revenue` is a convention on properties JSON. Server validates it's a positive number.

## Site Configuration
Add optional `currency` setting to site config (default: USD). Used for display only — storage is raw numeric.

## API Changes

### Revenue summary
```
GET /api/v1/sites/{site}/revenue?range=30d
```
Response:
```json
{
  "total_revenue": 12480.50,
  "orders": 248,
  "aov": 50.32,
  "revenue_per_session": 3.90,
  "currency": "USD"
}
```

### Revenue timeseries
```
GET /api/v1/sites/{site}/revenue/timeseries?range=30d&granularity=day
```
Returns daily revenue + order count buckets.

### Top pages by revenue
```
GET /api/v1/sites/{site}/revenue/pages?range=30d&limit=20
```
Last page before purchase event per session — identifies highest-value conversion paths.

### Revenue by dimension
```
GET /api/v1/sites/{site}/revenue/breakdown?dimension=referrer&range=30d
GET /api/v1/sites/{site}/revenue/breakdown?dimension=country&range=30d
GET /api/v1/sites/{site}/revenue/breakdown?dimension=utm_source&range=30d
```

## Deduplication
If `order_id` property provided: deduplicate on `(site_id, order_id)` — don't double-count repeat events for the same order (e.g., SPA retry or page refresh).

## UI
- Revenue cards added to main dashboard when revenue events detected: Total Revenue, Orders, AOV
- "Revenue" tab: timeseries chart + breakdown tables
- Top referrers table gains "Revenue" and "Revenue/session" columns
- Goals: if a goal event has `revenue` property, show total revenue attributed to goal

## CLI Changes
```
spq query revenue --site <id> [--range]
spq query revenue-breakdown --site <id> --dimension referrer [--range]
```

## Edge Cases
- Negative revenue (refunds): accept and subtract from totals. Store as-is.
- Multi-currency: store raw value + currency, convert at query time using static exchange rates (not live). Or report per-currency separately.
- Revenue without `order_id`: each event counted independently (no deduplication)
- Very high values (> 1,000,000): accept — B2B deals are large
