# Gambler Web UI

The `gambler` app should expose a web UI as the primary operator surface for observing odds selection, reviewing candidate coupons, and tracking Hermes learning.

## Product Goal

The UI should make the system inspectable. An operator should be able to see what the system observed, why it selected or rejected odds, which safety gates passed, and what Hermes is proposing.

The UI is not a marketing site. It is an operational dashboard for repeated review.

## Views

### Overview

- Browser observation health.
- Last successful Oddset and Tips snapshots.
- Sports intelligence ingestion health by sport and source.
- Candidate count by product, market, confidence, and status.
- Open simulated placements, unresolved settlements, and settled paper results.
- Active local limits and whether real-money placement is disabled.
- Recent warnings, login expiry, maintenance windows, and blocked states.

### Odds Reasoning

For each candidate bet or coupon, show:

- Product: Oddset or Tips.
- Event, market, selection, available odds, and observed timestamp.
- Implied probability and estimated probability.
- Estimated edge and confidence.
- Evidence inputs, such as market movement, sport stats, team/player news, weather, seasonality, model features, and historical calibration.
- Missing or stale data warnings for stats, weather, news, rankings, and availability signals.
- Rejected alternatives and rejection reasons.
- Risk checks: duplicate exposure, stake limits, loss-cooldown, odds staleness, responsible-gambling flags, and terms/safety gate.
- Recommendation state: observed, candidate, rejected, needs review, approved for simulation, or promoted baseline.
- Simulated placement state: not placed, simulated placed, awaiting result, settled won, settled lost, void, or unresolved.

The reasoning panel should show structured rationale and evidence. It should not show hidden chain-of-thought, raw model scratchpads, credentials, cookies, browser profile data, or raw account payloads.

### Coupon Builder

The coupon builder should be read-only or simulation-only by default:

- Support single, double, triple, and larger accumulator views when the provider allows those combinations.
- Show proposed legs and combined odds.
- Show each leg's rationale and uncertainty.
- Show provider rule evidence for why the selected legs can be combined, including any same-sport or same-category restriction.
- Show exposure if the coupon were approved.
- Show whether the coupon would be written to the simulation ledger.
- Show why any leg was removed or replaced.
- Keep submission actions disabled while `DANSKESPIL_ALLOW_REAL_MONEY_PLACEMENT=false`.

### Hermes

The Hermes view should show:

- Latest reflection summary.
- Recent reflections with timestamps and evidence references.
- One-variable experiment proposals.
- Experiment status: pending review, rejected, approved for replay, active simulation, failed, promoted, or rolled back.
- Changed variable, baseline value, proposed value, expected effect, and measured result.
- Approval history and operator notes.
- Active baseline context when one exists.

### Audit

The audit view should show immutable events:

- Browser observations.
- Sports data ingestion runs.
- Feature snapshot creation.
- Candidate creation.
- Simulated placement creation.
- Settlement lookup and grading.
- Reasoning-trace writes.
- Safety gate failures.
- Human review events.
- Hermes proposal lifecycle transitions.
- Configuration changes that affect risk limits.

## Data Contract

The UI should read from normalized application state rather than browser internals.

Candidate tables:

- `sports`
- `competitions`
- `teams`
- `players`
- `sport_events`
- `feature_snapshots`
- `source_registry`
- `odds_snapshots`
- `tips_coupons`
- `candidate_bets`
- `candidate_coupons`
- `simulated_bets`
- `simulated_coupons`
- `simulated_coupon_legs`
- `selection_reasoning_traces`
- `web_review_events`
- `settlement_observations`
- `settlement_sources`
- `hermes_reflections`
- `strategy_experiments`
- `strategy_baselines`

## Safety Requirements

- Never render secrets, cookies, MitID data, Spil-ID identifiers, payment data, raw account payloads, or browser profiles.
- Never show raw hidden model chain-of-thought.
- Redact or omit personal account details.
- Mark stale odds clearly.
- Label all paper-ledger results as simulated.
- Keep unresolved or ambiguous outcomes out of performance promotion metrics until reviewed.
- Show disabled state and reason for any betting-critical action.
- Default every mutation to review-only until the compliance decision changes.

## UX Principles

- Dense, scannable, work-focused layout.
- Tabs for Overview, Odds Reasoning, Coupons, Hermes, and Audit.
- Tables for candidates and experiments.
- Detail panels for rationale and evidence.
- Clear status badges for safety gates and lifecycle state.
- No nested card layouts or decorative hero sections.
