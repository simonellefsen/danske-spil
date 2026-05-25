# POC Implementation Notes

This POC keeps the system non-mutating. It uses `agent-browser` for visual/DOM reconnaissance and a read-only content-service probe for structured odds data.

The active app implementation is Rust. The HTTP API is served by Axum, the operator shell is rendered with Dioxus SSR, and the same binary runs either the API server or the scheduled worker loop.

## Agent Browser Setup

Use a dedicated browser session:

```bash
rtk bash scripts/agent_browser_poc.sh
```

The script:

- Opens Oddset anonymously.
- Selects the restrictive cookie option if the consent modal appears.
- Captures an interactive snapshot and screenshot.
- Extracts sports links, event links, market chips, and odds button text.
- Opens Tips and captures the same basic artifacts.

Artifacts are written under `tmp/browser-observations/`, which is ignored by Git.

## Structured Content Probe

The page loads odds data from a read-only content-service endpoint. Probe the initial sport scope:

```bash
rtk python3 scripts/probe_danskespil_content.py --sport all --limit 3 --max-markets 8 --pretty
```

The script normalizes:

- Sports and drilldown ids.
- Competitions/leagues/tournaments.
- Events and external provider ids.
- Teams/participants.
- Markets and market group codes.
- Outcome names and decimal odds.
- Handicap and over/under line fields.
- Live scoreboard facts such as score, cards, and corners when available.
- Formula 1 outright markets from the Formula 1 competition feed.

By default the probe filters out started/live events and obvious virtual/eSports spillover. Add `--include-live` when the monitoring POC needs live clocks, scores, corners, cards, or in-play prices. Add `--date-days N` only when a specific upcoming date band is needed; the site may otherwise expose useful near-term events without a date band.

## Observed Sport Navigation

Initial anonymous navigation exposed these useful sport entry points:

- Football/soccer: `/oddset/sport/12/fodbold/matches`
- Tennis: `/oddset/sport/854/tennis/matches`
- Basketball: `/oddset/sport/465/basketball/matches`
- Motorsport/Formula 1: `/oddset/sport/319/motorsport/matches`
- Golf: `/oddset/sport/561/golf/matches`
- Cycling: `/oddset/sport/660/cykling/matches`

Formula 1 appears under Motorsport, with a competition page at `/oddset/sports/competition/17711/motorsport/formel-1/formel-1/matches`.

## Observed Bet Structure

The content-service event model currently exposes:

- `event.id`, `event.name`, `event.startTime`, status, live/result/settlement flags.
- `category` for sport display name and code.
- `class` and `type` for country/competition grouping.
- `teams` with `HOME` and `AWAY` sides for team events.
- `externalIds` from providers such as Betradar, Betgenius, Enetpulse, and LSports.
- `markets` with names such as `Kampvinder`, group codes such as `MATCH_RESULT`, and accumulator constraints.
- `outcomes` with names, home/draw/away subtypes, active/display status, decimal odds, and handicap line values.
- `commentary.facts` for live facts such as score, corners, cards, and penalties when available.

The rendered page also exposes market selector chips such as:

- `Kampvinder`
- `Antal mål`
- `Handicap`
- `Begge hold scorer`
- `Dobbeltchance`
- Tennis set/game market labels where available
- Basketball period/quarter lines where available
- Over/under buttons using `O` and `U`
- Formula 1 season outrights and head-to-head driver/team markets
- Cycling stage head-to-head markets

Golf currently returns no anonymous match or outright events from the observed feeds when probed on 2026-05-25. Keep it in scope, but treat it as a feed-discovery item for the next browser pass.

## Normalized Market Catalog

Each Rust scanner run now persists both the raw snapshot and a normalized catalog:

- `sports`: sport keys, labels, drilldown ids, and source sport codes.
- `competitions`: league, tournament, or class grouping observed for a sport.
- `sport_events`: events, start times, live/result/settlement flags, and raw event payloads.
- `event_participants`: teams or participants attached to observed events when the feed exposes them.
- `market_observations`: per-snapshot market rows with market kind, group code, active/display state, and outcome count.
- `outcome_observations`: per-snapshot outcome rows with odds, active/display state, subtype, and handicap/line values.

The web UI and API expose catalog coverage at:

```text
GET /api/catalog/coverage
```

This endpoint is meant for feed-quality inspection before strategy work. It shows whether the scanner is actually identifying sports, competitions, market kinds, outcomes, and candidates across the configured sport scope.

## Safety Boundary

Do not click odds or `Tilføj kupon` during POC runs. The POC should only read navigation, DOM, and content-service data.

## Candidate Ranking POC

The API stores a conservative `poc_ranker_v1` watchlist score for each candidate. The score is not a betting recommendation; it exists to make candidate ordering, paper-ledger choices, and Hermes review replayable.

Stored candidate fields include:

- Implied probability from the observed decimal odds.
- A first-pass model probability from odds shape, market kind, and metadata completeness.
- Expected value, confidence, and score.
- Risk flags such as missing participants, specialized market, long-horizon market, line market, very short price, or long price.
- A feature snapshot containing only decision-time metadata from the normalized odds feed.

The ranker intentionally uses weak, transparent heuristics until sports intelligence ingestion is wired in. Later model versions should replace these heuristics with feature snapshots from stats, news, weather, form, and settlement history.

## Paper Settlement POC

The web UI can manually settle paper-ledger rows as won, lost, or void. This writes settlement metadata and simulated return/profit-loss fields to Postgres. Manual settlement is a placeholder for the planned result-lookup worker; it should only be used when the operator has verified the result from an acceptable source.
