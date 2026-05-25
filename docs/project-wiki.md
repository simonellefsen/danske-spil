# Project Knowledge Wiki

This repository uses an LLM-maintained wiki pattern. The wiki is the maintained synthesis layer between raw source files, browser observations, strategy experiments, and future agent sessions.

## Directory Structure

```text
wiki/
  index.md
  log.md
  schema.md
  concepts/
  sources/
  runbooks/
  decisions/
  experiments/
```

## What Belongs In The Wiki

- Browser investigation findings.
- Stable DOM/navigation models.
- Web UI decisions and operator-review flows.
- Simulation ledger, settlement, and performance methodology.
- Sports intelligence ingestion, feature snapshots, and source policies.
- Strategy hypotheses and rejected ideas.
- Hermes reflections and one-variable experiment summaries.
- Kubernetes and operations runbooks.
- Compliance and safety decisions.

## What Does Not Belong In The Wiki

- Credentials, cookies, session exports, MitID data, Spil-ID data, payment data, or raw account payloads.
- Real-money betting instructions that bypass human approval.
- Unreviewed advice to violate Danske Spil terms or responsible-gambling controls.

## qmd Setup

Run from the repository root when local markdown search is needed:

```bash
rtk qmd init
rtk qmd collection add /Users/lindau/codex/danske-spil/wiki --name danske-spil-wiki
rtk qmd collection add /Users/lindau/codex/danske-spil/docs --name danske-spil-docs
rtk qmd update
rtk qmd embed -c danske-spil-wiki
```

Search examples:

```bash
rtk qmd search "Oddset selector betslip" -c danske-spil-wiki -n 10
rtk qmd query "what is the Hermes experiment gate" -c danske-spil-wiki -n 10
```

## Maintenance Workflow

When a new source matters:

1. Create or update a page under `wiki/sources/`.
2. Update affected concept, runbook, decision, or experiment pages.
3. Update `wiki/index.md`.
4. Append one entry to `wiki/log.md`.

For durable conclusions from chat or browser investigation, file the conclusion in the wiki before ending the work session.
