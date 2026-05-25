# Project Plan

This plan starts from an empty local repository and aims for a guarded, observable system before any betting-critical behavior exists.

## Constraints Found During Planning

Danske Licens Spil's current net-game terms for the Blue Account state that the company can limit deposits, block accounts, cancel games, and reverse funds for several suspected behaviors. The listed behaviors include applications, robots, or similar mechanisms used to affect or automate play.

That means this project should begin as a research and decision-support system. Any move toward real-money placement needs an explicit human decision after reviewing account, legal, responsible-gambling, and terms-of-service risk.

## Target Shape

```mermaid
flowchart TB
  subgraph K["Docker Desktop Kubernetes"]
    subgraph NS["namespace: danske-spil"]
      G["gambler agent\nread-only browser + strategy service"]
      UI["gambler web UI\nreasoning + review dashboard"]
      GMCP["gambler-mcp\nHermes-safe tools"]
      H["hermes-agent\nreflection + experiment proposals"]
      PG[("CloudNativePG\n2 instances")]
      W["wiki/docs\nmounted read-only or synced"]
    end
  end

  DS["danskespil.dk\nOddset + Tips"] --> G
  G --> PG
  UI --> G
  UI --> PG
  GMCP --> PG
  H --> GMCP
  H --> W
```

## Phase 0: Planning And Knowledge Base

Deliverables:

- Initial docs and wiki scaffold.
- Compliance and safety guardrails.
- Browser investigation runbook.
- Kubernetes architecture plan for namespace `danske-spil`.
- Hermes/gambler goal contract and experiment rules.
- Web UI requirements for reasoning, candidate review, and Hermes lifecycle visibility.
- Simulation ledger requirements for paper bet placement, exposure, and outcome reconciliation.
- Sports data intelligence requirements for stats, trends, weather, seasonality, and news.

Exit criteria:

- The repo has enough durable context that a later implementation session can start without rediscovering the operating model.

## Phase 1: Non-Mutating Browser Investigation

Scope:

- Use `agent-browser` or a Playwright-backed equivalent with a real browser engine and persistent session.
- Learn the Oddset and Tips navigation model.
- Record sanitized selectors, page states, URL transitions, and network/API observations.
- Identify login flow boundaries, MitID/Spil-ID checkpoints, cookie behavior, and bot/automation blocks.
- Stop before final bet submission, deposit, withdrawal, payment, account change, or bonus activation.

Outputs:

- `wiki/sources/danske-spil-dom-observations.md`
- `wiki/concepts/oddset-navigation-model.md`
- `wiki/concepts/tips-navigation-model.md`
- Sanitized screenshots under a future ignored or reviewed artifact directory.

## Phase 2: Read-Only Data Model

Scope:

- Persist odds snapshots, Tips coupons, event metadata, candidate selections, browser-session metadata, and audit events.
- Persist simulated bet placements and coupon placements as immutable paper-ledger records.
- Persist settlement lookups and final win/loss grading for each simulated bet.
- Persist sports intelligence inputs for the initial focus sports: football/soccer, tennis, basketball, Formula 1, golf, and cycling.
- Add deduplication and timestamps so strategy learning can replay observations.
- Store no credentials, cookies, MitID data, or raw personally identifying account payloads in Postgres.
- Store structured decision traces that explain candidate selection without storing private model scratchpads or secrets.

Candidate tables:

- `odds_snapshots`
- `tips_coupons`
- `candidate_bets`
- `candidate_coupons`
- `simulated_bets`
- `simulated_coupons`
- `simulated_coupon_legs`
- `settlement_observations`
- `settlement_sources`
- `sports`
- `leagues`
- `seasons`
- `teams`
- `players`
- `sport_events`
- `participant_stats`
- `team_stats`
- `weather_observations`
- `news_items`
- `trend_signals`
- `seasonality_profiles`
- `injury_reports`
- `rankings_snapshots`
- `dom_observations`
- `browser_session_audit`
- `selection_reasoning_traces`
- `web_review_events`
- `hermes_reflections`
- `strategy_experiments`
- `risk_limits`

