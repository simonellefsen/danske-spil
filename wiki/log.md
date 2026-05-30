---
type: wiki-log
tags:
  - danske-spil/wiki
  - maintained-by-llm
updated: 2026-05-25
---

# Wiki Log

Append-only timeline for project wiki maintenance. Use headings with the format `## [YYYY-MM-DD] kind | summary`.

## [2026-05-25] scaffold | Initial planning wiki

- Created the initial project wiki structure under [wiki/](.).
- Added schema, index, source notes, concepts, runbooks, decisions, and experiment landing pages.
- Added project docs for planning, compliance, browser investigation, Hermes, Kubernetes, and wiki operation.
- Recorded the research-first and human-approved posture because current Danske Licens Spil terms create clear automation risk.

## [2026-05-25] planning | Gambler web UI requirement

- Added a dedicated web UI requirement for `gambler`.
- Defined operator views for odds reasoning, coupon review, Hermes reflections, experiment lifecycle, and audit events.
- Clarified that visible "thinking" means structured rationale and evidence, not hidden model scratchpads.
- Kept the documentation self-contained for this repository.

## [2026-05-25] planning | Simulation ledger requirement

- Added a dedicated simulation ledger requirement for `gambler`.
- Clarified that the system should scan and monitor markets, create immutable paper placements, and reconcile final outcomes.
- Added settlement lookup expectations, grading states, and simulated performance metrics.

## [2026-05-25] planning | Sports data intelligence

- Added the sports intelligence layer for stats, trends, weather, seasonality, news, and availability signals.
- Set the initial sport scope to football/soccer, tennis, basketball, Formula 1, golf, and cycling.
- Documented Postgres as the durable state store for normalized entities, source provenance, feature snapshots, and ingestion audit state.

## [2026-05-25] implementation | Agent-browser and content-service POC

- Used a dedicated `agent-browser` session for anonymous Oddset reconnaissance.
- Observed sport navigation, market chips, event links, odds buttons, and content-service JSON calls.
- Added POC scripts for browser artifact capture and read-only content-service normalization.
- Added the first DOM/content-service source note.

## [2026-05-25] implementation | Core service POC

- Added the first `gambler` API, web UI, scanner service, and paper-ledger flow.
- Added Postgres schema initialization for odds snapshots, candidate bets, simulated bets, audit events, and Hermes reflections.
- Added local Docker Desktop Kubernetes manifests for `gambler-api`, `gambler-worker`, `hermes-agent` POC, and a two-instance CNPG cluster.

## [2026-05-25] infrastructure | ngrok shared path routing

- Made the `gambler` UI base-path aware for `/danske-spil`.
- Documented the ngrok path-routing model for exposing the `danske-spil` UI behind Google SSO.
- Documented the shared ngrok hostname path-routing model.

## [2026-05-27] infrastructure | shared ngrok gateway ownership

- Moved shared ngrok routing ownership out of this app repository and into `/Users/lindau/codex/shared-ngrok-gateway`.
- Removed deploy-time patching of shared ngrok `AgentEndpoint` and `NgrokTrafficPolicy` resources.
- Kept this repository responsible for the `danske-spil` namespace, `gambler-api` service, and `/danske-spil` base-path behavior.

## [2026-05-27] implementation | combined strategy played summaries

- Updated strategy played summaries to count both single paper bets and multi-leg paper coupons.
- Updated sport performance aggregation so doubles, triples, and larger paper coupon positions contribute to turnover, exposure, P/L, and hit-rate metrics.
- Exposed separate single and coupon counts in the web UI strategy table.

## [2026-05-27] implementation | recent paper plays feed

- Added a web UI recent plays table backed by `/api/strategy/played`.
- Included paper singles and multi-leg coupons with strategy, stake, observed odds, status, score, and confidence.

## [2026-05-27] implementation | scan cadence visibility

- Added scanner cadence, latest snapshot age, and next scan due metadata to `/api/status`.
- Added dashboard metrics for the configured scan cadence and next scan due time.

## [2026-05-27] implementation | ingestion run visibility

- Added a web UI ingestion runs table backed by `/api/intelligence/coverage`.
- Exposed recent scanner run source, status, covered sports, event count, and snapshot id in the dashboard.

