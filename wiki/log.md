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

- Created the initial project wiki structure under [wiki/](/Users/lindau/codex/danske-spil/wiki).
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
