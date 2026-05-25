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
