# POC Implementation Notes

This POC keeps the system non-mutating. It uses `agent-browser` for visual/DOM reconnaissance and a read-only content-service probe for structured odds data.

The active app implementation is Rust. The HTTP API is served by Axum and the operator shell is rendered with Dioxus SSR. The full `danske-spil-gambler` binary runs the API server or scheduled worker loop, while the separate `danske-spil-result-agent` binary runs the paper-only result reconciliation service without compiling the Dioxus UI dependency graph.

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
GET /api/odds/movement
```

This endpoint is meant for feed-quality inspection before strategy work. It shows whether the scanner is actually identifying sports, competitions, market kinds, outcomes, and candidates across the configured sport scope.

`GET /api/odds/movement` compares the latest and previous stored observation for the same event, market, and outcome. It is an operator monitoring view only: it surfaces odds drift, active/displayed state, and observation timestamps without treating movement as a settlement-grade or placement signal.

When a candidate is inserted, the store also stamps the same latest-prior movement evidence into `candidate_bets.feature_snapshot.odds_movement` and the candidate rationale. Movement is classified as stable, normal, or large, and risk flags such as `odds_moved_up`, `odds_moved_down`, and `large_odds_movement` are persisted with the candidate. Numeric score changes remain gated behind reviewed strategy experiments.

## Sports Intelligence Feature Snapshots

The scanner also creates a first decision-time feature snapshot per observed event:

- Source registry row for the read-only Danske Spil content-service.
- Ingestion run metadata tied to the snapshot id.
- `market_context_v1` feature rows with event metadata, participant count, market count, outcome count, market kinds, external provider coverage, live/result flags, and missing-signal markers.

The feature snapshots are not a predictive model. They make the POC honest about what information is currently available and what is still missing. Weather, news, rankings, form, and injury/availability are recorded as missing until real sources are added.

Coverage is available at:

```text
GET /api/intelligence/coverage
```

## Strategy Baseline And Experiment POC

The service persists a paper-only `poc_ranker_v1` baseline and one-variable strategy experiment proposals:

- Active baseline state lives in `strategy_baselines`.
- Scan-derived proposals live in `strategy_experiments`.
- Operator actions live in `web_review_events`.
- Per-candidate baseline decisions live in `strategy_candidate_decisions`.
- The web UI shows proposal status and lets the operator approve, reject, activate, or promote proposals.

The current automatic proposals are deliberately conservative. If a scan produces enough long-price candidate risk, the service proposes lowering `max_decimal_odds` from `8.0` to `6.0`. If specialized-market exposure is more visible, the service proposes excluding those market kinds until settlement and feature coverage are stronger. If large odds movement is visible, it proposes excluding the `large_odds_movement` risk flag through `excluded_risk_flags`. Each proposal is evidence for review, not an autonomous behavior change.

Approved experiments can be replayed before activation or promotion. Replay compares the active baseline config with the proposed one over the proposal snapshot, stores selected/rejected deltas in `strategy_experiments.decision_payload.replay_evidence`, and does not place paper bets or change the active baseline. The UI only enables activation or promotion once replay evidence is present.

Each scan also applies the active baseline to every generated candidate and records a paper-only decision:

- `selected`: candidate is eligible for paper-ledger simulation.
- `rejected`: candidate failed one or more active baseline gates, such as max odds, minimum confidence, excluded market kind, or live-market restriction.

Rejected candidates cannot be added to the paper ledger through the API. This keeps manual simulation aligned with the active strategy while still preserving the rejected alternatives for review.

When `GAMBLER_AUTO_PAPER_ENABLED=true`, each scan automatically paper-places the top selected single-leg candidates up to `GAMBLER_AUTO_PAPER_PER_SCAN_LIMIT` and the `GAMBLER_AUTO_PAPER_MAX_OPEN_EXPOSURE` cap. If the active strategy baseline generates provider-supported candidate coupons, the scan also attempts to paper-place eligible coupons under the same stake and exposure caps. The Kubernetes worker currently runs this loop every 900 seconds, so the intended POC cadence is roughly every 15 minutes. The placement writes only to `simulated_bets` and `simulated_coupons`; it never clicks Danske Spil odds or submits a coupon. Operators can also trigger the same idempotent single-leg flow with:

```text
POST /api/simulate/selected
```

Operators can trigger the coupon placement pass with:

```text
POST /api/coupons/simulate/selected
```

The local Kubernetes POC keeps this paper-only exposure cap at 200 simulated currency units so the worker can continue taking new paper opportunities while unresolved positions are waiting for result review. The performance report shows remaining capacity and whether auto-paper placement is blocked by exposure:

```text
GET /api/performance
GET /api/performance/history
```

`GET /api/status` exposes the configured scanner cadence, scan limits, latest
snapshot age, and next scan due time so the web UI can show whether the roughly
15-minute worker loop is fresh or overdue.

`GET /api/intelligence/coverage` includes recent `ingestion_runs`, and the web
UI renders them as scanner audit history with source, status, sports, event
count, and snapshot id.

`GET /api/audit/events` exposes recent immutable audit events written by scan,
paper-placement, settlement, daily-reflection, and strategy-review flows. The
web UI renders the latest events for operator inspection.

Each scan writes a row to `simulation_performance_snapshots` after placement,
settlement queueing, and review refresh complete. The web UI shows this history
so exposure and due-review pressure can be compared across scan cycles.

Each scan evaluates a broader ranked candidate window than the two-per-scan
placement cap. Duplicate-covered opportunities are skipped, and the first
eligible lower-ranked selections are paper-placed until the scan limit or
exposure cap is reached.

The worker and UI can also advance open paper bets into `awaiting_result` once the observed event start time has passed:

```text
GET  /api/ledger/queue
POST /api/ledger/queue
```

This is only a queue transition. It does not grade a bet as won, lost, or void. Paper bets and paper coupons now store the observed event start time and an expected finish timestamp, then re-check awaiting-result items on the same 15-minute worker cadence until the result is verified or the item needs manual review.

Strategy state is available at:

```text
GET /api/strategy
GET /api/strategy/decisions
POST /api/strategy/experiment/review  # actions: approve, reject, replay, activate, promote, rollback
```

## Safety Boundary

Do not click odds or `Tilføj kupon` during POC runs. The POC should only read navigation, DOM, and content-service data.

## Candidate Ranking POC

The API stores a conservative `poc_ranker_v1` watchlist score for each candidate. The score is not a betting recommendation; it exists to make candidate ordering, paper-ledger choices, and Hermes review replayable.

Stored candidate fields include:

- Implied probability from the observed decimal odds.
- A first-pass model probability from odds shape, market kind, and metadata completeness.
- Expected value, confidence, and score.
- Risk flags such as missing participants, specialized market, long-horizon market, line market, very short price, long price, or odds movement.
- Active strategy baselines can reject candidates by `excluded_risk_flags`; movement-driven exclusions are proposed through Hermes review before activation.
- A feature snapshot containing decision-time metadata from the normalized odds feed, including odds movement when a previous observation exists for the same event, market, and outcome.

The ranker intentionally uses weak, transparent heuristics until sports intelligence ingestion is wired in. Later model versions should replace these heuristics with feature snapshots from stats, news, weather, form, and settlement history.

## Coupon Strategy POC

The strategy model should support single-leg candidates and provider-supported multi-leg coupon candidates. The initial baseline keeps only singles enabled, but the config shape reserves explicit switches for doubles, triples, and larger accumulators.

Before a strategy can create a multi-leg paper coupon, it must prove from the observed market metadata that the provider allows the legs to be combined. The normalized Danske Spil market payload preserves `minimum_accumulator` and `maximum_accumulator`, and scanner runs persist those bounds in `coupon_rule_observations` for operator review. Unknown cross-sport, cross-category, and market-exclusion restrictions are explicitly retained as unknowns until browser or feed investigation proves them.

Multi-leg candidates should store:

- Coupon type: single, double, triple, or accumulator.
- Leg count, combined decimal odds, and per-leg observed odds.
- Provider rule evidence that allowed the combination.
- Same-sport or same-category validation result where required.
- Leg-level rationale, risk flags, and settlement state.
- Coupon-level simulated stake, return, and profit/loss.

The web UI should label these as simulated coupons and keep real submission disabled.

Current implementation status:

- `candidate_coupons` and `candidate_coupon_legs` are initialized in Postgres.
- `coupon_rule_observations` stores observed provider accumulator metadata by sport, event, market, and snapshot.
- `simulated_coupons` and `simulated_coupon_legs` are initialized in Postgres for paper-only coupon placements.
- Scans attempt coupon-candidate generation after single-leg strategy decisions are stored.
- `/api/coupon-rules` lists recent provider accumulator-rule observations and per-sport summary counts.
- `/api/coupons` lists stored multi-leg coupon proposals.
- `/api/coupons/generate` re-runs generation for the latest or supplied snapshot.
- `/api/coupons/simulate` writes a candidate coupon into the paper coupon ledger without opening or submitting a provider coupon.
- `/api/coupons/simulate/selected` walks ranked candidate coupons for the latest or supplied snapshot and paper-places eligible coupons until the per-scan or exposure cap is reached.
- `/api/coupons/simulated` lists simulated coupon placements with leg evidence, stake, combined odds, status, and simulated P/L.
- `/api/coupons/settle` allows manual paper settlement of a simulated coupon as won, lost, void, pushed, refunded, cancelled, postponed, or unresolved.
- The Dioxus UI shows provider coupon rules, candidate coupons, and simulated coupons as separate tables, with real submission still disabled.
- The Dioxus UI shows an odds movement table so repeated scanner runs can be inspected for price drift by event, market, and outcome.
- The settlement queue now treats simulated coupons as first-class paper-ledger items: a coupon moves to `awaiting_result` only after the latest leg start time has passed, and its legs move with it.
- The default baseline still disables doubles, triples, and accumulators. When a scan observes enough same-sport, distinct-event selections with provider accumulator metadata for a double, Hermes can propose a reviewed `coupon_modes` experiment that enables paper doubles only.
- `POST /api/hermes/reflect/yesterday` records an idempotent daily Hermes reflection for the previous Europe/Copenhagen calendar day. It summarizes performance snapshots, paper placements, current open/realized status, settlement observations, and whether results are evaluable.
- Each successful scan refreshes that previous-day reflection automatically so manual settlements or refund/cancellation outcomes update the daily record without a separate operator action.

## Paper Settlement POC

The web UI can manually settle paper-ledger rows as won, lost, void, pushed, refunded, cancelled, postponed, or unresolved. This writes settlement metadata and simulated return/profit-loss fields to Postgres. Refunded, cancelled, abandoned, void, and pushed outcomes return the simulated stake with zero P/L. Postponed remains open exposure so the worker and operator keep rechecking it. Manual settlement is a placeholder for the planned result-lookup worker; it should only be used when the operator has verified the result from an acceptable source.

The planned automated settlement worker should handle normal final results plus cancelled, postponed, abandoned, voided, pushed, and agency-refunded outcomes. These states should keep their source evidence and grading rule in Postgres so simulation metrics can distinguish real losses from stake returns or unresolved events.

Current result-review status:

- `GET/POST /api/settlement/review` refreshes review evidence for `awaiting_result`, `unresolved`, and `postponed` paper bets.
- `GET /api/result-agent/queue` turns due or stale settlement-review rows into read-only result-agent tasks, including source precedence, known result links, search terms, expected finish timing, and whether a local Danske Spil account-history browser agent is available from environment presence checks.
- `GET /api/settlement/sources` lists approved settlement-capable source classes from `source_registry`.
- `GET /api/settlement/source-links` lists operator-managed public result URLs that have been added for stale event review.
- `GET /api/settlement/observations` lists recent manual settlement observations for audit and Hermes-safe review.
- `GET /api/settlement/lookup-attempts` lists recent non-grading review-loop lookup attempts for due paper singles and coupons.
- The same review endpoint also refreshes simulated coupon evidence with leg-level event, market, outcome, latest price, and result-state metadata.
- Review rows include `last_lookup_at`, `lookup_stale`, and the lookup cooldown so operators can see whether each due paper single or coupon has a fresh non-grading lookup attempt.
- Strategy played and sport performance summaries aggregate both single simulated bets and multi-leg simulated coupons.
- The web UI renders recent strategy plays from `/api/strategy/played`, including singles and coupons with stake, odds, status, score, and confidence.
- `/api/strategy/played` also aggregates paper performance by candidate risk flag, including movement tags such as `large_odds_movement`, so strategy gates can be reviewed against realized paper outcomes before promotion.
- The worker runs the same review refresh after advancing the settlement queue.
- `simulated_bets`, `simulated_coupons`, and `simulated_coupon_legs` preserve event start and expected finish timestamps for operator scheduling.
- Review evidence is written into each bet's `settlement_payload.review_evidence`, including the approved settlement source policy order used for manual grading.
- Coupon review evidence is written into each simulated coupon's `settlement_payload.review_evidence`, including the same settlement source policy order.
- Manual settlement must cite a `source_registry` row where `can_settle=true`; the settlement payload preserves earlier review evidence and adds the selected source policy under `manual_settlement.source_policy`.
- The review queue joins paper bets to the latest observed event, market, and outcome payloads from the Danske Spil content feed.
- Each review refresh records `settlement_lookup_attempts` rows with the source key, recommendation, current event/outcome state, and the approved settlement source policy. Writes are throttled by `GAMBLER_SETTLEMENT_LOOKUP_COOLDOWN_MINUTES` so repeated UI refreshes do not create duplicate attempts inside the intended 15-minute recheck cadence.
- `/api/performance` now includes `settlement_work.lookup_cadence`, which reports how many due paper positions have a recent lookup attempt, how many are due without a recent attempt, the last lookup time, and the next lookup due time.
- The same performance payload includes `settlement_work.lookup_due_items`, a capped operator queue of due paper singles or coupons whose latest lookup is missing or outside the cooldown window.
- The system recommends `manual_grade_ready`, `manual_void_or_refund_review`, `external_result_required`, `expected_finish_passed_recheck`, or `await_more_evidence`.
- If a paper position is still not resulted or settled more than 2 hours after the sport-specific expected event finish time, the review loop escalates it to `external_result_required` and recommends external evidence sources instead of continuing to rely only on stale Danske Spil content-feed state. For example, football uses kickoff plus roughly 130 minutes as the expected finish, then waits another 2 hours before external auto-checking. Current external review sources are `official_competition_results`, `flashscore_results`, `sofascore_results`, `livescore_results`, and `documented_third_party_results`.
- The external auto-settlement POC only handles single-leg winner markets when a stable source URL is configured, the fetched page exposes a parseable final score, and the selected outcome maps deterministically to home, away, or draw. Every automatic settlement writes `paper_bet_auto_settled_external` audit evidence with the source URL, source title, score, selected outcome, and result.
- Direct Sofascore HTTP fetches returned 403 in local testing even with browser-like request headers, while `agent-browser` could load the same match page and expose the final result text. Sofascore source rows are therefore flagged as requiring browser automation, and the direct auto-settler skips them with an audited `browser_automation_required_for_source` reason.
- Browser-backed result evidence can be submitted through `POST /api/settlement/external-evidence`. The payload stores an `external_result_evidence` row and, when `settle` is true, attempts deterministic paper settlement for matching open single-leg winner markets only. The evidence payload should include `source_key`, `source_url`, `event_name`, `home_name`, `away_name`, `home_score`, `away_score`, and a short `raw_text_excerpt`; it must not include cookies, credentials, browser storage, or full account payloads.
- `scripts/external_result_evidence_probe.py` is the local operator probe for browser-backed Sofascore, Flashscore, and LiveScore evidence. It opens the public match URL with `agent-browser`, extracts a compact final-score payload, and posts to the API only when not run with `--dry-run`. It defaults to `settle=false`, and explicit score/team arguments can be supplied when the page loads but does not expose a parseable final score.
- The dedicated Rust result-agent deployment consumes `/api/result-agent/queue` on its own cadence and runs built-in Flashscore discovery for stale football, tennis, and basketball rows that do not yet have configured public result links. It stores the discovered source link and posts sanitized evidence through `POST /api/settlement/external-evidence` when a final score is available.
- `scripts/result_agent.py` remains available for local diagnostics and browser-only public result probes for configured links, so stale rows do not require operator URL prompts.
- Settlement-review rows include all configured external result links when known match URLs exist, including whether each source requires browser evidence. The direct auto-check attempts every non-browser source before reporting that browser evidence is required.
- Operators can still attach additional public result URLs through `POST /api/settlement/source-link` as a fallback integration point. The normal UI path now favors the result-agent queue instead of prompting operators to paste URLs.
- It still does not auto-grade won/lost from the Danske Spil content feed because those result semantics have not been proven for each market type.
