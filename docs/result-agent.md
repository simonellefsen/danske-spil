# Result Agent

The result agent is the automated settlement-evidence worker for stale paper
positions. It does not place bets. Its job is to find trustworthy final-result
truth, store sanitized evidence, and let the existing paper-settlement logic
grade only markets it can map deterministically.

## Queue

The Rust service exposes:

```text
GET /api/result-agent/queue
GET /api/result-agent/account-requests
```

The queue is built from settlement-review rows that are due, overdue, ready to
grade, or require cancellation/refund review. Each task includes:

- Paper bet or coupon ids.
- Event, sport, competition, market, outcome, and coupon-leg context.
- Paper stake and priority score, so per-cycle work can focus on the highest exposure and oldest overdue rows first.
- Expected result-check timestamp and overdue minutes.
- Known public result links, when configured.
- Deterministic search terms for a result-source discovery worker.
- Recommended action for a read-only browser result agent.

The queue deliberately avoids raw cookies, credentials, browser storage, and
account payloads.

Tasks are ordered by priority score. The score is paper stake weighted by
overdue age, which keeps the agent deterministic while making stale,
higher-exposure rows run before low-impact backlog entries.

`POST /api/result-agent/run` returns the same priority accounting used by the
scheduled worker: queued task count/exposure, selected task count/exposure,
attempted discovery count/exposure, skipped exposure, and the highest selected
priority. This makes a capped result-agent cycle auditable without inspecting
raw database rows.
When a selected row already has a direct, non-browser public result link, the
cycle processes that `public_result_evidence_check` task immediately. The older
global overdue sweep remains as a backstop, but queued configured-link tasks no
longer wait for the 120-minute sweep before attempting direct evidence.

`GET /api/result-agent/queue` also includes the latest compact
`result_agent_cycle_completed` audit event as `latest_cycle` plus the most
recent compact cycle summaries as `recent_cycles`. It also returns
`cycle_health`, which compares the latest completed cycle with
`GAMBLER_RESULT_AGENT_INTERVAL_SECONDS` and marks the loop `current`, `stale`,
`no_cycle`, or `disabled`. The web UI renders these fields beside the backlog
so operators can compare the current queue with scheduled and manual
result-agent runs over time. Cycle summaries include a compact
`failure_summary` with top skipped reasons, Flashscore no-match diagnostic
reasons, and a few sanitized examples containing the search names and candidate
counts used for discovery. Flashscore no-match summaries also include
`recommended_actions`, such as adding or verifying a home/away alias, checking
gender scope, retrying with browser-backed evidence, or adding a non-Flashscore
source adapter.

`GET /api/result-agent/account-requests` exposes a focused subset for a local
read-only Danske Spil account-history browser agent. It is independent of
whether the Kubernetes API pod has credentials, because the intended worker is
operator-controlled and local. Each request includes paper bet/coupon ids,
selection context, paper stake, priority score, expected truth to inspect, and
an evidence template for `POST /api/settlement/external-evidence`. Requests use
the same priority order as the public result-agent queue.

The account-history request contract explicitly forbids storing credentials,
cookies, browser storage, payment data, Spil-ID/MitID payloads, or full account
pages. The first submitted payload should use `settle=false` unless the local
agent has deterministic bookmaker-settlement evidence for the paper row.

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
The public result-agent queue also expands the displayed event participants
through the alias registry before source discovery. Flashscore participant
search therefore tries learned names such as localized country names,
sponsor-heavy club names, gender-scoped team variants, and source-specific
team/player names when a row has no configured result link yet.

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

The Kubernetes POC runs a dedicated `gambler-result-agent` deployment and
slim `danske-spil-result-agent` image for the read-only public-source
result-agent pass. It consumes
`GET /api/result-agent/queue`, discovers missing Flashscore result links for
supported sports, stores the durable source link, and posts sanitized
final-score evidence when the event is finished. The same cycle can be
triggered manually from the web UI or API:
For team sports, the Flashscore discovery step requires a stronger selected
participant match than a single shared token. This prevents rows such as a
city-name-only match from being resolved through an unrelated club and records
`home_participant_low_confidence` or `away_participant_low_confidence` in the
cycle diagnostics instead of probing irrelevant feeds.
The queue also respects the settlement lookup cooldown. Rows whose latest lookup
attempt is still fresh remain visible in settlement review, but they are not
emitted as result-agent or account-history tasks until `lookup_stale=true`.