## Phase 3: Gambler Agent

Scope:

- Implement `gambler` as a service that can collect read-only market state, build candidate coupons, score them, and expose sanitized context.
- Implement scanner loops for Oddset and Tips that monitor configured products, events, markets, coupons, odds changes, and disabled/suspended states.
- Implement data-ingestion loops for relevant leagues, teams, players, drivers, golfers, riders, and events in the initial sports scope.
- Normalize stats, weather, news, trend, and seasonality inputs into Postgres with source provenance and timestamps.
- Implement a simulation ledger that records when the system would have taken a bet, at what observed odds, with what hypothetical stake, and under which strategy baseline.
- Implement a settlement worker that later looks up final outcomes, grades simulated bets and coupon legs, and records simulated P/L.
- Expose a web UI for operator visibility into candidates, odds, reasoning, safety gates, and Hermes state.
- Expose a narrow `gambler-mcp` adapter for Hermes.
- Keep browser cookies and credentials isolated from Hermes.
- Add dry-run coupon preparation only if it can stop reliably before final submission.

Default mode:

```yaml
gambler:
  mode: observe_only
  allow_real_money_placement: false
  require_human_approval: true
```

## Phase 4: Web UI

Scope:

- Build the first `gambler` dashboard as the primary operator surface.
- Show current observation status, candidate bets/coupons, selected odds, expected value inputs, model confidence, responsible-gambling checks, and rejected alternatives.
- Show structured rationale: evidence, scoring factors, uncertainty, and final recommendation. Do not display hidden chain-of-thought or raw prompt scratchpads.
- Include a Hermes view with recent reflections, one-variable experiment proposals, lifecycle state, evidence, approval history, and active baseline context.
- Keep all action buttons disabled or review-only while `DANSKESPIL_ALLOW_REAL_MONEY_PLACEMENT=false`.

## Phase 5: Hermes Agent

Scope:

- Deploy Hermes in the same namespace with its own PVC.
- Allow Hermes to read sanitized context through MCP.
- Allow Hermes to write reflections and one-variable experiment proposals.
- Forbid Hermes from browser control, credential access, Kubernetes secret reads, or bet placement.

## Phase 6: Simulation And Reinforcement Learning

Scope:

- Build a simulation loop from odds snapshots, candidate bets, simulated placements, and settlement observations.
- Join candidate decisions to the relevant historical stats, form, news, weather, rankings, and seasonality features available at decision time.
- Treat simulated placements as immutable: later odds changes should create observations, not rewrite the simulated entry price.
- Reconcile event outcomes from the most authoritative available source, preferably Danske Spil settlement/result pages when available, and fall back only to documented external result sources.
- Compute simulated stake, return, profit/loss, hit rate, calibration, drawdown, and coupon-level performance.
- Let Hermes optimize prompts, thresholds, staking heuristics, or market filters one variable at a time.
- Require backtest/replay evidence and paper observation before any human can promote a baseline.

## Phase 7: Human-Approved Action Surface

This phase is intentionally gated. It should not start until the compliance document is reviewed.

Possible scope after review:

- A UI or CLI that displays a proposed coupon, stake, evidence, and risk impact.
- The human manually confirms or rejects.
- The system records immutable audit events.
- Real-money placement remains disabled by default and must require a deliberate local config change.

## Backlog

- Confirm whether Danske Spil offers any official API, affiliate feed, or export surface for odds/history.
- Identify licensed or otherwise permissible data sources for each sport and record source reliability in Postgres.
- Decide whether `gambler` runtime should be Python-first for browser automation or Rust-first for deployment simplicity.
- Choose the web UI stack after the first data model is stable.
- Design the first Postgres schema migration.
- Create Kubernetes manifests and deploy script after the first non-mutating implementation exists.
- Add qmd collections once the wiki grows beyond the initial scaffold.
