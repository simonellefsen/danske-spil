---
type: source-note
tags:
  - danske-spil/wiki
  - browser
  - oddset
  - tips
updated: 2026-05-25
sources:
  - /Users/lindau/codex/danske-spil/docs/poc-implementation.md
---

# Danske Spil DOM And Content-Service Observations

Initial anonymous `agent-browser` reconnaissance found that Oddset can be inspected without login for navigation, market, event, and odds metadata. The POC did not submit bets, click odds, deposit, withdraw, or mutate account state.

## Browser Session

- Session name: `danske-spil-poc`.
- Initial URL: `https://danskespil.dk/oddset`.
- Cookie modal: dismissed with `Fravælg alle` so optional tracking is not accepted.
- Captured artifacts should stay under ignored `tmp/browser-observations/`.

## Sports Navigation

Observed sport links:

- Football/soccer: `/oddset/sport/12/fodbold/matches`
- Tennis: `/oddset/sport/854/tennis/matches`
- Basketball: `/oddset/sport/465/basketball/matches`
- Motorsport: `/oddset/sport/319/motorsport/matches`
- Golf: `/oddset/sport/561/golf/matches`
- Cycling: `/oddset/sport/660/cykling/matches`

Formula 1 appears under Motorsport:

- Competition: `/oddset/sports/competition/17711/motorsport/formel-1/formel-1/matches`
- Outrights example: `/oddset/sports/competition/17711/motorsport/formel-1/formel-1/outrights`

## Content-Service Endpoint

The rendered page calls `https://content.sb.danskespil.dk/content-service/api/v1/q/time-band-event-list`.

Useful query fields observed:

- `drilldownTagIds`: sport id from the navigation URL.
- `maxMarkets`: number of markets to include per event.
- `includeChildMarkets=true`.
- `includeCommentary=true`.
- `includeIncidents=true`.
- `includeMedia=true`.
- `useMarketGroupCodeCombis=true`.
- `lang=da-DK`.
- `channel=I`.

Formula 1 outrights were observed through `event-list` with `eventSortsIncluded=TNMT` and `drilldownTagIds=17711`.

## Event Fields

The content-service event objects include:

- Event id, name, status, start time, started/live/resulted/settled flags.
- Category/sport name and sport code.
- Class and type for country, league, competition, tournament, or series grouping.
- Home/away teams for team events.
- External provider ids, including Betradar, Betgenius, Enetpulse, and LSports where available.
- Market count and market summaries.
- Commentary facts for live score, cards, corners, penalties, and period clocks when available.

## Market And Outcome Fields

Market objects include:

- Market id, name, template id, group code, status, display flags, and accumulator constraints.
- Market types observed include `Kampvinder`, handicap, totals/over-under, both-teams-score, double chance, combinations, and future/outright markets.
- Football live commentary facts include score, cards, penalties, and corners.
- Tennis commentary facts include max sets, and match markets can be expanded later into set/game markets.
- Basketball markets include moneyline, handicap, and points over/under lines; quarter/period lines should be captured in the next deeper event-market pass.
- Formula 1 markets include season outrights and driver/team head-to-head outcomes.
- Cycling markets include stage head-to-head outcomes.
- Outcome objects include name, home/draw/away subtype where relevant, status, active/display flags, result state, decimal odds, fractional odds, and handicap line fields.

## POC Scripts

- `scripts/agent_browser_poc.sh` captures anonymous page artifacts.
- `scripts/probe_danskespil_content.py` fetches and normalizes read-only content-service JSON for the initial sports scope.