```text
POST /api/result-agent/run
```

Kubernetes enables this by default with
`GAMBLER_RESULT_AGENT_ENABLED=true`, schedules the dedicated service with
`GAMBLER_RESULT_AGENT_INTERVAL_SECONDS=900`, and caps each cycle with
`GAMBLER_RESULT_AGENT_PER_CYCLE_LIMIT=25`. The cap is deliberately above the
normal backlog size so stale rows at the end of the priority list are still
rechecked during manual runs. The scanner worker sets
`GAMBLER_RESULT_AGENT_ENABLED=false` so it advances settlement-review state
without also running public result reconciliation. The web/API deployment sets
`GAMBLER_RESULT_AGENT_URL=http://gambler-result-agent:8080`, so UI-triggered
queue and run requests are forwarded to the dedicated result-agent service.

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
`x-fsign` value when current pages do not expose a page-local `feed_sign`.
Alias expansion combines built-in aliases with the central `entity_aliases`
registry populated by prior source links and external evidence, so each
successful or operator-seeded match can improve future no-link discovery.
Friendly and neutral-ground football matches can be listed with arbitrary team
order across providers. Settlement matching therefore treats `A - B` and
`B - A` as the same external-result lookup target, accepts localized
Flashscore domains, and orients winner-market grading by participant aliases
before applying the final score. This prevents a source-side `Irak - Andorra
1:0` result from being graded as if it were event-side `Andorra - Irak 1:0`.
The same alias path handles Danish/localized provider names and person-name
order changes, for example `Bosnien-Hercegovina` versus `Bosnia and
Herzegovina`, `Irak` versus `Iraq`, women markers such as `(k)`/`(W)`, and
tennis source names such as `Paul Tommy` for `Tommy Paul`.
When a neutral-friendly source lists teams in the opposite order, a seeded
known result stores the score in the Danske Spil event order and keeps the
source URL in the audit payload so automatic settlement remains deterministic.
The same rule is used for basketball rows where Flashscore may list the URL
participants in the reverse order, for example `Fuenlabrada - Palencia`, or
where a Flashscore match URL has no `mid` query and only exposes a page-title
score such as `NSA - Antonine 77:84`.
Basketball and football aliases also normalize sponsor-heavy or abbreviated
names such as `Rinascita Basket Rimini` to `Rimini`, `Ueb Cividale` to
`Cividale`, `Baskonia Vitoria-Gasteiz` to `Baskonia`, `Cb Malaga` to
`Malaga`, `Næstved BK` to `Naestved`, and `Fortaleza C.E.I.F. FC` to
`Fortaleza`. Football aliases also normalize `Paris SG`, `PSG`, and
`Paris Saint-Germain`. Women-team aliases also account for Flashscore naming such as
`Vasco W`, `America Mineiro W`, `America de Cali W`, and `Inter Palmira W`.
Football knockout or cup matches can expose two valid scores: the regulation
score used for a normal `Kampvinder`/match-winner market, and the decided-winner
score after extra time or penalties. Flashscore feeds may publish the decided
score in the primary final fields and the regulation score in separate fields.
Xscores can represent the same type of match as a full-time score plus a
penalty-shootout score, for example full time 1:1 and penalties 4:3.
The result agent therefore grades normal football winner markets from regulation
time when that score is available, while markets that explicitly include extra
time, penalties, qualification, or advancement keep using the decided-winner
score. Explicit yes/no penalty-shootout markets are settled from the presence
of a penalty-shootout score. Settlement notes record regulation, penalty, and
decided score values plus the `grading_score_basis`.
If a row still returns `flashscore_discovery_no_match`, the next step is to add
another source adapter or a sport-specific pagination path, not an operator
prompt.
Tennis doubles use a dedicated Flashscore path. The agent searches the
individual players on each side, matches both player ids in the participant
feed row, and accepts provider-reversed pair order when the player-id sets are
otherwise exact. Source URLs and aliases are built from the player ids that
actually appeared in the matched feed row, so same-surname search candidates do
not pollute later alias matching. Tennis doubles links also bypass the central
participant-alias registry on read and write, because temporary pair aliases are
not stable enough to share globally. When Flashscore has a matched doubles row
with no final score but a terminal no-play/status marker, the paper ledger is
settled as `refunded` and the audit payload records the source URL, event id,
stage, raw row preview, and `paper_only=true`.

