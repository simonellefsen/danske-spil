# Sports Data Intelligence

`gambler` should enrich betting candidates with durable sports intelligence stored in Postgres. The goal is to make simulated decisions explainable and replayable from the information available at decision time.

Initial sports scope:

- Football/soccer.
- Tennis.
- Basketball.
- Motorsports, including Formula 1, IndyCar, NASCAR, endurance racing such as Le Mans, and motorbike racing.
- Golf.
- Cycling.

## Responsibilities

`gambler` should ingest and normalize:

- Historical and recent team/player performance.
- Form trends and market movement.
- League, tournament, event, circuit, course, and route context.
- Weather and venue conditions where they matter.
- Seasonal cycles, schedule density, travel, rest, and calendar effects.
- News, injury, suspension, lineup, roster, equipment, and availability signals.
- Rankings, standings, seeds, draws, and qualification context.
- Final outcomes for settlement and model calibration.

Every ingested record should carry source provenance, observed timestamp, source timestamp when available, confidence, and a normalized entity mapping.

## Sports Coverage Model

Coverage should be configurable. "All relevant leagues" means the configured coverage universe for the current strategy, not an unbounded crawler.

Recommended entities:

- Sport.
- League, tournament, series, tour, or competition.
- Season or campaign.
- Venue, circuit, course, route, or stage.
- Team, constructor, national team, or club.
- Player, driver, golfer, rider, or coach where relevant.
- Event or match.
- Market and selection mapping.

Sport-specific notes:

- Football/soccer: teams, players, injuries, suspensions, lineups, rest days, travel, venue, weather, table position, expected goals, scoring/conceding trends, set-piece trends.
- Tennis: player rankings, surface, draw, fatigue, head-to-head, recent form, injury/retirement signals, travel, indoor/outdoor and weather where relevant.
- Basketball: teams, players, injuries, rotations, pace, offensive/defensive ratings, schedule density, back-to-backs, travel, home/away splits.
- Motorsports: series, drivers/riders, teams/constructors, circuits, practice/qualifying/race sessions, weather, tire degradation, grid position, penalties, safety-car history, and series-specific race format.
- Golf: players, course fit, weather/wind, strokes-gained categories, recent form, tournament history, tee-time wave effects.
- Cycling: riders, teams, race profile, stage type, route, weather/wind, climbing/time-trial ability, fatigue, injuries, team tactics, general-classification context.

## Postgres State

Postgres should be the durable state store for raw normalized inputs, derived features, and audit metadata.

Core tables:

- `sports`
- `competitions`
- `seasons`
- `venues`
- `teams`
- `players`
- `participants`
- `sport_events`
- `event_participants`
- `team_stats`
- `participant_stats`
- `rankings_snapshots`
- `standings_snapshots`
- `injury_reports`
- `availability_reports`
- `weather_observations`
- `news_items`
- `trend_signals`
- `seasonality_profiles`
- `feature_snapshots`
- `source_registry`
- `ingestion_runs`
- `coupon_rule_observations`

Feature snapshots should be immutable and tied to the decision timestamp. If data changes later, create a new snapshot rather than rewriting the old one.

Current POC status:

- `source_registry` records the read-only Danske Spil content-service as a market snapshot source.
- `source_registry` also seeds settlement-capable source classes for Danske Spil account/coupon history, official competition results, Flashscore, Sofascore, LiveScore match pages, and documented third-party fallbacks. Sofascore is flagged as requiring browser automation because direct HTTP tests returned 403 even with browser-like request headers, while `agent-browser` could access the same match page.
- `external_result_links` stores operator-added public match-result URLs separately from seeded source policy. Settlement-review reads merge these links into the source policy so additions survive schema bootstrap refreshes.
- `external_result_evidence` stores sanitized browser-backed result evidence for sources that require a real browser session. It records source URL, event name, participant names, final score, confidence, and a short text excerpt, not cookies or browser storage.
- `ingestion_runs` records scanner runs, the snapshot id, covered sports, event count, and completion status.
- The web UI surfaces recent ingestion runs so scanner completion history can be reviewed without querying Postgres directly.
- `feature_snapshots` stores one `market_context_v1` row per observed event per snapshot.
- The first feature set is intentionally limited to market-feed context: competition, start time, participant count, market count, outcome count, market kinds, external providers, live/result flags, sport-specific context, and missing-signal markers.
- Motorsports feature snapshots include `sport_context` with a broad `series_family` classifier for Formula 1, IndyCar, NASCAR, endurance racing, motorbikes, rally, or `unknown`. Unknown series add a `motorsports_series` missing-signal marker so later source adapters can target the gap.
- `coupon_rule_observations` stores observed provider accumulator bounds for markets that expose `minimum_accumulator` or `maximum_accumulator`.
- `GET /api/odds/movement` derives latest-vs-previous odds drift from `outcome_observations` for decision-time monitoring.
- Candidate feature snapshots embed `odds_movement` when the same event, market, and outcome had a prior observation before the current scan, and movement-derived risk flags are persisted for replay.
- Weather, news, rankings, form, and injury/availability are explicitly marked missing until separate sources are configured and reviewed.
- Coverage is exposed through `GET /api/intelligence/coverage` and shown in the web UI. The same endpoint includes `motorsports_series`, a series-family coverage summary for the broad motorsports category. The read model derives an effective series from stored competition, class, and event names when older snapshots still have `unknown` series context, and reports recovered versus genuinely missing rows.

## Decision-Time Features

Candidate scoring should use only data known at or before the simulated placement timestamp.

Examples:

- Recent form window.
- Season-to-date baseline.
- Head-to-head record.
- Weather impact.
- Venue or surface fit.
- Injury/availability state.
- Schedule density and travel.
- News sentiment or event flags.
- Market movement and odds drift.
- Model calibration bucket.

## Source Policy

For each source, record:

- Source name.
- URL or provider identifier.
- License/terms notes when known.
- Sport coverage.
- Refresh cadence.
- Reliability score.
- Whether it can be used for settlement.
- Whether manual review is required.

Do not ingest private account data, credentials, cookies, or browser session material into sports intelligence tables.

## Hermes Context

Hermes should receive summarized, sanitized feature context:

- Feature values used for candidate scoring.
- Missing or stale data warnings.
- Source reliability notes.
- Evidence snapshots for simulated placements.
- Aggregate performance by sport, competition, feature bucket, and strategy baseline.

Hermes should not receive raw personal account data or browser-session details.
