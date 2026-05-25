---
type: concept
tags:
  - danske-spil/wiki
  - web-ui
  - operator-review
updated: 2026-05-25
sources:
  - /Users/lindau/codex/danske-spil/docs/web-ui.md
---

# Gambler Web UI

The `gambler` web UI is the operator-facing surface for understanding what the system observes, why it selects or rejects odds, and what Hermes is proposing.

## Required Views

- Overview: observation health, recent snapshots, active limits, and warnings.
- Odds Reasoning: candidates, selected odds, scoring inputs, confidence, rejected alternatives, and safety gates.
- Coupons: proposed legs, combined odds, exposure, uncertainty, simulation-ledger status, and disabled submission state.
- Hermes: reflections, one-variable experiment proposals, lifecycle state, evidence, and active baseline context.
- Audit: immutable observation, reasoning, safety, review, and experiment events.

## Reasoning Boundary

The UI should show structured rationale and evidence. It must not show hidden chain-of-thought, raw model scratchpads, credentials, cookies, browser profiles, or raw account payloads.

## Related

- [Hermes gambler loop](hermes-gambler-loop.md)
- [browser automation investigation](browser-automation-investigation.md)
