# Gambler Web UI

The `gambler` app should expose a web UI as the primary operator surface for observing odds selection, reviewing candidate coupons, and tracking Hermes learning.

## Product Goal

The UI should make the system inspectable. An operator should be able to see what the system observed, why it selected or rejected odds, which safety gates passed, and what Hermes is proposing.

The UI is not a marketing site. It is an operational dashboard for repeated review.

## Views

### Overview

- Browser observation health.
- Last successful Oddset and Tips snapshots.
- Scanner cadence, latest snapshot age, and whether the next worker scan is due.
- Sports intelligence ingestion health by sport and source, including motorsports series-family coverage for the broad motorsports category and a recovered count for older snapshots whose series can be inferred from stored feed text.
- Recent ingestion runs with source, status, covered sports, event count, and snapshot id.
- Provider coupon-rule observations, including observed accumulator bounds and restriction scope.
- Latest odds movements between repeated observations of the same event, market, and outcome.
- Candidate count by product, market, confidence, and status.
- Open simulated placements, unresolved settlements, and settled paper results.
- Recent paper plays, including both singles and multi-leg coupons with strategy, stake, observed odds, status, score, and confidence.
- Daily paper-placement rows should show expected result-check time, latest lookup time, lookup source/recommendation, latest settlement observation, and overdue age so unresolved rows can be reconciled without opening raw database state.
- Daily performance aggregate rows should show settlement-truth coverage counts next to settled counts so operators can see whether daily P/L is backed by recorded evidence.
- Daily performance aggregate rows should split out awaiting-result counts and exposure under open exposure so operators can distinguish active unsettled settlement work from other open states.
- Daily performance aggregate rows should mark incomplete days as provisional, show settlement progress and unresolved exposure ratio, and show worst/best-case P/L for remaining open paper positions.
- All-time strategy, sport, and risk-flag performance rows should also split out awaiting-result exposure so unresolved settlement backlog is visible outside the daily panels.
- Settlement workload metrics should show both counts and paper exposure for due review items, lookup-stale items, and sport-level due rows.
- Result-agent queue rows should show paper stake and priority score so the highest-impact stale result work is visible before an agent cycle runs.
- The latest result-agent cycle should be rendered next to the queue with queued, selected, attempted, skipped, and settled exposure/count summaries.
- Recent result-agent cycles should be rendered as a short history so operators can confirm the scheduled reconciliation loop is progressing and see whether cycles are settling or skipping backlog rows.
- Result-agent loop health should be visible as a top-level metric and a cycle-panel note, using the latest completed cycle age relative to the configured interval.
- Browser-backed external result evidence, including source, event, score, confidence, and whether the evidence has driven a paper settlement.
- Account-history coupon evidence shows preserved leg event names in the settlement observations and external evidence tables so coupon audit rows are readable without opening raw payload JSON.
- All known external result links on settlement-review rows, with browser-evidence markers for sources such as Sofascore that block direct HTTP lookups.
- Result-agent queue tasks for stale settlement rows, including source readiness, expected action, and whether public browser evidence or account-history evidence is needed.
- Native hover tooltips on the main action buttons and result-review panels so operators can see what each control does without expanding the dashboard.
- Operator-managed result links, including source, URL host, event aliases, and whether browser evidence is required.
- Recent audit events for scan, paper-placement, settlement, reflection, and strategy-review actions.
- Active local limits and whether real-money placement is disabled.
- Recent warnings, login expiry, maintenance windows, and blocked states.

### Odds Reasoning

For each candidate bet or coupon, show:

- Product: Oddset or Tips.
- Event, market, selection, available odds, and observed timestamp.
- Implied probability and estimated probability.
- Estimated edge and confidence.
- Evidence inputs, such as market movement, sport stats, team/player news, weather, seasonality, model features, and historical calibration.
- Odds movement should show previous odds, current odds, absolute move, percentage move, classification band, and whether the latest outcome is still active/displayed. Candidate rows should also show the movement known at candidate creation time when available.
- Missing or stale data warnings for stats, weather, news, rankings, and availability signals.
- Rejected alternatives and rejection reasons.
- Risk checks: duplicate exposure, stake limits, loss-cooldown, odds staleness, responsible-gambling flags, and terms/safety gate.
- Recommendation state: observed, candidate, rejected, needs review, approved for simulation, or promoted baseline.
- Simulated placement state: not placed, simulated placed, awaiting result, settled won, settled lost, void, pushed, refunded, cancelled, postponed, or unresolved.

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
- `market_observations`
- `outcome_observations`
- `tips_coupons`
- `candidate_bets`
- `candidate_coupons`
- `candidate_coupon_legs`
- `coupon_rule_observations`
- `simulated_bets`
- `simulated_coupons`
- `simulated_coupon_legs`
- `selection_reasoning_traces`
- `web_review_events`
- `audit_events`
- `settlement_observations`
- `external_result_evidence`
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
