# Multi-Site Comparison

## Problem
Teams running multiple sites (staging + production, regional variants, multiple products) have to switch between dashboards and mentally compare numbers. No unified view.

## Goal
Side-by-side comparison of key metrics across multiple sites in the same organization.

## Scope
Read-only comparison view. Not a merged analytics view — each site's data remains separate. Useful for:
- Staging vs prod traffic sanity check
- Comparing product lines
- Agency managing multiple client sites

## API Changes
```
GET /api/v1/orgs/{org}/compare?sites=site_a,site_b,site_c&range=30d
```

Response:
```json
{
  "range": "30d",
  "sites": [
    {
      "site_id": "...",
      "domain": "example.com",
      "pageviews": 12400,
      "sessions": 3200,
      "bounce_rate": 38.2,
      "avg_session_duration_s": 142,
      "top_country": "US",
      "top_referrer": "google.com"
    }
  ]
}
```

## UI
- "Compare sites" option in org-level navigation (not inside a single site)
- Site selector: checkboxes to pick 2–5 sites from the org
- Comparison table: sites as columns, metrics as rows
- Sparkline row: 30-day pageview trend per site (small, aligned by date)
- Delta vs first selected site: show +/- % difference in muted text

## Access Control
- User must have read access to all selected sites
- Sites from different orgs cannot be compared
- If user lacks access to one selected site: return that site's data as `null` with a `no_access` flag

## CLI Changes
```
spq compare --sites site_a,site_b [--range 30d]
```

## Edge Cases
- Comparing sites with very different traffic scales: normalize option (show %) vs absolute
- Max 5 sites per comparison (UI layout constraint)
- One site with no data in range: show zeros, not hidden
