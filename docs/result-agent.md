# Result Agent

The result agent is the automated settlement-evidence worker for stale paper
positions. It does not place bets. Its job is to find trustworthy final-result
truth, store sanitized evidence, and let the existing paper-settlement logic
grade only markets it can map deterministically.

## Queue

The Rust service exposes:

```text
GET /api/result-agent/queue
```

The queue is built from settlement-review rows that are due, overdue, ready to
grade, or require cancellation/refund review. Each task includes:

- Paper bet or coupon ids.
- Event, sport, competition, market, outcome, and coupon-leg context.
- Expected result-check timestamp and overdue minutes.
- Known public result links, when configured.
- Deterministic search terms for a result-source discovery worker.
- Recommended action for a read-only browser result agent.

The queue deliberately avoids raw cookies, credentials, browser storage, and
account payloads.

## Alias Registry

Aliases are stored centrally in `entity_aliases`, not only on individual result
links. The registry supports teams, players, leagues, competitions, drivers,
golfers, riders, and generic participants. Each row stores the canonical name,
alias, normalized keys, optional sport, optional gender scope, optional
source/external id, confidence, and a small paper-only payload. Gender scope is
used to distinguish men, women, and mixed competitions when the same club,
league, or participant name appears across multiple competition contexts.

The API surface is:

```text
GET  /api/aliases
POST /api/aliases
```

`POST /api/aliases` accepts `entity_kind`, optional `sport_key`, optional
`gender_scope` (`men`, `women`, or `mixed`), `canonical_name`, `alias_name`,
optional `source_key`, optional `external_id`, optional `confidence`, and
optional `notes`.

Result-source automation records aliases automatically when it adds a public
result link or ingests external result evidence. Settlement matching expands
home/away aliases from this registry before grading, so names learned from one
source can help later Flashscore, Sofascore, LiveScore, official, or account
history checks.

## Source Order

Result evidence should be collected in this order:

1. Read-only Danske Spil account or coupon history, when an authenticated local
   browser session is available.
2. Official league, tournament, federation, or event result pages.
3. Flashscore match pages.
4. Sofascore match pages.
5. LiveScore match pages.

The Danske Spil account path is useful because account history can show the
bookmaker's own settlement, cancellation, push, refund, or postponed state.
That agent must run locally with an operator-controlled browser session and
post only compact settlement facts to the API.

## Built-In Public-Source Agent

The Kubernetes POC runs a dedicated `gambler-result-agent` deployment for the
read-only public-source result-agent pass. It consumes
`GET /api/result-agent/queue`, discovers missing Flashscore result links for
supported sports, stores the durable source link, and posts sanitized
final-score evidence when the event is finished. The same cycle can be
triggered manually from the web UI or API:

```text
POST /api/result-agent/run
```

Kubernetes enables this by default with
`GAMBLER_RESULT_AGENT_ENABLED=true`, schedules the dedicated service with
`GAMBLER_RESULT_AGENT_INTERVAL_SECONDS=900`, and limits each cycle with
`GAMBLER_RESULT_AGENT_PER_CYCLE_LIMIT`. The scanner worker sets
`GAMBLER_RESULT_AGENT_ENABLED=false` so it advances settlement-review state
without also running public result reconciliation.

## Local Browser Public-Source Agent

The optional local Python runner remains useful for browser-only public pages
or diagnostics. It consumes the queue and has two paths:

- Configured result links are probed directly or through browser automation.
- Missing links are first resolved through Flashscore participant search and
  event feeds for supported sports, then the discovered result link and compact
  final-score evidence are posted back to the API.

Run it with:

```text
rtk kubectl --context docker-desktop -n danske-spil port-forward svc/gambler-api 18083:8080
rtk python3 scripts/result_agent.py --api http://127.0.0.1:18083 --dry-run
```

Remove `--dry-run` after reviewing the extracted payload. Add `--settle` only
when deterministic paper settlement is intended. `--browser-only` focuses the
runner on links that require browser automation and skips direct HTTP links.
`--no-discover` disables automated Flashscore discovery and restores the older
"only configured links" behavior.

Built-in Flashscore discovery currently covers football, tennis, and basketball
where a participant feed exposes the event row and final score. Participant
lookup expands common aliases before search, including Danish country names,
Flashscore basketball naming differences, tennis first/last-name order, and
gender-scoped team variants. The feed fetch falls back to Flashscore's stable
`x-fsign` value when current pages do not expose a page-local `feed_sign`. If a
row still returns `flashscore_discovery_no_match`, the next step is to add
another source adapter or a sport-specific pagination path, not an operator
prompt.

## Evidence Contract

All automated result workers submit evidence through:

```text
POST /api/settlement/external-evidence
```

Accepted evidence is limited to sanitized facts such as source key, source URL,
event name, participant names, final score, confidence, and a short text
excerpt. It must not include credentials, cookies, browser storage, full account
pages, or hidden model reasoning.

## Current Boundary

The current implementation creates the queue, supports configured public result
links, and automatically discovers Flashscore result links for common stale
football, basketball, and tennis rows from the scheduled Rust worker. Winner and
over/under markets can be graded from external final-score evidence. The next
implementation step is a local read-only Danske Spil account-history agent that
uses an authenticated browser session to read settled coupon history and submit
the same sanitized evidence payload, especially for cancellations, refunds, and
markets that cannot be graded from a plain final score.
