---
type: wiki-index
tags:
  - danske-spil/wiki
  - maintained-by-llm
updated: 2026-05-25
---

# Danske Spil Knowledge Wiki

Future Codex and Hermes sessions should read this file first for project-history, architecture, strategy, browser-investigation, and operations questions.

## Start Here

- [schema](schema.md) - Maintenance rules and page conventions.
- [log](log.md) - Append-only timeline of wiki operations.
- [concepts/llm-maintained-project-wiki](concepts/llm-maintained-project-wiki.md) - How this repo uses the LLM wiki pattern.
- [concepts/hermes-gambler-loop](concepts/hermes-gambler-loop.md) - Safe learning loop for `gambler` and Hermes.
- [concepts/gambler-web-ui](concepts/gambler-web-ui.md) - Operator dashboard for reasoning, candidate review, and Hermes state.
- [concepts/browser-automation-investigation](concepts/browser-automation-investigation.md) - How browser investigation should be run.

## Source Notes

- [sources/danske-spil-netspil-terms](sources/danske-spil-netspil-terms.md) - Source-note summary of official terms and responsible-gambling pages reviewed during planning.
- [sources/llm-wiki](sources/llm-wiki.md) - Source-note summary for the LLM wiki pattern.

## Runbooks

- [runbooks/README](runbooks/README.md) - Runbook landing page.
- [runbooks/browser-investigation](runbooks/browser-investigation.md) - Non-mutating Oddset/Tips browser investigation checklist.

## Decisions

- [decisions/README](decisions/README.md) - Decision-record landing page.
- [decisions/0001-research-first-human-approved](decisions/0001-research-first-human-approved.md) - Research-first, human-approved posture.

## Experiments

- [experiments/README](experiments/README.md) - Hermes and strategy experiment landing page.

## Open Questions

- Does Danske Spil provide any official odds/history API or export that should replace browser scraping?
- Which browser-session storage model is safest for Kubernetes without leaking cookies or credentials?
- Should `gambler` be Python-first for Playwright speed or Rust-first for deployment simplicity?
- What exact human approval UX is acceptable if real-money placement is ever considered?
