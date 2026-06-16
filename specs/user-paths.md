# User Paths / Flow

## Problem
Top pages shows which pages are popular but not how visitors navigate between them. No visibility into common journeys or drop-off points outside of explicit funnels.

## Goal
Show top N-step navigation sequences — the most common paths visitors take through the site. Surface as a ranked table or Sankey diagram.

## Definition
A path is an ordered sequence of distinct consecutive pages within a session (deduplicate consecutive same-page reloads). Analyze top 2-step and 3-step sequences.

Example paths:
- `/` → `/pricing` → `/signup` (420 sessions)
- `/blog/post-1` → `/` → `/pricing` (180 sessions)

## API Changes
```
GET /api/v1/sites/{site}/paths?steps=3&range=30d&limit=20
GET /api/v1/sites/{site}/paths?steps=2&start_url=/pricing&range=30d&limit=20
```

Params:
- `steps`: 2 or 3 (max 3 for performance)
- `start_url`: optional filter — only paths starting at this URL
- `end_url`: optional filter — only paths ending at this URL

Response:
```json
{
  "steps": 3,
  "paths": [
    {
      "sequence": ["/", "/pricing", "/signup"],
      "sessions": 420,
      "pct": 8.2
    }
  ]
}
```

## Query Strategy
1. For each session, extract ordered URL sequence (deduplicated consecutive)
2. Emit all N-grams of length `steps`
3. COUNT GROUP BY sequence
4. Return top K by count

This can be expensive — enforce:
- Max steps: 3
- Max range: 90 days
- Cache with 30-min TTL

## UI
- "Paths" tab in site dashboard
- Default: top 3-step paths, table view
- Step count toggle: 2 / 3
- Optional: Sankey diagram for visual flow (heavier frontend, lower priority)
- "Start from page" filter: type or click a URL from top-pages to see paths from there
- Each path row shows sequence as breadcrumb chips + session count + %

## CLI Changes
```
spq query paths --site <id> --steps 3 [--range] [--limit] [--start-url /pricing]
```

## Edge Cases
- Single-page sessions: no paths to emit (excluded)
- Very long sessions (100+ pageviews): truncate sequence at 20 pages before N-gram extraction
- URL normalization: strip query params and fragments by default (configurable)