## Local Account-History Agent

`scripts/account_history_agent.py` is the local read-only companion for
`GET /api/result-agent/account-requests`. It opens
`DANSKESPIL_ACCOUNT_HISTORY_URL` when configured, otherwise
`DANSKESPIL_LOGIN_URL`, in an `agent-browser` session and inspects the visible
account/history text locally. The script never posts page text in full. It
matches queued paper rows to visible event context, accepts only deterministic
bookmaker states, and sends compact status-only evidence to
`POST /api/settlement/external-evidence`.

Typical run:

```text
rtk kubectl --context docker-desktop -n danske-spil port-forward svc/gambler-api 18083:8080
rtk python3 scripts/account_history_agent.py --api http://127.0.0.1:18083 --dry-run
```

Use an existing authenticated browser session or let the script open the page
so the operator can sign in locally. Add `--settle` only after dry-run output
shows deterministic bookmaker truth for the paper rows. `--no-open` inspects
the current `agent-browser` session page without navigating.
For parser development or sanitized fixtures, use `--history-text-file` or
`--extracted-json` together with `--requests-json`; these modes bypass
`agent-browser` and the API request queue and are suitable for offline tests.
Account-history source URLs are stored without query strings or fragments.
By default the agent defers non-terminal bookmaker states such as `afventer`,
`open`, `pending`, and `postponed`; use `--include-nonterminal` only for
diagnostic dry runs where those states should be emitted as payloads.
Coupon requests preserve all leg event names in the payload and use a synthetic
`Coupon: ...` event label when the request has no single event name.
Coupon account-history matching requires every leg event to be visible in the
local history context before emitting bookmaker evidence for the coupon.
When the same queued event appears in multiple visible account-history contexts
with conflicting deterministic statuses, the local agent skips the request as
ambiguous instead of posting settlement evidence.
The checked-in sanitized fixture can be exercised with:

```text
rtk make account-history-agent-fixture-dry-run
```

The web UI account-history panel exposes the same local runbook fields returned
by `GET /api/result-agent/account-requests`: the port-forward command, dry-run
command, script name, and the `DANSKESPIL_ACCOUNT_HISTORY_URL` environment
knob. This is intentionally informational only; the cluster does not launch the
operator browser or read account-history pages.

## Evidence Contract

All automated result workers submit evidence through:

```text
POST /api/settlement/external-evidence
```

Accepted evidence is limited to sanitized facts such as source key, source URL,
event name, participant names, final score, confidence, and a short text
excerpt. It must not include credentials, cookies, browser storage, full account
pages, or hidden model reasoning.

For `source_key=danskespil_account_history`, the endpoint also accepts
status-only bookmaker evidence when a final score is not available or is not
the right grading signal. The payload must include `bet_id` or
`coupon_simulation_id`, `event_name`, and a deterministic
`settlement_result`/`result_status` that normalizes to one of `won`, `lost`,
`void`, `pushed`, `refunded`, `cancelled`, `abandoned`, `postponed`, or
`unresolved`. Submit with `settle=false` for capture-only evidence; use
`settle=true` only when the bookmaker account history is deterministic and the
paper ledger should be reconciled from that truth. The stored evidence payload
marks these rows with `mode=account_history_settlement_evidence` and
`score_available=false` when no score was supplied.
Coupon evidence may also include `event_names`; these leg event names are
preserved in the stored evidence payload and settlement notes for auditability.

## Current Boundary

The current implementation creates the queue, supports configured public result
links, and automatically discovers Flashscore result links for common stale
football, basketball, and tennis rows from the scheduled Rust worker. Winner and
over/under markets can be graded from external final-score evidence. The
account-history request endpoint and dashboard table define the sanitized
worklist for a local read-only Danske Spil account-history agent, and the local
Python worker can consume those requests and submit compact bookmaker-status
evidence. The API can persist or apply status-only account-history evidence for
cancellations, refunds, and markets that cannot be graded from a plain final
score. The next implementation step is to keep expanding source adapters and
operational observability so stale paper rows can be reconciled without
operator URL prompts.
