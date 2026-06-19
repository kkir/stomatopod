# A/B Test Tracking

## Problem
No built-in way to compare conversion rates between variants of a feature or page. Teams run experiments but analytics can't tell them which variant won.

## Goal
Track experiment variants via custom event properties and compare conversion rates per variant. No server-side assignment — Stomatopod is the measurement layer, not the assignment layer.

## Approach
Teams emit variant assignment as an event property on custom events or as a dedicated experiment event. Stomatopod queries group by variant value.

## Tracking Pattern
```js
// Option A: tag any event with a variant property
stomatopod.track('button_click', { experiment: 'checkout_cta', variant: 'A' });

// Option B: dedicated experiment event on assignment
stomatopod.track('experiment_viewed', { experiment: 'checkout_cta', variant: 'B' });
```

No schema changes — properties are already JSON.

## API Changes

### Experiment list (auto-detected from events)
```
GET /api/v1/sites/{site}/experiments?range=30d
```
Returns distinct `experiment` property values found in events:
```json
{
  "experiments": [
    { "name": "checkout_cta", "variants": ["A", "B"], "first_seen": "2025-06-01" }
  ]
}
```

### Experiment results
```
GET /api/v1/sites/{site}/experiments/{experiment_name}?range=30d&goal=signup
```
Response:
```json
{
  "experiment": "checkout_cta",
  "range": "30d",
  "variants": [
    {
      "variant": "A",
      "exposures": 1240,    -- sessions that saw this variant
      "conversions": 62,    -- sessions that completed the goal
      "conversion_rate": 5.0,
      "pct_of_traffic": 49.8
    },
    {
      "variant": "B",
      "exposures": 1251,
      "conversions": 88,
      "conversion_rate": 7.0,
      "pct_of_traffic": 50.2
    }
  ],
  "winner": "B",
  "confidence": 94.2   -- statistical significance %
}
```

Statistical significance: two-proportion z-test. Display as confidence level. Note: Stomatopod provides the calculation but teams should verify with a statistician for high-stakes decisions.

## Goal Linking
Conversion is measured via Goals (see goals.md). If no goal specified, conversion = any event in the session after the experiment event.

## UI
- "Experiments" tab in site dashboard (appears automatically when experiment events detected)
- List: experiments with active variants + participant counts
- Detail: variant comparison table with conversion rates + winner badge
- Confidence level bar (red < 80%, yellow 80–95%, green > 95%)
- Date range selector

## CLI Changes
```
spq query experiments --site <id> [--range]
spq query experiment --site <id> --experiment checkout_cta --goal signup [--range]
```

## Edge Cases
- No goal specified: show exposure counts only (no conversion rate)
- Single variant: show data but note "no comparison variant found"
- Very low traffic (< 100 exposures per variant): flag as "insufficient data"
- Experiment property name is convention-based — no enforcement. If teams use `test`, `exp`, `variant` inconsistently, data fragments across names.
