---
type: decision
tags:
  - danske-spil/wiki
  - compliance
  - safety
updated: 2026-05-25
sources:
  - ../sources/danske-spil-netspil-terms.md
  - ../../docs/compliance-and-safety.md
---

# 0001 Research-First, Human-Approved Posture

## Status

Accepted for initial project scaffold.

## Decision

The project starts as observe-only and simulation-only. It may investigate the website, record sanitized state, model candidate bets, and run Hermes strategy experiments. It must not submit unattended real-money bets.

## Rationale

The official net-game terms reviewed during planning create direct risk around applications, robots, or similar mechanisms that affect or automate play. Regulated gambling also requires stronger local safety controls than a normal browser automation project.

## Consequences

- `DANSKESPIL_ALLOW_REAL_MONEY_PLACEMENT=false` remains the default.
- Stake limits default to zero.
- Hermes cannot control the browser or access credentials.
- Browser investigation stops before final submission.
- A new decision record is required before implementation may submit real-money bets.