## [2026-05-27] implementation | audit event visibility

- Added `GET /api/audit/events` for recent immutable app audit events.
- Added a web UI audit events table for scan, paper-placement, settlement, reflection, and strategy-review actions.

## [2026-05-27] implementation | provider coupon rule visibility

- Added `coupon_rule_observations` for provider accumulator metadata observed during scans.
- Added `GET /api/coupon-rules` and a web UI table for accumulator bounds, sport scope, market context, and snapshot evidence.

## [2026-05-27] implementation | odds movement visibility

- Added `GET /api/odds/movement` to compare latest and previous observations for the same event, market, and outcome.
- Added a web UI odds movement table with previous odds, current odds, absolute move, percentage move, and latest active/displayed state.

## [2026-05-27] implementation | candidate movement evidence

- Embedded latest-prior odds movement into candidate feature snapshots and rationale at candidate insert time.
- Added candidate table movement hints so selected and rejected opportunities show whether odds drift was known when the candidate was created.

## [2026-05-27] implementation | movement risk classification

- Classified candidate odds movement as stable, normal, or large at insert time.
- Added movement-derived candidate risk flags while keeping numeric score changes gated behind reviewed strategy experiments.

## [2026-05-27] implementation | reviewed movement strategy gate

- Added `excluded_risk_flags` to active strategy baselines and replay evaluation.
- Added scan-derived Hermes proposals to exclude `large_odds_movement` when enough candidates show large odds drift.

## [2026-05-27] implementation | risk-flag performance view

- Added paper performance aggregation by candidate risk flag across singles and simulated coupons.
- Exposed the breakdown in the web UI so movement-risk gates can be compared against settled paper outcomes.

## [2026-05-27] implementation | stale settlement escalation

- Added a stale settlement recommendation for paper positions still unresulted more than 24 hours after the expected result-check time.
- Settlement review now recommends official competition results for stale rows instead of repeatedly treating the stale Danske Spil content-feed state as enough context.

## [2026-05-27] implementation | Flashscore settlement source

- Added Flashscore match pages as a settlement-capable fallback source for football, tennis, and basketball paper-result review.
- Stale external-result review rows now surface official competition results, Flashscore, and documented third-party sources as available manual evidence classes.

## [2026-05-27] implementation | batch and external settlement

- Settlement review selections are now staged in the UI and committed in a batch, with selected rows highlighted before submission.
- Added a conservative external auto-settlement pass for paper singles overdue by more than 2 hours when a configured source URL exposes a parseable final score.
- Tightened the overdue basis so the 2-hour external auto-check grace is relative to sport-specific expected finish, such as kickoff plus roughly 130 minutes for football.
- Tested Sofascore with plain and browser-like User-Agent headers. Direct HTTP still returned 403, while `agent-browser` loaded the page, so Sofascore is flagged as requiring browser automation evidence.
- Added `external_result_evidence` and API routes for browser-backed result evidence. Submitted evidence can settle matching open single-leg winner markets only when the selected outcome maps deterministically to the supplied final score.
- Added `scripts/sofascore_evidence_probe.py`, a local `agent-browser` probe that extracts sanitized Sofascore result evidence and submits it to the API with `settle=false` by default.
- Surfaced browser-backed external result evidence in the web UI next to settlement observations and lookup attempts.
- Added a generalized browser evidence probe for Sofascore, Flashscore, and LiveScore public match URLs, and surfaced known external result links directly on settlement-review rows.
- Settlement-review rows now carry all configured external result links for a match, and the direct external auto-check tries each non-browser source before requiring browser evidence.
- Added persistent operator-managed external result links, host validation, a `POST /api/settlement/source-link` endpoint, and settlement-review UI controls for attaching public result URLs.
- Added `GET /api/settlement/source-links` and an operator result-links UI table so persisted result URLs are directly auditable.

## [2026-05-28] implementation | result agent queue

