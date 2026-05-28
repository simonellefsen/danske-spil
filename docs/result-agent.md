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

## Local Public-Source Agent

For known public result URLs, run:

```text
rtk kubectl --context docker-desktop -n danske-spil port-forward svc/gambler-api 18083:8080
rtk python3 scripts/result_agent.py --api http://127.0.0.1:18083 --browser-only --dry-run
```

Remove `--dry-run` after reviewing the extracted payload. Add `--settle` only
when deterministic paper settlement is intended. Direct HTTP result links are
already attempted by the worker; `--browser-only` focuses the local browser
agent on sources such as Sofascore that block direct fetches.

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

The current implementation creates the queue and local public-source agent.
The next implementation step is a local read-only Danske Spil account-history
agent that uses an authenticated browser session to read settled coupon history
and submit the same sanitized evidence payload.
