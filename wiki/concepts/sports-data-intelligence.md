---
type: concept
tags:
  - danske-spil/wiki
  - sports-data
  - postgres
  - features
updated: 2026-05-25
sources:
  - ../../docs/data-intelligence.md
---

# Sports Data Intelligence

`gambler` should use Postgres to maintain durable sports intelligence for decision support and simulation replay.

## Initial Sport Scope

- Football/soccer.
- Tennis.
- Basketball.
- Motorsports, including Formula 1, IndyCar, NASCAR, endurance racing such as Le Mans, and motorbike racing.
- Golf.
- Cycling.

## Stored Context

- Teams, players, drivers, golfers, riders, competitions, seasons, venues, events, and participants.
- Stats, rankings, standings, injuries, availability, weather, news, trends, and seasonality.
- Source provenance, ingestion runs, reliability notes, and feature snapshots.

## Core Rule

Feature snapshots are immutable and decision-time scoped. A simulated placement should be explainable from the stats, news, weather, seasonality, and trend data that existed at the simulated placement timestamp.

Motorsports is intentionally broad. Feature snapshots classify racing rows into
series families such as Formula 1, IndyCar, NASCAR, endurance, motorbike,
rally, or unknown. Unknown series should be treated as a source-adapter gap,
not as Formula 1 by default.

## Related

- [simulation ledger](simulation-ledger.md)
- [gambler web UI](gambler-web-ui.md)