- Added `GET /api/result-agent/queue`, which turns due settlement-review rows into read-only result-agent tasks with expected finish timing, source links, search terms, source precedence, and sanitized account-agent availability flags.
- Added a result-agent queue table to the web UI and removed the normal settlement-review prompt flow for manually pasting public result URLs.
- Added `scripts/result_agent.py`, a local agent runner that consumes the queue and automates browser-backed public result evidence collection for configured match links.
- Documented the read-only Danske Spil account-history result-agent boundary: use an operator browser session, prefer account/coupon history when available, and post only sanitized settlement facts.
- Added hover tooltips to the main dashboard actions and settlement/result-agent panels so the paper-settlement workflow is discoverable in the UI.

## [2026-05-28] implementation | scheduled Rust result agent

- Added a Rust-native Flashscore result-agent cycle to the worker so missing public result links can be discovered inside the scratch container without the Python runner.
- Added `POST /api/result-agent/run` and a web UI action to trigger the same read-only discovery pass manually.
- The worker now attempts result-agent discovery on the normal 15-minute scan cadence, stores discovered Flashscore links, records aliases with sport/gender scope when known, and posts sanitized paper-settlement evidence for finished events.
- Expanded Flashscore participant matching with alias variants, Danish-to-English country names, gender-aware ranking, and a stable `x-fsign` fallback for current Flashscore pages that no longer expose `feed_sign` in page HTML.
- Split result-agent runtime responsibility into a dedicated `gambler-result-agent` Kubernetes deployment and ClusterIP service. The worker now refreshes settlement review state while the result-agent service owns scheduled paper-only result reconciliation.
- Split the result-agent build path into a separate `danske-spil-result-agent` binary and scratch image built with `--no-default-features`, avoiding Dioxus compilation for result-agent-only image builds.
- Routed web/API result-agent queue and run endpoints through the dedicated `gambler-result-agent` ClusterIP service via `GAMBLER_RESULT_AGENT_URL`, keeping local execution as a development fallback.

## [2026-05-28] implementation | Hermes loop service

- Converted the `hermes-agent` Kubernetes deployment from a passive API view into a loop participant by running `/gambler hermes-agent`.
- Added a scheduled Hermes-safe cycle that refreshes the paper-only daily reflection, summarizes active strategy/proposal state, and records a `hermes_cycle_completed` audit event.
- Added `POST /api/hermes/run` and a web UI button for manually triggering one Hermes-safe cycle without browser control, credential access, or real-money placement.
- Added Hermes promotion gates for active experiments, including replay evidence, one-variable status, minimum settled sample size, unresolved exposure, and paper-only safety blockers.
- The web UI now renders promotion gates and disables experiment promotion until the Hermes gate marks the experiment eligible.
- The strategy review API now rejects promotion attempts when the Hermes promotion gate has not cleared.
- Hermes cycles now refresh replay evidence for open strategy experiments before recomputing promotion gates; the refresh is paper-only and does not change experiment status or place bets.
- `GET /api/hermes` and the web UI now expose the latest Hermes cycle audit summary, including reflection id, replay refresh counts, trigger, and safety posture.

## [2026-05-29] implementation | account-history request contract

- Added `GET /api/result-agent/account-requests` as a focused, sanitized worklist for a future local read-only Danske Spil account-history browser agent.
- Added a web UI account-history requests table showing the paper row, expected bookmaker truth, and allowed evidence contract.
- Documented that the account-history path must not store credentials, cookies, browser storage, payment data, Spil-ID/MitID payloads, or full account pages.

## [2026-05-29] implementation | account-history settlement evidence

- Added status-only account-history evidence ingestion for bookmaker won, lost, void, pushed, refund, cancellation, abandonment, postponement, and unresolved states.
- The API can now persist `mode=account_history_settlement_evidence` rows without final scores and can reconcile a paper bet or coupon when `settle=true`.
- The web UI external-evidence table now displays the bookmaker settlement state instead of a placeholder score for status-only account-history evidence.

## [2026-05-29] implementation | local account-history agent

- Added `scripts/account_history_agent.py`, a local `agent-browser` worker that consumes `GET /api/result-agent/account-requests`, matches visible account-history text to queued paper rows, and posts compact status-only evidence.
- Added a `make account-history-agent-dry-run` helper and documented the dry-run-first workflow before allowing `--settle`.
- Added `DANSKESPIL_ACCOUNT_HISTORY_URL` to the local env template so the operator-controlled account/history page can be configured without storing browser state.

## [2026-05-29] implementation | account-history runbook in UI

