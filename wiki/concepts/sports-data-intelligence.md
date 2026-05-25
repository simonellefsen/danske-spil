---
type: concept
tags:
  - danske-spil/wiki
  - sports-data
  - postgres
  - features
updated: 2026-05-25
sources:
  - /Users/lindau/codex/danske-spil/docs/data-intelligence.md
---

# Sports Data Intelligence

`gambler` should use Postgres to maintain durable sports intelligence for decision support and simulation replay.

## Initial Sport Scope

- Football/soccer.
- Tennis.
- Basketball.
- Formula 1.
- Golf.
- Cycling.

## Stored Context

- Teams, players, drivers, golfers, riders, competitions, seasons, venues, events, and participants.
- Stats, rankings, standings, injuries, availability, weather, news, trends, and seasonality.
- Source provenance, ingestion runs, reliability notes, and feature snapshots.

## Core Rule

Feature snapshots are immutable and decision-time scoped. A simulated placement should be explainable from the stats, news, weather, seasonality, and trend data that existed at the simulated placement timestamp.

## Related

- [simulation ledger](simulation-ledger.md)
- [gambler web UI](gambler-web-ui.md)