- Added local account-history runbook metadata to `GET /api/result-agent/account-requests`, including the port-forward command, dry-run command, script name, and local history URL environment key.
- Surfaced that runbook in the web UI next to account-history requests so operators can run the local agent without guessing the next safe command.

## [2026-05-29] implementation | account-history parser tests

- Added offline fixture modes to `scripts/account_history_agent.py` so parser development can use sanitized text or extracted JSON without opening a browser session.
- Added request-queue fixture support so the account-history agent can dry-run the full matching path without Kubernetes or browser access.
- Added unit tests for Danish-name normalization, ambiguous status rejection, account-history URL query stripping, and text-fixture line extraction.
- Added checked-in sanitized account-history fixtures and `make account-history-agent-fixture-dry-run` for a no-browser, no-cluster parser smoke test.
- Deferred non-terminal account-history states by default and added `--include-nonterminal` for diagnostic dry runs that intentionally emit unresolved/postponed payloads.
- Preserved coupon leg event names in local account-history evidence payloads and synthesized a coupon-level event label when a request has no single event name.
- Required all coupon legs to be visible in account-history context before emitting bookmaker evidence for a coupon.
- Preserved account-history `event_names` through the API evidence template, stored evidence payload, and settlement notes so coupon-level audit rows keep their leg context.
- Displayed preserved account-history coupon leg names in the settlement observations and external evidence tables.
- Skipped local account-history evidence when multiple visible contexts for the same queued event contain conflicting deterministic bookmaker statuses.
- Added `make account-history-agent-test` for the local parser test suite.

## [2026-05-29] implementation | Ledger reporting context

- Added joined candidate context to `/api/ledger` rows (`sport_key`, event, competition, market, and outcome) so daily reporting and the web UI do not need to recover paper placement labels from nested payload JSON.
- Added `/api/performance/today` for Europe/Copenhagen local-day paper performance, including singles, coupons, by-sport aggregates, recent placements, and settlement observation counts.
- Added a web UI `Today` panel backed by `/api/performance/today`.
- Added Makefile wrappers for local scratch-image builds, Kubernetes deployment, and namespace status checks.
- Generalized daily paper performance to `/api/performance/yesterday` and `/api/performance/day?date=YYYY-MM-DD`, and added a web UI `Yesterday` panel.
- Added a web UI daily-performance date picker backed by `/api/performance/day`.
- Added selected-day recent paper placements to the daily-performance lookup panel.
- Rendered unsettled daily-placement P/L as `-` instead of `0.00` so open positions are not confused with void/refund zero-P/L outcomes.
- Added expected result-check, latest lookup, and overdue-age fields to selected-day placement rows so stale paper positions are easier to reconcile.
- Added latest lookup source and recommendation to selected-day placement rows to explain what settlement path was last attempted.
- Added latest settlement observation result, source, confidence, and timestamp to selected-day placement rows so the daily report shows the recorded truth next to ledger status.
- Added settlement-truth coverage counts to daily aggregate rows so the selected-day report shows how many placements have recorded settlement observations.
- Added awaiting-result counts and exposure to daily aggregate rows so open exposure shows the settlement backlog separately.

## [2026-05-25] implementation | Candidate ranking and paper settlement POC

- Added `poc_ranker_v1` candidate scoring fields: implied probability, model probability, expected value, confidence, score, risk flags, and feature snapshot.
- Extended the simulation ledger with settlement metadata, simulated return, profit/loss, and settlement observations.
- Added web UI metrics for open exposure and paper P/L plus manual paper-settlement controls.

## [2026-05-25] implementation | Rust and Dioxus runtime migration

- Reimplemented the active `gambler` POC as a Rust service with Axum, Dioxus SSR, Postgres state, and the read-only Danske Spil scanner.
- Switched deployment to a single Rust binary that can run the API/Hermes web view or the scheduled worker loop.
- Replaced the Python runtime container with a multi-stage Docker build and `scratch` final image.

## [2026-05-25] documentation | GitHub-ready Start Here links

- Replaced absolute local README links with repository-relative links that work on GitHub.
- Expanded the README Start Here section with short topic descriptions.
- Updated wiki link conventions to avoid local-machine paths in Markdown links.
