use crate::models::{CandidateBet, HermesReflection, LedgerSummary, SimulatedBet};
use anyhow::{anyhow, Context};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, CACHE_CONTROL, PRAGMA,
};
use reqwest::{Client as HttpClient, Url};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration as StdDuration;
use tokio_postgres::{Client, NoTls, Row, Transaction};
use uuid::Uuid;

const FLASHSCORE_BASE_URL: &str = "https://www.flashscore.com";
const FLASHSCORE_DEFAULT_XFSIGN: &str = "SW9D1eZo";

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS odds_snapshots (
  id text PRIMARY KEY,
  observed_at timestamptz NOT NULL,
  source text NOT NULL,
  mode text NOT NULL,
  sport_keys text[] NOT NULL,
  event_count integer NOT NULL,
  payload jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sports (
  sport_key text PRIMARY KEY,
  label text,
  drilldown_id text,
  sport_codes text[] NOT NULL DEFAULT '{}',
  first_seen_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  payload jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE TABLE IF NOT EXISTS competitions (
  id text PRIMARY KEY,
  sport_key text NOT NULL REFERENCES sports(sport_key) ON DELETE CASCADE,
  name text NOT NULL,
  class_name text,
  drilldown_tag_id text,
  first_seen_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  UNIQUE (sport_key, name)
);

CREATE TABLE IF NOT EXISTS sport_events (
  id text PRIMARY KEY,
  sport_key text NOT NULL REFERENCES sports(sport_key) ON DELETE CASCADE,
  competition_name text,
  event_name text,
  start_time timestamptz,
  status text,
  live_now boolean NOT NULL DEFAULT false,
  started boolean NOT NULL DEFAULT false,
  resulted boolean NOT NULL DEFAULT false,
  settled boolean NOT NULL DEFAULT false,
  first_seen_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  payload jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE TABLE IF NOT EXISTS event_participants (
  id text PRIMARY KEY,
  event_id text NOT NULL REFERENCES sport_events(id) ON DELETE CASCADE,
  name text NOT NULL,
  role text,
  first_seen_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  UNIQUE (event_id, name, role)
);

CREATE TABLE IF NOT EXISTS entity_aliases (
  id text PRIMARY KEY,
  entity_kind text NOT NULL,
  sport_key text,
  gender_scope text,
  canonical_name text NOT NULL,
  canonical_key text NOT NULL,
  alias_name text NOT NULL,
  alias_key text NOT NULL,
  source_key text,
  external_id text,
  confidence numeric NOT NULL DEFAULT 0.75,
  payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  first_seen_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE entity_aliases ADD COLUMN IF NOT EXISTS gender_scope text;

DROP INDEX IF EXISTS idx_entity_aliases_unique;
CREATE UNIQUE INDEX IF NOT EXISTS idx_entity_aliases_unique
ON entity_aliases (
  entity_kind,
  COALESCE(sport_key, ''),
  COALESCE(gender_scope, ''),
  canonical_key,
  alias_key,
  COALESCE(source_key, ''),
  COALESCE(external_id, '')
);

DROP INDEX IF EXISTS idx_entity_aliases_alias_key;
CREATE INDEX IF NOT EXISTS idx_entity_aliases_alias_key
ON entity_aliases (entity_kind, COALESCE(sport_key, ''), COALESCE(gender_scope, ''), alias_key);

CREATE TABLE IF NOT EXISTS market_observations (
  id text PRIMARY KEY,
  snapshot_id text NOT NULL REFERENCES odds_snapshots(id) ON DELETE CASCADE,
  event_id text NOT NULL REFERENCES sport_events(id) ON DELETE CASCADE,
  market_id text,
  market_name text,
  market_kind text,
  group_code text,
  active boolean,
  displayed boolean,
  bet_in_run boolean,
  outcome_count integer NOT NULL DEFAULT 0,
  observed_at timestamptz NOT NULL DEFAULT now(),
  payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  UNIQUE (snapshot_id, event_id, market_id)
);

CREATE TABLE IF NOT EXISTS coupon_rule_observations (
  id text PRIMARY KEY,
  snapshot_id text NOT NULL REFERENCES odds_snapshots(id) ON DELETE CASCADE,
  sport_key text NOT NULL REFERENCES sports(sport_key) ON DELETE CASCADE,
  event_id text NOT NULL REFERENCES sport_events(id) ON DELETE CASCADE,
  market_observation_id text NOT NULL REFERENCES market_observations(id) ON DELETE CASCADE,
  market_id text,
  market_name text,
  market_kind text,
  group_code text,
  competition_name text,
  minimum_accumulator integer,
  maximum_accumulator integer,
  restriction_scope text NOT NULL,
  observed_at timestamptz NOT NULL DEFAULT now(),
  payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  UNIQUE (snapshot_id, event_id, market_id)
);

CREATE TABLE IF NOT EXISTS outcome_observations (
  id text PRIMARY KEY,
  snapshot_id text NOT NULL REFERENCES odds_snapshots(id) ON DELETE CASCADE,
  market_observation_id text NOT NULL REFERENCES market_observations(id) ON DELETE CASCADE,
  outcome_id text,
  outcome_name text,
  outcome_type text,
  outcome_sub_type text,
  decimal_odds numeric,
  active boolean,
  displayed boolean,
  handicap_low numeric,
  handicap_high numeric,
  observed_at timestamptz NOT NULL DEFAULT now(),
  payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  UNIQUE (snapshot_id, market_observation_id, outcome_id)
);

CREATE TABLE IF NOT EXISTS source_registry (
  source_key text PRIMARY KEY,
  source_name text NOT NULL,
  source_type text NOT NULL,
  url_pattern text,
  sport_scope text[] NOT NULL DEFAULT '{}',
  reliability numeric NOT NULL DEFAULT 0.5,
  can_settle boolean NOT NULL DEFAULT false,
  manual_review_required boolean NOT NULL DEFAULT true,
  notes text,
  first_seen_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  payload jsonb NOT NULL DEFAULT '{}'::jsonb
);

INSERT INTO source_registry (
  source_key, source_name, source_type, url_pattern, sport_scope,
  reliability, can_settle, manual_review_required, notes, payload
)
VALUES
  (
    'danskespil_account_history',
    'Danske Spil account or coupon history',
    'operator_settlement',
    'Danske Spil authenticated account/coupon history',
    ARRAY['football','tennis','basketball','motorsports','golf','cycling'],
    0.95,
    true,
    true,
    'Preferred source when accessible in an operator-controlled browser session. Never expose credentials or account payloads to Hermes.',
    '{"paper_only": true, "requires_operator_session": true, "priority": 1}'::jsonb
  ),
  (
    'official_competition_results',
    'Official league, tournament, or federation results',
    'official_result',
    'Official competition result pages',
    ARRAY['football','tennis','basketball','motorsports','golf','cycling'],
    0.90,
    true,
    true,
    'Use official event, league, tournament, or federation result pages when Danske Spil settlement history is unavailable.',
    '{"paper_only": true, "priority": 2}'::jsonb
  ),
  (
    'flashscore_results',
    'Flashscore match result pages',
    'third_party_result',
    'https://www.flashscore.com/match/*',
    ARRAY['football','tennis','basketball'],
    0.82,
    true,
    true,
    'Fallback match-result source for sports where a stable Flashscore match URL is available. Use only for manual paper-ledger review and preserve the URL used as evidence.',
    '{
      "paper_only": true,
      "priority": 3,
      "fallback_only": true,
      "known_matches": [
        {
          "event_name": "Lyngby - Horsens",
          "url": "https://www.flashscore.com/match/football/horsens-WIOwwITb/lyngby-tjPFkxq5/?mid=feSsAstf",
          "home_aliases": ["Lyngby", "Lyngby AC"],
          "away_aliases": ["Horsens", "AC Horsens"]
        },
        {
          "event_name": "Notts County - Salford City FC",
          "url": "https://www.flashscore.com/match/football/notts-county-EwJVdqzn/salford-W4AadhN3/?mid=E3uvsQPP",
          "home_aliases": ["Notts Co", "Notts County"],
          "away_aliases": ["Salford", "Salford City", "Salford City FC"]
        },
        {
          "event_name": "Dallas Wings - Las Vegas Aces",
          "url": "https://www.flashscore.dk/kamp/basketball/dallas-wings-WlAAvRyL/las-vegas-aces-nZjYLTCd/?mid=88ogphkR",
          "sport_key": "basketball",
          "gender_scope": "women",
          "home_aliases": ["Dallas Wings"],
          "away_aliases": ["Las Vegas Aces"]
        },
        {
          "event_name": "Casper Ruud - Tommy Paul",
          "url": "https://www.flashscore.dk/kamp/tennis/paul-tommy-pd3ye1BS/ruud-casper-zN9UpRqp/?mid=UHlr5dhM",
          "sport_key": "tennis",
          "gender_scope": "men",
          "home_aliases": ["Casper Ruud", "Ruud Casper", "Ruud"],
          "away_aliases": ["Tommy Paul", "Paul Tommy", "Paul"]
        },
        {
          "event_name": "Bosnien-Hercegovina - Nordmakedonien",
          "url": "https://www.flashscore.dk/kamp/fodbold/bosnien-herzegovina-fqe7WYTr/nordmakedonien-GrTQ3oHB/?mid=4lrOQdEN",
          "sport_key": "football",
          "home_aliases": ["Bosnien-Hercegovina", "Bosnien Herzegovina", "Bosnia and Herzegovina"],
          "away_aliases": ["Nordmakedonien", "North Macedonia"]
        },
        {
          "event_name": "Andorra - Irak",
          "url": "https://www.flashscore.dk/kamp/fodbold/andorra-dnO5z404/irak-K8aAGt6r/",
          "sport_key": "football",
          "home_aliases": ["Andorra"],
          "away_aliases": ["Irak", "Iraq"],
          "home_score": 0,
          "away_score": 1,
          "result_status": "finished",
          "result_observed_from": "user_supplied_flashscore_result",
          "notes": "Flashscore listed the neutral friendly as Irak - Andorra, with Irak winning 1-0. Stored scores are oriented to the Danske Spil event order Andorra - Irak."
        },
        {
          "event_name": "CD Maristas Palencia - Cb Fuenlabrada",
          "url": "https://www.flashscore.dk/kamp/basketball/fuenlabrada-E1z0hlIr/palencia-hMgAw6Je/?mid=4UaIOcR6",
          "sport_key": "basketball",
          "home_aliases": ["CD Maristas Palencia", "Palencia"],
          "away_aliases": ["Cb Fuenlabrada", "Fuenlabrada"],
          "home_score": 51,
          "away_score": 76,
          "result_status": "finished",
          "result_observed_from": "user_supplied_flashscore_result",
          "notes": "Flashscore lists the match as Fuenlabrada - Palencia 76:51. Stored scores are oriented to the Danske Spil event order CD Maristas Palencia - Cb Fuenlabrada."
        },
        {
          "event_name": "Nsa - Club Antonin Sportif",
          "url": "https://www.flashscore.com/match/basketball/antonine-xMbwy4Uk/nsa-xjRIpje7/",
          "sport_key": "basketball",
          "home_aliases": ["Nsa", "NSA"],
          "away_aliases": ["Club Antonin Sportif", "Antonine", "Antonin"],
          "home_score": 77,
          "away_score": 84,
          "result_status": "finished",
          "result_observed_from": "user_supplied_flashscore_result",
          "notes": "Flashscore page title reports NSA - Antonine 77:84. The URL has no mid query, so settlement uses this documented page-title result instead of a feed id."
        },
        {
          "event_name": "CR Vasco da Gama (W) - America MG (k)",
          "url": "https://www.flashscore.com/match/football/vasco-htf1ZG8n/america-mineiro-4xVVf8gB/?mid=jkrAljBU",
          "sport_key": "football",
          "gender_scope": "women",
          "home_aliases": ["CR Vasco da Gama (W)", "Vasco W", "Vasco da Gama W"],
          "away_aliases": ["America MG (k)", "America Mineiro W", "America MG"],
          "home_score": 1,
          "away_score": 0,
          "result_status": "finished",
          "result_observed_from": "flashscore_participant_feed",
          "notes": "Flashscore lists the Copa do Brasil women match as Vasco W - America Mineiro W 1:0. Stored scores are oriented to the Danske Spil event order."
        },
        {
          "event_name": "America De Cali SA (k) - Internacional de Palmira (W)",
          "url": "https://www.flashscore.com/match/football/america-de-cali-Q9RoAthL/inter-palmira-IqOwC2N8/?mid=6m1Fnb8U",
          "sport_key": "football",
          "gender_scope": "women",
          "home_aliases": ["America De Cali SA (k)", "America de Cali W", "America de Cali"],
          "away_aliases": ["Internacional de Palmira (W)", "Inter Palmira W", "Inter Palmira"],
          "home_score": 5,
          "away_score": 0,
          "result_status": "finished",
          "result_observed_from": "flashscore_participant_feed",
          "notes": "Flashscore lists the Liga Femenina match as America de Cali W - Inter Palmira W 5:0. Stored scores are oriented to the Danske Spil event order."
        }
      ]
    }'::jsonb
  ),
  (
    'sofascore_results',
    'Sofascore match result pages',
    'third_party_result',
    'https://www.sofascore.com/*',
    ARRAY['football','tennis','basketball'],
    0.82,
    true,
    true,
    'Fallback match-result source for manual paper-ledger review. Direct HTTP is blocked by Sofascore in local testing, so automated settlement requires browser automation evidence for this source.',
    '{
      "paper_only": true,
      "priority": 4,
      "fallback_only": true,
      "requires_browser_automation": true,
      "direct_http_blocked_observed_at": "2026-05-27",
      "known_matches": [
        {
          "event_name": "Andreea Diana Soare - Katarina Kujovic",
          "url": "https://www.sofascore.com/da/tennis/match/katarina-kujovic-andreea-diana-soare/FtiesyjFg",
          "home_aliases": ["Andreea Diana Soare", "Soare"],
          "away_aliases": ["Katarina Kujovic", "Kujovic"]
        },
        {
          "event_name": "Notts County - Salford City FC",
          "url": "https://www.sofascore.com/da/football/match/salford-city-notts-county/gbsYjp#id:16189253",
          "home_aliases": ["Notts County", "Notts Co"],
          "away_aliases": ["Salford City", "Salford City FC", "Salford"]
        },
        {
          "event_name": "Lyngby - Horsens",
          "url": "https://www.sofascore.com/da/football/match/lyngby-ac-horsens/XAsgK#id:15885987",
          "home_aliases": ["Lyngby", "Lyngby AC"],
          "away_aliases": ["Horsens", "AC Horsens"]
        },
        {
          "event_name": "CR Vasco da Gama (W) - America MG (k)",
          "url": "https://www.sofascore.com/da/football/match/vasco-da-gama-america-mineiro/WzocsKgAc",
          "sport_key": "football",
          "gender_scope": "women",
          "home_aliases": ["CR Vasco da Gama (W)", "Vasco da Gama", "Vasco da Gama W"],
          "away_aliases": ["America MG (k)", "America MG", "America Mineiro", "America Mineiro W"]
        }
      ]
    }'::jsonb
  ),
  (
    'xscores_results',
    'Xscores match result pages',
    'third_party_result',
    'https://www.xscores.com/*',
    ARRAY['football','tennis','basketball'],
    0.78,
    true,
    true,
    'Fallback match-result source when a stable Xscores match URL is available. Direct HTTP may be Cloudflare-gated, so known matches can carry a documented final score for paper-ledger settlement.',
    '{
      "paper_only": true,
      "priority": 5,
      "fallback_only": true,
      "known_matches": [
        {
          "event_name": "Brendan Loh - Marcus Schoeman",
          "url": "https://www.xscores.com/tennis/match/brendan-loh-vs-marcus-schoeman/26-05-2026/2783346",
          "sport_key": "tennis",
          "gender_scope": "men",
          "home_aliases": ["Brendan Loh", "Loh Brendan", "Loh"],
          "away_aliases": ["Marcus Schoeman", "Schoeman Marcus", "Schoeman"],
          "home_score": 0,
          "away_score": 2,
          "result_status": "finished",
          "result_observed_from": "public_search_index",
          "notes": "Xscores public result page was indexed with Brendan Loh 0 - 2 Marcus Schoeman and Finished status for 2026-05-26."
        }
      ]
    }'::jsonb
  ),
  (
    'livescore_results',
    'LiveScore match result pages',
    'third_party_result',
    'https://www.livescore.com/*',
    ARRAY['football','tennis','basketball'],
    0.80,
    true,
    true,
    'Fallback match-result source for manual paper-ledger review when a stable LiveScore match URL is available.',
    '{"paper_only": true, "priority": 6, "fallback_only": true}'::jsonb
  ),
  (
    'documented_third_party_results',
    'Documented third-party result source',
    'third_party_result',
    'Configured third-party result provider',
    ARRAY['football','tennis','basketball','motorsports','golf','cycling'],
    0.70,
    true,
    true,
    'Fallback only when source reliability and URL pattern are documented for the sport and event.',
    '{"paper_only": true, "priority": 7, "fallback_only": true}'::jsonb
  )
ON CONFLICT (source_key) DO UPDATE
SET source_name = EXCLUDED.source_name,
    source_type = EXCLUDED.source_type,
    url_pattern = EXCLUDED.url_pattern,
    sport_scope = EXCLUDED.sport_scope,
    reliability = EXCLUDED.reliability,
    can_settle = EXCLUDED.can_settle,
    manual_review_required = EXCLUDED.manual_review_required,
    notes = EXCLUDED.notes,
    payload = EXCLUDED.payload,
    last_seen_at = now();

CREATE TABLE IF NOT EXISTS ingestion_runs (
  id text PRIMARY KEY,
  source_key text REFERENCES source_registry(source_key),
  snapshot_id text REFERENCES odds_snapshots(id) ON DELETE SET NULL,
  started_at timestamptz NOT NULL DEFAULT now(),
  completed_at timestamptz NOT NULL DEFAULT now(),
  status text NOT NULL,
  sport_keys text[] NOT NULL DEFAULT '{}',
  event_count integer NOT NULL DEFAULT 0,
  payload jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE TABLE IF NOT EXISTS feature_snapshots (
  id text PRIMARY KEY,
  snapshot_id text NOT NULL REFERENCES odds_snapshots(id) ON DELETE CASCADE,
  event_id text NOT NULL REFERENCES sport_events(id) ON DELETE CASCADE,
  sport_key text NOT NULL REFERENCES sports(sport_key) ON DELETE CASCADE,
  feature_set text NOT NULL,
  source_key text REFERENCES source_registry(source_key),
  created_at timestamptz NOT NULL DEFAULT now(),
  confidence numeric NOT NULL,
  missing_signals jsonb NOT NULL DEFAULT '[]'::jsonb,
  features jsonb NOT NULL,
  UNIQUE (snapshot_id, event_id, feature_set)
);

CREATE TABLE IF NOT EXISTS candidate_bets (
  id text PRIMARY KEY,
  snapshot_id text REFERENCES odds_snapshots(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  sport_key text NOT NULL,
  event_id text,
  event_name text,
  competition text,
  market_id text,
  market_name text,
  market_kind text,
  outcome_id text,
  outcome_name text,
  decimal_odds numeric,
  rationale jsonb NOT NULL,
  implied_probability numeric,
  model_probability numeric,
  expected_value numeric,
  confidence numeric,
  score numeric,
  risk_flags jsonb NOT NULL DEFAULT '[]'::jsonb,
  feature_snapshot jsonb NOT NULL DEFAULT '{}'::jsonb,
  status text NOT NULL DEFAULT 'candidate'
);

ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS implied_probability numeric;
ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS model_probability numeric;
ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS expected_value numeric;
ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS confidence numeric;
ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS score numeric;
ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS risk_flags jsonb NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS feature_snapshot jsonb NOT NULL DEFAULT '{}'::jsonb;

CREATE TABLE IF NOT EXISTS candidate_coupons (
  id text PRIMARY KEY,
  snapshot_id text REFERENCES odds_snapshots(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  coupon_type text NOT NULL,
  leg_count integer NOT NULL,
  leg_signature text NOT NULL,
  combined_decimal_odds numeric,
  score numeric,
  confidence numeric,
  status text NOT NULL DEFAULT 'candidate',
  strategy_id text NOT NULL DEFAULT 'poc_ranker_v1',
  strategy_baseline_id text,
  strategy_version integer,
  provider_rule_evidence jsonb NOT NULL DEFAULT '{}'::jsonb,
  rationale jsonb NOT NULL DEFAULT '{}'::jsonb,
  UNIQUE (snapshot_id, coupon_type, leg_signature)
);

CREATE TABLE IF NOT EXISTS candidate_coupon_legs (
  id text PRIMARY KEY,
  coupon_id text NOT NULL REFERENCES candidate_coupons(id) ON DELETE CASCADE,
  candidate_id text NOT NULL REFERENCES candidate_bets(id) ON DELETE CASCADE,
  leg_index integer NOT NULL,
  observed_decimal_odds numeric,
  payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  UNIQUE (coupon_id, leg_index),
  UNIQUE (coupon_id, candidate_id)
);

CREATE TABLE IF NOT EXISTS simulated_bets (
  id text PRIMARY KEY,
  candidate_id text REFERENCES candidate_bets(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  hypothetical_stake numeric NOT NULL,
  observed_decimal_odds numeric,
  status text NOT NULL DEFAULT 'open',
  strategy_id text NOT NULL DEFAULT 'poc_ranker_v1',
  settled_at timestamptz,
  simulated_return numeric,
  profit_loss numeric,
  settlement_payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  payload jsonb NOT NULL
);

ALTER TABLE simulated_bets ADD COLUMN IF NOT EXISTS strategy_id text NOT NULL DEFAULT 'poc_ranker_v1';
ALTER TABLE simulated_bets ADD COLUMN IF NOT EXISTS settled_at timestamptz;
ALTER TABLE simulated_bets ADD COLUMN IF NOT EXISTS simulated_return numeric;
ALTER TABLE simulated_bets ADD COLUMN IF NOT EXISTS profit_loss numeric;
ALTER TABLE simulated_bets ADD COLUMN IF NOT EXISTS settlement_payload jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE simulated_bets ADD COLUMN IF NOT EXISTS event_start_time timestamptz;
ALTER TABLE simulated_bets ADD COLUMN IF NOT EXISTS expected_result_check_after timestamptz;

CREATE TABLE IF NOT EXISTS simulated_coupons (
  id text PRIMARY KEY,
  coupon_id text REFERENCES candidate_coupons(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  hypothetical_stake numeric NOT NULL,
  observed_combined_decimal_odds numeric,
  status text NOT NULL DEFAULT 'open',
  strategy_id text NOT NULL DEFAULT 'poc_ranker_v1',
  settled_at timestamptz,
  simulated_return numeric,
  profit_loss numeric,
  settlement_payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  payload jsonb NOT NULL
);

CREATE TABLE IF NOT EXISTS simulated_coupon_legs (
  id text PRIMARY KEY,
  simulated_coupon_id text NOT NULL REFERENCES simulated_coupons(id) ON DELETE CASCADE,
  candidate_id text REFERENCES candidate_bets(id),
  leg_index integer NOT NULL,
  observed_decimal_odds numeric,
  status text NOT NULL DEFAULT 'open',
  settlement_payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  UNIQUE (simulated_coupon_id, leg_index),
  UNIQUE (simulated_coupon_id, candidate_id)
);

ALTER TABLE simulated_coupons ADD COLUMN IF NOT EXISTS strategy_id text NOT NULL DEFAULT 'poc_ranker_v1';
ALTER TABLE simulated_coupons ADD COLUMN IF NOT EXISTS settled_at timestamptz;
ALTER TABLE simulated_coupons ADD COLUMN IF NOT EXISTS simulated_return numeric;
ALTER TABLE simulated_coupons ADD COLUMN IF NOT EXISTS profit_loss numeric;
ALTER TABLE simulated_coupons ADD COLUMN IF NOT EXISTS settlement_payload jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE simulated_coupons ADD COLUMN IF NOT EXISTS latest_event_start_time timestamptz;
ALTER TABLE simulated_coupons ADD COLUMN IF NOT EXISTS expected_result_check_after timestamptz;

ALTER TABLE simulated_coupon_legs ADD COLUMN IF NOT EXISTS event_start_time timestamptz;
ALTER TABLE simulated_coupon_legs ADD COLUMN IF NOT EXISTS expected_result_check_after timestamptz;

CREATE UNIQUE INDEX IF NOT EXISTS idx_simulated_coupons_one_normal_per_coupon
ON simulated_coupons(coupon_id)
WHERE coupon_id IS NOT NULL AND status <> 'duplicate_void';

WITH ranked AS (
  SELECT
    id,
    row_number() OVER (PARTITION BY candidate_id ORDER BY created_at ASC, id ASC) AS duplicate_rank
  FROM simulated_bets
  WHERE candidate_id IS NOT NULL
    AND status <> 'duplicate_void'
)
UPDATE simulated_bets sb
SET status = 'duplicate_void',
    settled_at = COALESCE(sb.settled_at, now()),
    simulated_return = COALESCE(sb.simulated_return, sb.hypothetical_stake),
    profit_loss = COALESCE(sb.profit_loss, 0),
    settlement_payload = sb.settlement_payload || jsonb_build_object(
      'duplicate_candidate_void',
      jsonb_build_object(
        'reason', 'duplicate paper placement for candidate_id',
        'deduped_at', now(),
        'paper_only', true
      )
    )
FROM ranked
WHERE sb.id = ranked.id
  AND ranked.duplicate_rank > 1;

CREATE UNIQUE INDEX IF NOT EXISTS idx_simulated_bets_one_normal_per_candidate
ON simulated_bets(candidate_id)
WHERE candidate_id IS NOT NULL AND status <> 'duplicate_void';

WITH logical AS (
  SELECT
    sb.id,
    row_number() OVER (
      PARTITION BY cb.event_id, cb.market_id, cb.outcome_id
      ORDER BY sb.created_at ASC, sb.id ASC
    ) AS duplicate_rank
  FROM simulated_bets sb
  JOIN candidate_bets cb ON cb.id = sb.candidate_id
  WHERE sb.status <> 'duplicate_void'
    AND cb.event_id IS NOT NULL
    AND cb.market_id IS NOT NULL
    AND cb.outcome_id IS NOT NULL
)
UPDATE simulated_bets sb
SET status = 'duplicate_void',
    settled_at = COALESCE(sb.settled_at, now()),
    simulated_return = COALESCE(sb.simulated_return, sb.hypothetical_stake),
    profit_loss = COALESCE(sb.profit_loss, 0),
    settlement_payload = sb.settlement_payload || jsonb_build_object(
      'duplicate_logical_selection_void',
      jsonb_build_object(
        'reason', 'duplicate paper placement for event/market/outcome',
        'deduped_at', now(),
        'paper_only', true
      )
    )
FROM logical
WHERE sb.id = logical.id
  AND logical.duplicate_rank > 1;

CREATE TABLE IF NOT EXISTS settlement_observations (
  id text PRIMARY KEY,
  simulated_bet_id text REFERENCES simulated_bets(id) ON DELETE CASCADE,
  simulated_coupon_id text REFERENCES simulated_coupons(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  source text NOT NULL,
  observed_result text NOT NULL,
  confidence numeric NOT NULL,
  payload jsonb NOT NULL
);

ALTER TABLE settlement_observations ADD COLUMN IF NOT EXISTS simulated_coupon_id text REFERENCES simulated_coupons(id) ON DELETE CASCADE;

CREATE TABLE IF NOT EXISTS settlement_lookup_attempts (
  id text PRIMARY KEY,
  created_at timestamptz NOT NULL DEFAULT now(),
  item_type text NOT NULL,
  simulated_bet_id text REFERENCES simulated_bets(id) ON DELETE CASCADE,
  simulated_coupon_id text REFERENCES simulated_coupons(id) ON DELETE CASCADE,
  source_key text NOT NULL,
  recommendation text NOT NULL,
  outcome_state jsonb NOT NULL DEFAULT '{}'::jsonb,
  payload jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS idx_settlement_lookup_attempts_created_at
ON settlement_lookup_attempts(created_at DESC);

CREATE TABLE IF NOT EXISTS external_result_evidence (
  id text PRIMARY KEY,
  created_at timestamptz NOT NULL DEFAULT now(),
  source_key text REFERENCES source_registry(source_key),
  source_url text,
  event_name text NOT NULL,
  home_name text NOT NULL,
  away_name text NOT NULL,
  home_score integer NOT NULL,
  away_score integer NOT NULL,
  confidence numeric NOT NULL,
  used_for_settlement boolean NOT NULL DEFAULT false,
  payload jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS idx_external_result_evidence_created_at
ON external_result_evidence(created_at DESC);

CREATE INDEX IF NOT EXISTS idx_external_result_evidence_event_name
ON external_result_evidence(event_name);

CREATE TABLE IF NOT EXISTS external_result_links (
  id text PRIMARY KEY,
  source_key text NOT NULL REFERENCES source_registry(source_key),
  event_name text NOT NULL,
  source_url text NOT NULL,
  home_aliases text[] NOT NULL DEFAULT '{}',
  away_aliases text[] NOT NULL DEFAULT '{}',
  requires_browser_automation boolean NOT NULL DEFAULT false,
  payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (source_key, event_name, source_url)
);

CREATE INDEX IF NOT EXISTS idx_external_result_links_event_name
ON external_result_links(event_name);

CREATE TABLE IF NOT EXISTS audit_events (
  id text PRIMARY KEY,
  created_at timestamptz NOT NULL DEFAULT now(),
  event_type text NOT NULL,
  details jsonb NOT NULL
);

CREATE TABLE IF NOT EXISTS simulation_performance_snapshots (
  id text PRIMARY KEY,
  created_at timestamptz NOT NULL DEFAULT now(),
  source text NOT NULL,
  odds_snapshot_id text REFERENCES odds_snapshots(id) ON DELETE SET NULL,
  ledger jsonb NOT NULL,
  played jsonb NOT NULL,
  performance jsonb NOT NULL,
  payload jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS idx_simulation_performance_snapshots_created_at
ON simulation_performance_snapshots(created_at DESC);

CREATE TABLE IF NOT EXISTS hermes_reflections (
  id text PRIMARY KEY,
  created_at timestamptz NOT NULL DEFAULT now(),
  title text NOT NULL,
  summary text NOT NULL,
  evidence jsonb NOT NULL,
  status text NOT NULL DEFAULT 'proposed'
);

CREATE TABLE IF NOT EXISTS strategy_baselines (
  id text PRIMARY KEY,
  created_at timestamptz NOT NULL DEFAULT now(),
  strategy_id text NOT NULL,
  version integer NOT NULL,
  status text NOT NULL,
  active boolean NOT NULL DEFAULT false,
  config jsonb NOT NULL,
  promoted_from_experiment_id text,
  notes text
);

ALTER TABLE strategy_baselines DROP CONSTRAINT IF EXISTS strategy_baselines_strategy_id_key;

CREATE TABLE IF NOT EXISTS strategy_experiments (
  id text PRIMARY KEY,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  title text NOT NULL,
  hypothesis text NOT NULL,
  variable_name text NOT NULL,
  baseline_value jsonb NOT NULL,
  proposed_value jsonb NOT NULL,
  baseline_strategy_id text NOT NULL,
  status text NOT NULL DEFAULT 'proposed',
  evidence jsonb NOT NULL,
  decision_payload jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE TABLE IF NOT EXISTS web_review_events (
  id text PRIMARY KEY,
  created_at timestamptz NOT NULL DEFAULT now(),
  subject_type text NOT NULL,
  subject_id text NOT NULL,
  action text NOT NULL,
  notes text,
  payload jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE TABLE IF NOT EXISTS strategy_candidate_decisions (
  id text PRIMARY KEY,
  created_at timestamptz NOT NULL DEFAULT now(),
  snapshot_id text REFERENCES odds_snapshots(id) ON DELETE CASCADE,
  candidate_id text REFERENCES candidate_bets(id) ON DELETE CASCADE,
  strategy_id text NOT NULL,
  strategy_baseline_id text NOT NULL,
  strategy_version integer NOT NULL,
  decision text NOT NULL,
  rejection_reasons jsonb NOT NULL DEFAULT '[]'::jsonb,
  score numeric,
  confidence numeric,
  evidence jsonb NOT NULL,
  UNIQUE (candidate_id, strategy_baseline_id)
);

INSERT INTO strategy_baselines (
  id, strategy_id, version, status, active, config, notes
)
VALUES (
  'poc_ranker_v1_baseline',
  'poc_ranker_v1',
  1,
  'active',
  true,
  '{
    "max_decimal_odds": 8.0,
    "min_confidence": 0.10,
    "excluded_market_kinds": [],
    "excluded_risk_flags": [],
    "allow_live_markets": false,
    "coupon_modes": {
      "single": true,
      "double": false,
      "triple": false,
      "accumulator": false,
      "max_legs": 1,
      "require_provider_accumulator_support": true,
      "require_same_sport_or_category_when_provider_requires_it": true
    },
    "paper_only": true,
    "one_variable_only": true
  }'::jsonb,
  'Initial transparent heuristic baseline. Real-money placement is disabled.'
)
ON CONFLICT (id) DO NOTHING;

UPDATE strategy_baselines
SET config = config || '{
  "coupon_modes": {
    "single": true,
    "double": false,
    "triple": false,
    "accumulator": false,
    "max_legs": 1,
    "require_provider_accumulator_support": true,
    "require_same_sport_or_category_when_provider_requires_it": true
  }
}'::jsonb
WHERE strategy_id = 'poc_ranker_v1'
  AND NOT (config ? 'coupon_modes');

UPDATE strategy_baselines
SET config = jsonb_set(config, '{excluded_risk_flags}', '[]'::jsonb, true)
WHERE strategy_id = 'poc_ranker_v1'
  AND NOT (config ? 'excluded_risk_flags');

INSERT INTO sports (sport_key, label, drilldown_id, sport_codes, payload)
VALUES (
  'motorsports',
  'Motorsports',
  '319',
  ARRAY['MOTOR_RACING','MOTORSPORT'],
  '{"renamed_from": "formula1", "paper_only": true}'::jsonb
)
ON CONFLICT (sport_key) DO UPDATE
SET label = EXCLUDED.label,
    drilldown_id = EXCLUDED.drilldown_id,
    sport_codes = EXCLUDED.sport_codes,
    payload = sports.payload || EXCLUDED.payload,
    last_seen_at = now();

UPDATE competitions SET sport_key = 'motorsports' WHERE sport_key = 'formula1';
UPDATE sport_events SET sport_key = 'motorsports' WHERE sport_key = 'formula1';
UPDATE coupon_rule_observations SET sport_key = 'motorsports' WHERE sport_key = 'formula1';
UPDATE feature_snapshots SET sport_key = 'motorsports' WHERE sport_key = 'formula1';
UPDATE candidate_bets SET sport_key = 'motorsports' WHERE sport_key = 'formula1';
UPDATE source_registry SET sport_scope = array_replace(sport_scope, 'formula1', 'motorsports');
UPDATE odds_snapshots SET sport_keys = array_replace(sport_keys, 'formula1', 'motorsports');
UPDATE ingestion_runs SET sport_keys = array_replace(sport_keys, 'formula1', 'motorsports');

CREATE INDEX IF NOT EXISTS idx_sport_events_sport_key ON sport_events(sport_key);
CREATE INDEX IF NOT EXISTS idx_sport_events_start_time ON sport_events(start_time);
CREATE INDEX IF NOT EXISTS idx_market_observations_snapshot ON market_observations(snapshot_id);
CREATE INDEX IF NOT EXISTS idx_market_observations_kind ON market_observations(market_kind);
CREATE INDEX IF NOT EXISTS idx_coupon_rule_observations_snapshot ON coupon_rule_observations(snapshot_id);
CREATE INDEX IF NOT EXISTS idx_coupon_rule_observations_sport ON coupon_rule_observations(sport_key);
CREATE INDEX IF NOT EXISTS idx_outcome_observations_snapshot ON outcome_observations(snapshot_id);
CREATE INDEX IF NOT EXISTS idx_feature_snapshots_snapshot ON feature_snapshots(snapshot_id);
CREATE INDEX IF NOT EXISTS idx_feature_snapshots_sport_key ON feature_snapshots(sport_key);
CREATE INDEX IF NOT EXISTS idx_strategy_experiments_status ON strategy_experiments(status);
CREATE UNIQUE INDEX IF NOT EXISTS idx_strategy_baselines_one_active ON strategy_baselines(strategy_id) WHERE active = true;
CREATE INDEX IF NOT EXISTS idx_strategy_candidate_decisions_snapshot ON strategy_candidate_decisions(snapshot_id);
CREATE INDEX IF NOT EXISTS idx_strategy_candidate_decisions_decision ON strategy_candidate_decisions(decision);
"#;

#[derive(Clone)]
pub struct Store {
    database_url: Option<String>,
}

impl Store {
    pub fn new(database_url: Option<String>) -> Self {
        Self { database_url }
    }

    pub async fn init_schema(&self) -> anyhow::Result<()> {
        let client = self.connect().await?;
        client
            .batch_execute(SCHEMA_SQL)
            .await
            .context("schema initialization failed")?;
        Ok(())
    }

    pub async fn status(&self) -> Value {
        if self.database_url.is_none() {
            return json!({"available": false, "connected": false, "last_error": "DATABASE_URL unavailable"});
        }
        match self.connect().await {
            Ok(client) => match client.query_one("SELECT 1 AS ok", &[]).await {
                Ok(row) => {
                    let ok: i32 = row.get("ok");
                    json!({"available": true, "connected": ok == 1, "last_error": null})
                }
                Err(error) => {
                    json!({"available": true, "connected": false, "last_error": error.to_string()})
                }
            },
            Err(error) => {
                json!({"available": true, "connected": false, "last_error": error.to_string()})
            }
        }
    }

    pub async fn latest_snapshot(&self) -> anyhow::Result<Option<Value>> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT id, observed_at, payload FROM odds_snapshots ORDER BY observed_at DESC LIMIT 1",
                &[],
            )
            .await?;
        Ok(row.map(|row| {
            let observed_at: DateTime<Utc> = row.get("observed_at");
            json!({
                "id": row.get::<_, String>("id"),
                "observed_at": observed_at,
                "payload": row.get::<_, Value>("payload")
            })
        }))
    }

    pub async fn save_snapshot(
        &self,
        payload: &Value,
        candidates: &mut [CandidateBet],
    ) -> anyhow::Result<String> {
        let snapshot_id = new_id();
        let observed_at = payload
            .get("observed_at")
            .and_then(Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        let sports = payload
            .get("sports")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let sport_keys: Vec<String> = sports
            .iter()
            .filter_map(|sport| {
                sport
                    .get("sport_key")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        let event_count: i32 = sports
            .iter()
            .map(|sport| {
                sport
                    .get("event_count")
                    .and_then(Value::as_i64)
                    .unwrap_or_default()
                    + sport
                        .get("outright_count")
                        .and_then(Value::as_i64)
                        .unwrap_or_default()
            })
            .sum::<i64>() as i32;
        for candidate in candidates.iter_mut() {
            candidate.snapshot_id = Some(snapshot_id.clone());
        }

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        transaction
            .execute(
                "INSERT INTO odds_snapshots (id, observed_at, source, mode, sport_keys, event_count, payload) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[
                    &snapshot_id,
                    &observed_at,
                    &payload.get("source").and_then(Value::as_str).unwrap_or("unknown"),
                    &payload.get("mode").and_then(Value::as_str).unwrap_or("unknown"),
                    &sport_keys,
                    &event_count,
                    payload,
                ],
            )
            .await?;
        save_source_registry(&transaction, &sport_keys).await?;
        save_market_catalog(&transaction, &snapshot_id, payload).await?;
        transaction
            .execute(
                r#"
                INSERT INTO ingestion_runs (
                  id, source_key, snapshot_id, status, sport_keys, event_count, payload
                )
                VALUES ($1,$2,$3,$4,$5,$6,$7)
                "#,
                &[
                    &new_id(),
                    &"danskespil_content_service",
                    &snapshot_id,
                    &"completed",
                    &sport_keys,
                    &event_count,
                    &json!({
                        "mode": payload.get("mode").cloned().unwrap_or(Value::Null),
                        "source": payload.get("source").cloned().unwrap_or(Value::Null),
                        "paper_only": true
                    }),
                ],
            )
            .await?;
        for candidate in candidates {
            if let Some(movement) =
                candidate_odds_movement(&transaction, &snapshot_id, observed_at, candidate).await?
            {
                attach_candidate_odds_movement(candidate, movement);
            }
            transaction
                .execute(
                    r#"
                    INSERT INTO candidate_bets (
                      id, snapshot_id, sport_key, event_id, event_name, competition,
                      market_id, market_name, market_kind, outcome_id, outcome_name,
                      decimal_odds, rationale, implied_probability, model_probability,
                      expected_value, confidence, score, risk_flags, feature_snapshot, status
                    )
                    VALUES (
                      $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,
                      ($12::float8)::numeric,
                      $13,
                      ($14::float8)::numeric,
                      ($15::float8)::numeric,
                      ($16::float8)::numeric,
                      ($17::float8)::numeric,
                      ($18::float8)::numeric,
                      $19,$20,$21
                    )
                    "#,
                    &[
                        &candidate.id,
                        &candidate.snapshot_id,
                        &candidate.sport_key,
                        &candidate.event_id,
                        &candidate.event_name,
                        &candidate.competition,
                        &candidate.market_id,
                        &candidate.market_name,
                        &candidate.market_kind,
                        &candidate.outcome_id,
                        &candidate.outcome_name,
                        &candidate.decimal_odds,
                        &candidate.rationale,
                        &candidate.implied_probability,
                        &candidate.model_probability,
                        &candidate.expected_value,
                        &candidate.confidence,
                        &candidate.score,
                        &candidate.risk_flags,
                        &candidate.feature_snapshot,
                        &candidate.status,
                    ],
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(snapshot_id)
    }

    pub async fn candidates(&self, limit: i64) -> anyhow::Result<Vec<CandidateBet>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT id, snapshot_id, created_at, sport_key, event_id, event_name, competition,
                       market_id, market_name, market_kind, outcome_id, outcome_name,
                       decimal_odds::float8 AS decimal_odds, rationale,
                       implied_probability::float8 AS implied_probability,
                       model_probability::float8 AS model_probability,
                       expected_value::float8 AS expected_value,
                       confidence::float8 AS confidence,
                       score::float8 AS score,
                       risk_flags, feature_snapshot, status
                FROM candidate_bets
                ORDER BY created_at DESC, score DESC NULLS LAST
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        Ok(rows.iter().map(candidate_from_row).collect())
    }

    pub async fn apply_active_strategy(
        &self,
        snapshot_id: &str,
        candidates: &[CandidateBet],
    ) -> anyhow::Result<Value> {
        let mut client = self.connect().await?;
        let baseline = client
            .query_opt(
                r#"
                SELECT id, strategy_id, version, config
                FROM strategy_baselines
                WHERE active = true
                ORDER BY created_at DESC
                LIMIT 1
                "#,
                &[],
            )
            .await?;
        let Some(baseline) = baseline else {
            return Ok(json!({
                "snapshot_id": snapshot_id,
                "selected_count": 0,
                "rejected_count": 0,
                "skipped": true,
                "reason": "no active strategy baseline"
            }));
        };

        let baseline_id: String = baseline.get("id");
        let strategy_id: String = baseline.get("strategy_id");
        let strategy_version: i32 = baseline.get("version");
        let config: Value = baseline.get("config");
        let max_decimal_odds = config
            .get("max_decimal_odds")
            .and_then(Value::as_f64)
            .unwrap_or(8.0);
        let min_confidence = config
            .get("min_confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.1);
        let allow_live_markets = config
            .get("allow_live_markets")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let excluded_market_kinds: HashSet<String> = config
            .get("excluded_market_kinds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        let excluded_risk_flags: HashSet<String> = config
            .get("excluded_risk_flags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();

        let transaction = client.transaction().await?;
        let mut selected_count = 0usize;
        let mut rejected_count = 0usize;
        let mut reason_counts = BTreeMap::<String, usize>::new();

        for candidate in candidates {
            let mut reasons = Vec::new();
            match candidate.decimal_odds {
                Some(odds) if odds > max_decimal_odds => reasons.push("above_max_decimal_odds"),
                Some(_) => {}
                None => reasons.push("missing_decimal_odds"),
            }
            if candidate.confidence.unwrap_or_default() < min_confidence {
                reasons.push("below_min_confidence");
            }
            if candidate
                .market_kind
                .as_ref()
                .is_some_and(|kind| excluded_market_kinds.contains(kind))
            {
                reasons.push("excluded_market_kind");
            }
            if candidate
                .risk_flags
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|flag| excluded_risk_flags.contains(flag))
            {
                reasons.push("excluded_risk_flag");
            }
            if !allow_live_markets
                && candidate
                    .feature_snapshot
                    .get("live_now")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            {
                reasons.push("live_market_disabled");
            }

            let decision = if reasons.is_empty() {
                selected_count += 1;
                "selected"
            } else {
                rejected_count += 1;
                for reason in &reasons {
                    *reason_counts.entry((*reason).to_string()).or_default() += 1;
                }
                "rejected"
            };
            let rejection_reasons = json!(reasons);
            let evidence = json!({
                "paper_only": true,
                "strategy_config": config,
                "candidate": {
                    "sport_key": candidate.sport_key,
                    "event_name": candidate.event_name,
                    "competition": candidate.competition,
                    "market_kind": candidate.market_kind,
                    "market_name": candidate.market_name,
                    "outcome_name": candidate.outcome_name,
                    "decimal_odds": candidate.decimal_odds,
                    "confidence": candidate.confidence,
                    "score": candidate.score,
                    "risk_flags": candidate.risk_flags
                },
                "feature_snapshot": candidate.feature_snapshot
            });
            transaction
                .execute(
                    r#"
                    INSERT INTO strategy_candidate_decisions (
                      id, snapshot_id, candidate_id, strategy_id, strategy_baseline_id,
                      strategy_version, decision, rejection_reasons, score, confidence, evidence
                    )
                    VALUES (
                      $1,$2,$3,$4,$5,$6,$7,$8,
                      ($9::float8)::numeric,
                      ($10::float8)::numeric,
                      $11
                    )
                    ON CONFLICT (candidate_id, strategy_baseline_id) DO UPDATE
                    SET decision = EXCLUDED.decision,
                        rejection_reasons = EXCLUDED.rejection_reasons,
                        score = EXCLUDED.score,
                        confidence = EXCLUDED.confidence,
                        evidence = EXCLUDED.evidence
                    "#,
                    &[
                        &new_id(),
                        &snapshot_id,
                        &candidate.id,
                        &strategy_id,
                        &baseline_id,
                        &strategy_version,
                        &decision,
                        &rejection_reasons,
                        &candidate.score,
                        &candidate.confidence,
                        &evidence,
                    ],
                )
                .await?;
            transaction
                .execute(
                    "UPDATE candidate_bets SET status = $1 WHERE id = $2",
                    &[&decision, &candidate.id],
                )
                .await?;
        }
        transaction.commit().await?;

        Ok(json!({
            "snapshot_id": snapshot_id,
            "strategy_id": strategy_id,
            "strategy_baseline_id": baseline_id,
            "strategy_version": strategy_version,
            "selected_count": selected_count,
            "rejected_count": rejected_count,
            "rejection_reason_counts": reason_counts,
            "paper_only": true
        }))
    }

    pub async fn strategy_decisions(&self, limit: i64) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT
                  d.id,
                  d.created_at,
                  d.snapshot_id,
                  d.candidate_id,
                  d.strategy_id,
                  d.strategy_baseline_id,
                  d.strategy_version,
                  d.decision,
                  d.rejection_reasons,
                  d.score::float8 AS score,
                  d.confidence::float8 AS confidence,
                  d.evidence,
                  cb.sport_key,
                  cb.event_name,
                  cb.competition,
                  cb.market_kind,
                  cb.market_name,
                  cb.outcome_name,
                  cb.decimal_odds::float8 AS decimal_odds
                FROM strategy_candidate_decisions d
                LEFT JOIN candidate_bets cb ON cb.id = d.candidate_id
                ORDER BY d.created_at DESC, d.score DESC NULLS LAST
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        Ok(json!({
            "items": rows.iter().map(|row| {
                let created_at: DateTime<Utc> = row.get("created_at");
                json!({
                    "id": row.get::<_, String>("id"),
                    "created_at": created_at,
                    "snapshot_id": row.get::<_, Option<String>>("snapshot_id"),
                    "candidate_id": row.get::<_, Option<String>>("candidate_id"),
                    "strategy_id": row.get::<_, String>("strategy_id"),
                    "strategy_baseline_id": row.get::<_, String>("strategy_baseline_id"),
                    "strategy_version": row.get::<_, i32>("strategy_version"),
                    "decision": row.get::<_, String>("decision"),
                    "rejection_reasons": row.get::<_, Value>("rejection_reasons"),
                    "score": row.get::<_, Option<f64>>("score"),
                    "confidence": row.get::<_, Option<f64>>("confidence"),
                    "evidence": row.get::<_, Value>("evidence"),
                    "candidate": {
                        "sport_key": row.get::<_, Option<String>>("sport_key"),
                        "event_name": row.get::<_, Option<String>>("event_name"),
                        "competition": row.get::<_, Option<String>>("competition"),
                        "market_kind": row.get::<_, Option<String>>("market_kind"),
                        "market_name": row.get::<_, Option<String>>("market_name"),
                        "outcome_name": row.get::<_, Option<String>>("outcome_name"),
                        "decimal_odds": row.get::<_, Option<f64>>("decimal_odds")
                    }
                })
            }).collect::<Vec<_>>()
        }))
    }

    pub async fn generate_candidate_coupons(
        &self,
        snapshot_id: &str,
        per_mode_limit: usize,
    ) -> anyhow::Result<Value> {
        if per_mode_limit == 0 {
            return Ok(json!({
                "enabled": true,
                "snapshot_id": snapshot_id,
                "generated_count": 0,
                "skipped": true,
                "reason": "per_mode_limit is zero"
            }));
        }

        let client = self.connect().await?;
        let Some(baseline) = client
            .query_opt(
                r#"
                SELECT id, strategy_id, version, config
                FROM strategy_baselines
                WHERE active = true
                ORDER BY created_at DESC
                LIMIT 1
                "#,
                &[],
            )
            .await?
        else {
            return Ok(json!({
                "enabled": false,
                "snapshot_id": snapshot_id,
                "generated_count": 0,
                "skipped": true,
                "reason": "no active strategy baseline"
            }));
        };
        let strategy_baseline_id: String = baseline.get("id");
        let strategy_id: String = baseline.get("strategy_id");
        let strategy_version: i32 = baseline.get("version");
        let config: Value = baseline.get("config");
        let coupon_modes = config.get("coupon_modes").unwrap_or(&Value::Null);
        let max_legs = coupon_modes
            .get("max_legs")
            .and_then(Value::as_u64)
            .unwrap_or(1) as usize;
        let modes = [
            ("double", 2usize),
            ("triple", 3usize),
            ("accumulator", 4usize),
        ];
        let enabled_modes: Vec<(&str, usize)> = modes
            .into_iter()
            .filter(|(mode, leg_count)| {
                coupon_modes
                    .get(*mode)
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    && *leg_count <= max_legs
            })
            .collect();
        if enabled_modes.is_empty() {
            return Ok(json!({
                "enabled": false,
                "snapshot_id": snapshot_id,
                "generated_count": 0,
                "skipped": true,
                "reason": "multi-leg coupon modes disabled by active baseline",
                "coupon_modes": coupon_modes
            }));
        }

        let rows = client
            .query(
                r#"
                SELECT cb.id, cb.snapshot_id, cb.created_at, cb.sport_key, cb.event_id, cb.event_name,
                       cb.competition, cb.market_id, cb.market_name, cb.market_kind, cb.outcome_id,
                       cb.outcome_name, cb.decimal_odds::float8 AS decimal_odds, cb.rationale,
                       cb.implied_probability::float8 AS implied_probability,
                       cb.model_probability::float8 AS model_probability,
                       cb.expected_value::float8 AS expected_value,
                       cb.confidence::float8 AS confidence,
                       cb.score::float8 AS score,
                       cb.risk_flags, cb.feature_snapshot, cb.status
                FROM strategy_candidate_decisions d
                JOIN candidate_bets cb ON cb.id = d.candidate_id
                WHERE d.snapshot_id = $1
                  AND d.decision = 'selected'
                  AND cb.status = 'selected'
                  AND cb.decimal_odds IS NOT NULL
                ORDER BY d.score DESC NULLS LAST, d.created_at ASC
                LIMIT 200
                "#,
                &[&snapshot_id],
            )
            .await?;
        let mut by_sport: BTreeMap<String, Vec<CandidateBet>> = BTreeMap::new();
        for row in rows {
            let candidate = candidate_from_row(&row);
            by_sport
                .entry(candidate.sport_key.clone())
                .or_default()
                .push(candidate);
        }

        let mut generated = Vec::new();
        let mut generated_count = 0usize;
        for (coupon_type, leg_count) in enabled_modes {
            let mut mode_count = 0usize;
            for (sport_key, candidates) in &by_sport {
                if mode_count >= per_mode_limit {
                    break;
                }
                let eligible: Vec<&CandidateBet> = candidates
                    .iter()
                    .filter(|candidate| accumulator_allowed(candidate, leg_count))
                    .collect();
                if eligible.len() < leg_count {
                    continue;
                }
                for combo in combinations(&eligible, leg_count) {
                    if mode_count >= per_mode_limit {
                        break;
                    }
                    if !distinct_events(&combo) {
                        continue;
                    }
                    let leg_signature = coupon_leg_signature(&combo);
                    let combined_decimal_odds = combo
                        .iter()
                        .filter_map(|candidate| candidate.decimal_odds)
                        .product::<f64>();
                    let score = combo
                        .iter()
                        .filter_map(|candidate| candidate.score)
                        .sum::<f64>()
                        / leg_count as f64;
                    let confidence = combo
                        .iter()
                        .filter_map(|candidate| candidate.confidence)
                        .fold(1.0_f64, f64::min);
                    let provider_rule_evidence = json!({
                        "source": "normalized_danskespil_market_metadata",
                        "sport_key": sport_key,
                        "coupon_type": coupon_type,
                        "leg_count": leg_count,
                        "same_sport_validation": true,
                        "distinct_event_validation": true,
                        "requires_provider_accumulator_support": true,
                        "legs": combo.iter().map(|candidate| json!({
                            "candidate_id": candidate.id,
                            "event_id": candidate.event_id,
                            "market_id": candidate.market_id,
                            "minimum_accumulator": candidate.feature_snapshot.get("minimum_accumulator").cloned().unwrap_or(Value::Null),
                            "maximum_accumulator": candidate.feature_snapshot.get("maximum_accumulator").cloned().unwrap_or(Value::Null)
                        })).collect::<Vec<_>>()
                    });
                    let rationale = json!({
                        "paper_only": true,
                        "selection_basis": "Provider-supported same-sport multi-leg coupon candidate from active strategy-selected single legs.",
                        "safety": "Real-money placement is disabled; coupon can only be reviewed or paper-ledgered after coupon simulation is implemented.",
                        "combined_decimal_odds": combined_decimal_odds,
                        "strategy_id": strategy_id,
                        "strategy_baseline_id": strategy_baseline_id,
                        "strategy_version": strategy_version
                    });
                    let coupon_id = new_id();
                    let affected = client
                        .execute(
                            r#"
                            INSERT INTO candidate_coupons (
                              id, snapshot_id, coupon_type, leg_count, leg_signature,
                              combined_decimal_odds, score, confidence, status,
                              strategy_id, strategy_baseline_id, strategy_version,
                              provider_rule_evidence, rationale
                            )
                            VALUES (
                              $1,$2,$3,$4,$5,
                              ($6::float8)::numeric,
                              ($7::float8)::numeric,
                              ($8::float8)::numeric,
                              $9,$10,$11,$12,$13,$14
                            )
                            ON CONFLICT (snapshot_id, coupon_type, leg_signature) DO NOTHING
                            "#,
                            &[
                                &coupon_id,
                                &snapshot_id,
                                &coupon_type,
                                &(leg_count as i32),
                                &leg_signature,
                                &combined_decimal_odds,
                                &score,
                                &confidence,
                                &"candidate",
                                &strategy_id,
                                &strategy_baseline_id,
                                &strategy_version,
                                &provider_rule_evidence,
                                &rationale,
                            ],
                        )
                        .await?;
                    let row = client
                        .query_one(
                            r#"
                            SELECT id
                            FROM candidate_coupons
                            WHERE snapshot_id = $1 AND coupon_type = $2 AND leg_signature = $3
                            "#,
                            &[&snapshot_id, &coupon_type, &leg_signature],
                        )
                        .await?;
                    let stored_coupon_id: String = row.get("id");
                    for (index, candidate) in combo.iter().enumerate() {
                        client
                            .execute(
                                r#"
                                INSERT INTO candidate_coupon_legs (
                                  id, coupon_id, candidate_id, leg_index, observed_decimal_odds, payload
                                )
                                VALUES ($1,$2,$3,$4,($5::float8)::numeric,$6)
                                ON CONFLICT (coupon_id, candidate_id) DO NOTHING
                                "#,
                                &[
                                    &new_id(),
                                    &stored_coupon_id,
                                    &candidate.id,
                                    &(index as i32),
                                    &candidate.decimal_odds,
                                    &json!({
                                        "candidate": candidate,
                                        "paper_only": true
                                    }),
                                ],
                            )
                            .await?;
                    }
                    if affected > 0 {
                        generated_count += 1;
                        mode_count += 1;
                    }
                    generated.push(json!({
                        "coupon_id": stored_coupon_id,
                        "coupon_type": coupon_type,
                        "leg_count": leg_count,
                        "combined_decimal_odds": combined_decimal_odds,
                        "score": score,
                        "confidence": confidence,
                        "inserted": affected > 0,
                        "sport_key": sport_key,
                        "leg_signature": leg_signature
                    }));
                }
            }
        }

        Ok(json!({
            "enabled": true,
            "snapshot_id": snapshot_id,
            "generated_count": generated_count,
            "returned_count": generated.len(),
            "per_mode_limit": per_mode_limit,
            "coupon_modes": coupon_modes,
            "items": generated,
            "paper_only": true
        }))
    }

    pub async fn candidate_coupons(&self, limit: i64) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT
                  cc.id, cc.snapshot_id, cc.created_at, cc.coupon_type, cc.leg_count,
                  cc.combined_decimal_odds::float8 AS combined_decimal_odds,
                  cc.score::float8 AS score,
                  cc.confidence::float8 AS confidence,
                  cc.status, cc.strategy_id, cc.strategy_baseline_id, cc.strategy_version,
                  cc.provider_rule_evidence, cc.rationale,
                  COALESCE(
                    jsonb_agg(
                      jsonb_build_object(
                        'candidate_id', ccl.candidate_id,
                        'leg_index', ccl.leg_index,
                        'observed_decimal_odds', ccl.observed_decimal_odds::float8,
                        'payload', ccl.payload
                      )
                      ORDER BY ccl.leg_index
                    ) FILTER (WHERE ccl.id IS NOT NULL),
                    '[]'::jsonb
                  ) AS legs
                FROM candidate_coupons cc
                LEFT JOIN candidate_coupon_legs ccl ON ccl.coupon_id = cc.id
                GROUP BY cc.id
                ORDER BY cc.created_at DESC, cc.score DESC NULLS LAST
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        Ok(json!({
            "items": rows.iter().map(|row| {
                let created_at: DateTime<Utc> = row.get("created_at");
                json!({
                    "id": row.get::<_, String>("id"),
                    "snapshot_id": row.get::<_, Option<String>>("snapshot_id"),
                    "created_at": created_at,
                    "coupon_type": row.get::<_, String>("coupon_type"),
                    "leg_count": row.get::<_, i32>("leg_count"),
                    "combined_decimal_odds": row.get::<_, Option<f64>>("combined_decimal_odds"),
                    "score": row.get::<_, Option<f64>>("score"),
                    "confidence": row.get::<_, Option<f64>>("confidence"),
                    "status": row.get::<_, String>("status"),
                    "strategy_id": row.get::<_, String>("strategy_id"),
                    "strategy_baseline_id": row.get::<_, Option<String>>("strategy_baseline_id"),
                    "strategy_version": row.get::<_, Option<i32>>("strategy_version"),
                    "provider_rule_evidence": row.get::<_, Value>("provider_rule_evidence"),
                    "rationale": row.get::<_, Value>("rationale"),
                    "legs": row.get::<_, Value>("legs")
                })
            }).collect::<Vec<_>>()
        }))
    }

    pub async fn simulated_coupons(&self, limit: i64) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT
                  sc.id, sc.coupon_id, sc.created_at,
                  sc.hypothetical_stake::float8 AS hypothetical_stake,
                  sc.observed_combined_decimal_odds::float8 AS observed_combined_decimal_odds,
                  sc.status, sc.strategy_id, sc.settled_at,
                  sc.latest_event_start_time,
                  sc.expected_result_check_after,
                  sc.simulated_return::float8 AS simulated_return,
                  sc.profit_loss::float8 AS profit_loss,
                  sc.settlement_payload, sc.payload,
                  COALESCE(
                    jsonb_agg(
                      jsonb_build_object(
                        'candidate_id', scl.candidate_id,
                        'leg_index', scl.leg_index,
                        'observed_decimal_odds', scl.observed_decimal_odds::float8,
                        'status', scl.status,
                        'event_start_time', scl.event_start_time,
                        'expected_result_check_after', scl.expected_result_check_after,
                        'settlement_payload', scl.settlement_payload,
                        'payload', scl.payload
                      )
                      ORDER BY scl.leg_index
                    ) FILTER (WHERE scl.id IS NOT NULL),
                    '[]'::jsonb
                  ) AS legs
                FROM simulated_coupons sc
                LEFT JOIN simulated_coupon_legs scl ON scl.simulated_coupon_id = sc.id
                GROUP BY sc.id
                ORDER BY sc.created_at DESC
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        Ok(json!({
            "paper_only": true,
            "items": rows.iter().map(|row| {
                let created_at: DateTime<Utc> = row.get("created_at");
                let settled_at: Option<DateTime<Utc>> = row.get("settled_at");
                let latest_event_start_time: Option<DateTime<Utc>> = row.get("latest_event_start_time");
                let expected_result_check_after: Option<DateTime<Utc>> = row.get("expected_result_check_after");
                json!({
                    "id": row.get::<_, String>("id"),
                    "coupon_id": row.get::<_, Option<String>>("coupon_id"),
                    "created_at": created_at,
                    "hypothetical_stake": row.get::<_, f64>("hypothetical_stake"),
                    "observed_combined_decimal_odds": row.get::<_, Option<f64>>("observed_combined_decimal_odds"),
                    "status": row.get::<_, String>("status"),
                    "strategy_id": row.get::<_, String>("strategy_id"),
                    "latest_event_start_time": latest_event_start_time,
                    "expected_result_check_after": expected_result_check_after,
                    "settled_at": settled_at,
                    "simulated_return": row.get::<_, Option<f64>>("simulated_return"),
                    "profit_loss": row.get::<_, Option<f64>>("profit_loss"),
                    "settlement_payload": row.get::<_, Value>("settlement_payload"),
                    "payload": row.get::<_, Value>("payload"),
                    "legs": row.get::<_, Value>("legs")
                })
            }).collect::<Vec<_>>()
        }))
    }

    pub async fn simulate_coupon(
        &self,
        coupon_id: &str,
        stake: f64,
        max_open_exposure: f64,
    ) -> anyhow::Result<Value> {
        if coupon_id.is_empty() {
            return Err(anyhow!("coupon_id is required"));
        }
        if stake <= 0.0 {
            return Err(anyhow!("stake must be positive"));
        }
        if let Some(existing) = self.simulated_coupon_by_coupon_id(coupon_id).await? {
            return Ok(existing);
        }
        let open_exposure = self.open_exposure().await?;
        if open_exposure + stake > max_open_exposure {
            return Err(anyhow!(
                "open exposure cap reached: current {open_exposure}, stake {stake}, max {max_open_exposure}"
            ));
        }

        let coupon = self
            .candidate_coupon_by_id(coupon_id)
            .await?
            .ok_or_else(|| anyhow!("candidate coupon not found: {coupon_id}"))?;
        let status = coupon
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if status == "rejected" {
            return Err(anyhow!(
                "rejected coupon cannot be paper-ledgered: {coupon_id}"
            ));
        }
        let legs = coupon
            .get("legs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if legs.len() < 2 {
            return Err(anyhow!("coupon must have at least two legs: {coupon_id}"));
        }
        let combined_decimal_odds = coupon.get("combined_decimal_odds").and_then(Value::as_f64);
        let latest_event_start_time = coupon_latest_event_start_time(&legs);
        let expected_result_check_after = coupon_expected_result_check_after(&legs);
        let strategy_id = coupon
            .get("strategy_id")
            .and_then(Value::as_str)
            .unwrap_or("poc_ranker_v1")
            .to_string();
        let simulated_coupon_id = new_id();
        let payload = json!({
            "coupon": coupon,
            "paper_only": true,
            "source": "operator_coupon_simulation",
            "safety": "Real-money placement is disabled; this only writes a simulated coupon ledger row."
        });

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let affected = transaction
            .execute(
                r#"
                INSERT INTO simulated_coupons (
                  id, coupon_id, hypothetical_stake, observed_combined_decimal_odds,
                  status, strategy_id, latest_event_start_time, expected_result_check_after,
                  settlement_payload, payload
                )
                VALUES ($1,$2,($3::float8)::numeric,($4::float8)::numeric,$5,$6,$7,$8,$9,$10)
                ON CONFLICT DO NOTHING
                "#,
                &[
                    &simulated_coupon_id,
                    &coupon_id,
                    &stake,
                    &combined_decimal_odds,
                    &"open",
                    &strategy_id,
                    &latest_event_start_time,
                    &expected_result_check_after,
                    &json!({}),
                    &payload,
                ],
            )
            .await?;
        if affected > 0 {
            for leg in legs {
                let candidate_id = leg
                    .get("candidate_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let leg_index = leg
                    .get("leg_index")
                    .and_then(Value::as_i64)
                    .unwrap_or_default() as i32;
                let observed_decimal_odds =
                    leg.get("observed_decimal_odds").and_then(Value::as_f64);
                let event_start_time = leg_event_start_time(&leg);
                let expected_result_check_after = leg_expected_result_check_after(&leg);
                transaction
                    .execute(
                        r#"
                        INSERT INTO simulated_coupon_legs (
                          id, simulated_coupon_id, candidate_id, leg_index,
                          observed_decimal_odds, status, event_start_time,
                          expected_result_check_after, settlement_payload, payload
                        )
                        VALUES ($1,$2,$3,$4,($5::float8)::numeric,$6,$7,$8,$9,$10)
                        ON CONFLICT DO NOTHING
                        "#,
                        &[
                            &new_id(),
                            &simulated_coupon_id,
                            &candidate_id,
                            &leg_index,
                            &observed_decimal_odds,
                            &"open",
                            &event_start_time,
                            &expected_result_check_after,
                            &json!({}),
                            &leg,
                        ],
                    )
                    .await?;
            }
            transaction
                .execute(
                    "UPDATE candidate_coupons SET status = 'paper_placed' WHERE id = $1 AND status = 'candidate'",
                    &[&coupon_id],
                )
                .await?;
        }
        transaction.commit().await?;
        self.simulated_coupon_by_coupon_id(coupon_id)
            .await?
            .ok_or_else(|| {
                anyhow!("simulated coupon insert skipped but existing row was not found")
            })
    }

    pub async fn paper_place_candidate_coupons(
        &self,
        snapshot_id: Option<&str>,
        stake: f64,
        per_scan_limit: usize,
        max_open_exposure: f64,
    ) -> anyhow::Result<Value> {
        if stake <= 0.0 {
            return Err(anyhow!("stake must be positive"));
        }
        if per_scan_limit == 0 {
            return Ok(json!({
                "enabled": true,
                "placed_count": 0,
                "skipped": true,
                "reason": "per_scan_limit is zero"
            }));
        }

        let client = self.connect().await?;
        let snapshot_id = match snapshot_id.filter(|value| !value.is_empty()) {
            Some(value) => value.to_string(),
            None => client
                .query_opt(
                    "SELECT id FROM odds_snapshots ORDER BY observed_at DESC LIMIT 1",
                    &[],
                )
                .await?
                .map(|row| row.get::<_, String>("id"))
                .ok_or_else(|| anyhow!("no snapshot available for paper coupon placement"))?,
        };

        let open_exposure = self.open_exposure().await?;
        let remaining_exposure = (max_open_exposure - open_exposure).max(0.0);
        let exposure_limited_slots = (remaining_exposure / stake).floor() as usize;
        let place_limit = per_scan_limit.min(exposure_limited_slots);
        if place_limit == 0 {
            return Ok(json!({
                "enabled": true,
                "snapshot_id": snapshot_id,
                "placed_count": 0,
                "open_exposure": open_exposure,
                "max_open_exposure": max_open_exposure,
                "skipped": true,
                "reason": "open exposure cap reached"
            }));
        }

        let candidate_search_limit = (place_limit.saturating_mul(25)).clamp(place_limit, 200);
        let rows = client
            .query(
                r#"
                SELECT cc.id
                FROM candidate_coupons cc
                WHERE cc.snapshot_id = $1
                  AND cc.status = 'candidate'
                  AND NOT EXISTS (
                    SELECT 1
                    FROM simulated_coupons sc
                    WHERE sc.coupon_id = cc.id
                      AND sc.status <> 'duplicate_void'
                  )
                ORDER BY cc.score DESC NULLS LAST, cc.confidence DESC NULLS LAST, cc.created_at ASC
                LIMIT $2
                "#,
                &[&snapshot_id, &(candidate_search_limit as i64)],
            )
            .await?;

        let considered_count = rows.len();
        let mut placed = Vec::new();
        let mut skipped = Vec::new();
        for row in rows {
            if placed.len() >= place_limit {
                break;
            }
            let coupon_id: String = row.get("id");
            match self
                .simulate_coupon(&coupon_id, stake, max_open_exposure)
                .await
            {
                Ok(item) => {
                    let coupon = item
                        .get("payload")
                        .and_then(|payload| payload.get("coupon"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    placed.push(json!({
                        "simulated_coupon_id": item.get("id").cloned().unwrap_or(Value::Null),
                        "coupon_id": coupon_id,
                        "coupon_type": coupon.get("coupon_type").cloned().unwrap_or(Value::Null),
                        "leg_count": coupon.get("leg_count").cloned().unwrap_or(Value::Null),
                        "observed_combined_decimal_odds": item.get("observed_combined_decimal_odds").cloned().unwrap_or(Value::Null),
                        "hypothetical_stake": item.get("hypothetical_stake").cloned().unwrap_or(Value::Null)
                    }));
                }
                Err(error) => {
                    let reason = error.to_string();
                    skipped.push(json!({
                        "coupon_id": coupon_id,
                        "reason": reason
                    }));
                    if skipped
                        .last()
                        .and_then(|item| item.get("reason"))
                        .and_then(Value::as_str)
                        .is_some_and(|value| value.contains("open exposure cap reached"))
                    {
                        break;
                    }
                }
            }
        }

        Ok(json!({
            "enabled": true,
            "snapshot_id": snapshot_id,
            "stake": stake,
            "per_scan_limit": per_scan_limit,
            "max_open_exposure": max_open_exposure,
            "open_exposure_before": open_exposure,
            "placed_count": placed.len(),
            "considered_count": considered_count,
            "skipped_count": considered_count.saturating_sub(placed.len()),
            "placed": placed,
            "skipped": skipped,
            "paper_only": true
        }))
    }

    async fn candidate_coupon_by_id(&self, coupon_id: &str) -> anyhow::Result<Option<Value>> {
        Ok(self
            .candidate_coupons(1000)
            .await?
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|item| item.get("id").and_then(Value::as_str) == Some(coupon_id))
            .cloned())
    }

    async fn simulated_coupon_by_coupon_id(
        &self,
        coupon_id: &str,
    ) -> anyhow::Result<Option<Value>> {
        Ok(self
            .simulated_coupons(1000)
            .await?
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|item| item.get("coupon_id").and_then(Value::as_str) == Some(coupon_id))
            .cloned())
    }

    pub async fn simulate_bet(
        &self,
        candidate_id: &str,
        stake: f64,
    ) -> anyhow::Result<SimulatedBet> {
        if stake <= 0.0 {
            return Err(anyhow!("stake must be positive"));
        }
        let candidate = self
            .candidates(200)
            .await?
            .into_iter()
            .find(|candidate| candidate.id == candidate_id)
            .ok_or_else(|| anyhow!("candidate not found: {candidate_id}"))?;
        if candidate.status == "rejected" {
            return Err(anyhow!(
                "candidate rejected by active strategy and cannot be paper-ledgered: {candidate_id}"
            ));
        }
        if let Some(existing) = self.simulated_bet_by_candidate(candidate_id).await? {
            return Ok(existing);
        }
        if let Some(existing) = self.simulated_bet_by_logical_selection(&candidate).await? {
            return Ok(existing);
        }
        let id = new_id();
        let event_start_time = candidate_event_start_time(&candidate);
        let expected_result_check_after =
            expected_result_check_after_for_sport(&candidate.sport_key, event_start_time);
        let payload =
            json!({"candidate": candidate, "paper_only": true, "strategy_id": "poc_ranker_v1"});
        let client = self.connect().await?;
        let affected = client
            .execute(
                r#"
                INSERT INTO simulated_bets (
                  id, candidate_id, hypothetical_stake, observed_decimal_odds, status,
                  strategy_id, event_start_time, expected_result_check_after, settlement_payload,
                  payload
                )
                VALUES ($1,$2,($3::float8)::numeric,($4::float8)::numeric,$5,$6,$7,$8,$9,$10)
                ON CONFLICT DO NOTHING
                "#,
                &[
                    &id,
                    &candidate_id,
                    &stake,
                    &candidate.decimal_odds,
                    &"open",
                    &"poc_ranker_v1",
                    &event_start_time,
                    &expected_result_check_after,
                    &json!({}),
                    &payload,
                ],
            )
            .await?;
        if affected == 0 {
            return self
                .simulated_bet_by_candidate(candidate_id)
                .await?
                .or(self.simulated_bet_by_logical_selection(&candidate).await?)
                .ok_or_else(|| {
                    anyhow!("simulated bet insert skipped but existing row was not found")
                });
        }
        self.simulated_bets(1)
            .await?
            .into_iter()
            .find(|bet| bet.id == id)
            .ok_or_else(|| anyhow!("inserted simulated bet not found"))
    }

    pub async fn paper_place_selected(
        &self,
        snapshot_id: Option<&str>,
        stake: f64,
        per_scan_limit: usize,
        max_open_exposure: f64,
    ) -> anyhow::Result<Value> {
        if stake <= 0.0 {
            return Err(anyhow!("stake must be positive"));
        }
        if per_scan_limit == 0 {
            return Ok(json!({
                "enabled": true,
                "placed_count": 0,
                "skipped": true,
                "reason": "per_scan_limit is zero"
            }));
        }

        let client = self.connect().await?;
        let snapshot_id = match snapshot_id.filter(|value| !value.is_empty()) {
            Some(value) => value.to_string(),
            None => client
                .query_opt(
                    "SELECT id FROM odds_snapshots ORDER BY observed_at DESC LIMIT 1",
                    &[],
                )
                .await?
                .map(|row| row.get::<_, String>("id"))
                .ok_or_else(|| anyhow!("no snapshot available for paper placement"))?,
        };
        let open_exposure = self.open_exposure().await?;
        let remaining_exposure = (max_open_exposure - open_exposure).max(0.0);
        let exposure_limited_slots = (remaining_exposure / stake).floor() as usize;
        let place_limit = per_scan_limit.min(exposure_limited_slots);
        if place_limit == 0 {
            return Ok(json!({
                "enabled": true,
                "snapshot_id": snapshot_id,
                "placed_count": 0,
                "open_exposure": open_exposure,
                "max_open_exposure": max_open_exposure,
                "skipped": true,
                "reason": "open exposure cap reached"
            }));
        }
        let candidate_search_limit = (place_limit.saturating_mul(25)).clamp(place_limit, 200);

        let rows = client
            .query(
                r#"
                SELECT
                  cb.id, cb.snapshot_id, cb.created_at, cb.sport_key, cb.event_id, cb.event_name,
                  cb.competition, cb.market_id, cb.market_name, cb.market_kind, cb.outcome_id,
                  cb.outcome_name, cb.decimal_odds::float8 AS decimal_odds, cb.rationale,
                  cb.implied_probability::float8 AS implied_probability,
                  cb.model_probability::float8 AS model_probability,
                  cb.expected_value::float8 AS expected_value,
                  cb.confidence::float8 AS confidence,
                  cb.score::float8 AS score,
                  cb.risk_flags, cb.feature_snapshot, cb.status,
                  d.id AS decision_id,
                  d.strategy_id,
                  d.strategy_baseline_id,
                  d.strategy_version,
                  d.evidence AS decision_evidence
                FROM strategy_candidate_decisions d
                JOIN candidate_bets cb ON cb.id = d.candidate_id
                WHERE d.snapshot_id = $1
                  AND d.decision = 'selected'
                  AND cb.status = 'selected'
                  AND NOT EXISTS (
                    SELECT 1 FROM simulated_bets sb
                    WHERE sb.candidate_id = cb.id
                      AND sb.status <> 'duplicate_void'
                  )
                ORDER BY d.score DESC NULLS LAST, d.created_at ASC
                LIMIT $2
                "#,
                &[&snapshot_id, &(candidate_search_limit as i64)],
            )
            .await?;

        let considered_count = rows.len();
        let mut placed = Vec::new();
        for row in rows {
            if placed.len() >= place_limit {
                break;
            }
            let candidate = candidate_from_row(&row);
            let decision_id: String = row.get("decision_id");
            let strategy_id: String = row.get("strategy_id");
            let strategy_baseline_id: String = row.get("strategy_baseline_id");
            let strategy_version: i32 = row.get("strategy_version");
            let decision_evidence: Value = row.get("decision_evidence");
            let event_start_time = candidate_event_start_time(&candidate);
            let expected_result_check_after =
                expected_result_check_after_for_sport(&candidate.sport_key, event_start_time);
            let id = new_id();
            let payload = json!({
                "candidate": candidate,
                "paper_only": true,
                "auto_paper": true,
                "decision_id": decision_id,
                "strategy_id": strategy_id,
                "strategy_baseline_id": strategy_baseline_id,
                "strategy_version": strategy_version,
                "decision_evidence": decision_evidence
            });
            let inserted_count = client
                .execute(
                    r#"
                INSERT INTO simulated_bets (
                  id, candidate_id, hypothetical_stake, observed_decimal_odds, status,
                  strategy_id, event_start_time, expected_result_check_after,
                  settlement_payload, payload
                )
                SELECT $1,$2,($3::float8)::numeric,($4::float8)::numeric,$5,$6,$7,$8,$9,$10
                WHERE NOT EXISTS (
                  SELECT 1 FROM simulated_bets WHERE candidate_id = $2 AND status <> 'duplicate_void'
                )
                AND NOT EXISTS (
                  SELECT 1
                  FROM simulated_bets sb
                  JOIN candidate_bets existing ON existing.id = sb.candidate_id
                  WHERE existing.event_id = $12
                    AND existing.market_id = $13
                    AND existing.outcome_id = $14
                    AND sb.status <> 'duplicate_void'
                )
                AND (
                  SELECT (
                    SELECT COALESCE(sum(hypothetical_stake), 0)
                    FROM simulated_bets
                    WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed')
                  ) + (
                    SELECT COALESCE(sum(hypothetical_stake), 0)
                    FROM simulated_coupons
                    WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed')
                  )
                ) + ($3::float8)::numeric <= ($11::float8)::numeric
                ON CONFLICT DO NOTHING
                "#,
                &[
                        &id,
                        &candidate.id,
                        &stake,
                        &candidate.decimal_odds,
                        &"open",
                    &strategy_id,
                    &event_start_time,
                    &expected_result_check_after,
                    &json!({}),
                    &payload,
                    &max_open_exposure,
                    &candidate.event_id,
                    &candidate.market_id,
                    &candidate.outcome_id,
                ],
                )
                .await?;
            if inserted_count > 0 {
                if let Some(item) = self.simulated_bet_by_candidate(&candidate.id).await? {
                    placed.push(json!({
                        "bet_id": item.id,
                        "candidate_id": item.candidate_id,
                        "event_name": candidate.event_name,
                        "market_name": candidate.market_name,
                        "outcome_name": candidate.outcome_name,
                        "observed_decimal_odds": item.observed_decimal_odds,
                        "hypothetical_stake": item.hypothetical_stake
                    }));
                }
            }
        }

        Ok(json!({
            "enabled": true,
            "snapshot_id": snapshot_id,
            "stake": stake,
            "per_scan_limit": per_scan_limit,
            "max_open_exposure": max_open_exposure,
            "open_exposure_before": open_exposure,
            "placed_count": placed.len(),
            "considered_count": considered_count,
            "skipped_count": considered_count.saturating_sub(placed.len()),
            "placed": placed,
            "paper_only": true
        }))
    }

    async fn simulated_bet_by_candidate(
        &self,
        candidate_id: &str,
    ) -> anyhow::Result<Option<SimulatedBet>> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                r#"
                SELECT sb.id, sb.candidate_id, sb.created_at,
                       cb.sport_key, cb.event_name, cb.competition, cb.market_name,
                       cb.market_kind, cb.outcome_name,
                       sb.hypothetical_stake::float8 AS hypothetical_stake,
                       sb.observed_decimal_odds::float8 AS observed_decimal_odds, sb.status,
                       sb.strategy_id, sb.event_start_time, sb.expected_result_check_after, sb.settled_at,
                       sb.simulated_return::float8 AS simulated_return,
                       sb.profit_loss::float8 AS profit_loss,
                       sb.settlement_payload, sb.payload
                FROM simulated_bets sb
                LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                WHERE sb.candidate_id = $1
                ORDER BY sb.created_at ASC
                LIMIT 1
                "#,
                &[&candidate_id],
            )
            .await?;
        Ok(row.map(|row| simulated_bet_from_row(&row)))
    }

    async fn simulated_bet_by_logical_selection(
        &self,
        candidate: &CandidateBet,
    ) -> anyhow::Result<Option<SimulatedBet>> {
        let (Some(event_id), Some(market_id), Some(outcome_id)) = (
            candidate.event_id.as_deref(),
            candidate.market_id.as_deref(),
            candidate.outcome_id.as_deref(),
        ) else {
            return Ok(None);
        };
        let client = self.connect().await?;
        let row = client
            .query_opt(
                r#"
                SELECT sb.id, sb.candidate_id, sb.created_at,
                       cb.sport_key, cb.event_name, cb.competition, cb.market_name,
                       cb.market_kind, cb.outcome_name,
                       sb.hypothetical_stake::float8 AS hypothetical_stake,
                       sb.observed_decimal_odds::float8 AS observed_decimal_odds,
                       sb.status, sb.strategy_id, sb.event_start_time,
                       sb.expected_result_check_after, sb.settled_at,
                       sb.simulated_return::float8 AS simulated_return,
                       sb.profit_loss::float8 AS profit_loss,
                       sb.settlement_payload, sb.payload
                FROM simulated_bets sb
                JOIN candidate_bets cb ON cb.id = sb.candidate_id
                WHERE cb.event_id = $1
                  AND cb.market_id = $2
                  AND cb.outcome_id = $3
                  AND sb.status <> 'duplicate_void'
                ORDER BY sb.created_at ASC
                LIMIT 1
                "#,
                &[&event_id, &market_id, &outcome_id],
            )
            .await?;
        Ok(row.map(|row| simulated_bet_from_row(&row)))
    }

    async fn open_exposure(&self) -> anyhow::Result<f64> {
        let client = self.connect().await?;
        let row = client
            .query_one(
                r#"
                SELECT (
                  SELECT COALESCE(sum(hypothetical_stake), 0)
                  FROM simulated_bets
                  WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed')
                )::float8
                + (
                  SELECT COALESCE(sum(hypothetical_stake), 0)
                  FROM simulated_coupons
                  WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed')
                )::float8 AS open_exposure
                "#,
                &[],
            )
            .await?;
        Ok(row.get("open_exposure"))
    }

    pub async fn advance_settlement_queue(
        &self,
        awaiting_grace_minutes: i64,
        limit: usize,
    ) -> anyhow::Result<Value> {
        if limit == 0 {
            return Ok(json!({
                "enabled": true,
                "transitioned_count": 0,
                "skipped": true,
                "reason": "settlement queue limit is zero"
            }));
        }

        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                WITH eligible AS (
                  SELECT
                    sb.id,
                    cb.event_id,
                    cb.event_name,
                    cb.market_name,
                    cb.outcome_name,
                    cb.sport_key,
                    cb.competition,
                    COALESCE(
                      sb.event_start_time,
                      se.start_time,
                      CASE
                        WHEN cb.feature_snapshot ? 'start_time'
                         AND cb.feature_snapshot->>'start_time' ~ '^[0-9]{4}-'
                        THEN (cb.feature_snapshot->>'start_time')::timestamptz
                        ELSE NULL
                      END
                    ) AS event_start_time,
                    COALESCE(
                      sb.expected_result_check_after,
                      CASE
                        WHEN COALESCE(
                          sb.event_start_time,
                          se.start_time,
                          CASE
                            WHEN cb.feature_snapshot ? 'start_time'
                             AND cb.feature_snapshot->>'start_time' ~ '^[0-9]{4}-'
                            THEN (cb.feature_snapshot->>'start_time')::timestamptz
                            ELSE NULL
                          END
                        ) IS NULL THEN NULL
                        WHEN cb.sport_key = 'football' THEN COALESCE(sb.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '130 minutes'
                        WHEN cb.sport_key = 'basketball' THEN COALESCE(sb.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '150 minutes'
                        WHEN cb.sport_key = 'tennis' THEN COALESCE(sb.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '240 minutes'
                        WHEN cb.sport_key IN ('motorsports', 'golf', 'cycling') THEN COALESCE(sb.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '1 day'
                        ELSE COALESCE(sb.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '4 hours'
                      END
                    ) AS expected_result_check_after,
                    se.status AS event_status,
                    se.resulted AS event_resulted,
                    se.settled AS event_settled
                  FROM simulated_bets sb
                  JOIN candidate_bets cb ON cb.id = sb.candidate_id
                  LEFT JOIN sport_events se ON se.id = cb.event_id
                  WHERE sb.status = 'open'
                    AND COALESCE(
                      sb.event_start_time,
                      se.start_time,
                      CASE
                        WHEN cb.feature_snapshot ? 'start_time'
                         AND cb.feature_snapshot->>'start_time' ~ '^[0-9]{4}-'
                        THEN (cb.feature_snapshot->>'start_time')::timestamptz
                        ELSE NULL
                      END
                    ) <= now() - ($1::int * interval '1 minute')
                  ORDER BY event_start_time ASC NULLS LAST, sb.created_at ASC
                  LIMIT $2
                )
                UPDATE simulated_bets sb
                SET status = 'awaiting_result',
                    event_start_time = COALESCE(sb.event_start_time, eligible.event_start_time),
                    expected_result_check_after = COALESCE(
                      sb.expected_result_check_after,
                      eligible.expected_result_check_after
                    ),
                    settlement_payload = sb.settlement_payload || jsonb_build_object(
                      'queue_transition',
                      jsonb_build_object(
                        'from_status', 'open',
                        'to_status', 'awaiting_result',
                        'transitioned_at', now(),
                        'source', 'settlement_queue',
                        'event_start_time', eligible.event_start_time,
                        'expected_result_check_after', eligible.expected_result_check_after,
                        'event_status', eligible.event_status,
                        'event_resulted', eligible.event_resulted,
                        'event_settled', eligible.event_settled,
                        'paper_only', true
                      )
                    )
                FROM eligible
                WHERE sb.id = eligible.id
                RETURNING
                  sb.id,
                  eligible.event_id,
                  eligible.event_name,
                  eligible.market_name,
                  eligible.outcome_name,
                  eligible.sport_key,
                  eligible.competition,
                  eligible.event_start_time,
                  eligible.expected_result_check_after,
                  eligible.event_status,
                  eligible.event_resulted,
                  eligible.event_settled
                "#,
                &[&(awaiting_grace_minutes as i32), &(limit as i64)],
            )
            .await?;
        let coupon_rows = client
            .query(
                r#"
                WITH eligible AS (
                  SELECT
                    sc.id,
                    sc.coupon_id,
                    cc.coupon_type,
                    cc.leg_count,
                    cc.combined_decimal_odds::float8 AS combined_decimal_odds,
                    cc.strategy_id,
                    max(
                      COALESCE(
                        scl.event_start_time,
                        se.start_time,
                        CASE
                          WHEN cb.feature_snapshot ? 'start_time'
                           AND cb.feature_snapshot->>'start_time' ~ '^[0-9]{4}-'
                          THEN (cb.feature_snapshot->>'start_time')::timestamptz
                          ELSE NULL
                        END
                      )
                    ) AS event_start_time,
                    max(
                      COALESCE(
                        scl.expected_result_check_after,
                        CASE
                          WHEN COALESCE(
                            scl.event_start_time,
                            se.start_time,
                            CASE
                              WHEN cb.feature_snapshot ? 'start_time'
                               AND cb.feature_snapshot->>'start_time' ~ '^[0-9]{4}-'
                              THEN (cb.feature_snapshot->>'start_time')::timestamptz
                              ELSE NULL
                            END
                          ) IS NULL THEN NULL
                          WHEN cb.sport_key = 'football' THEN COALESCE(scl.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '130 minutes'
                          WHEN cb.sport_key = 'basketball' THEN COALESCE(scl.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '150 minutes'
                          WHEN cb.sport_key = 'tennis' THEN COALESCE(scl.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '240 minutes'
                          WHEN cb.sport_key IN ('motorsports', 'golf', 'cycling') THEN COALESCE(scl.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '1 day'
                          ELSE COALESCE(scl.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '4 hours'
                        END
                      )
                    ) AS expected_result_check_after,
                    jsonb_agg(
                      jsonb_build_object(
                        'candidate_id', cb.id,
                        'leg_index', scl.leg_index,
                        'event_id', cb.event_id,
                        'event_name', cb.event_name,
                        'sport_key', cb.sport_key,
                        'competition', cb.competition,
                        'market_name', cb.market_name,
                        'outcome_name', cb.outcome_name,
                        'event_start_time', COALESCE(
                          scl.event_start_time,
                          se.start_time,
                          CASE
                            WHEN cb.feature_snapshot ? 'start_time'
                             AND cb.feature_snapshot->>'start_time' ~ '^[0-9]{4}-'
                            THEN (cb.feature_snapshot->>'start_time')::timestamptz
                            ELSE NULL
                          END
                        ),
                        'expected_result_check_after', COALESCE(
                          scl.expected_result_check_after,
                          CASE
                            WHEN COALESCE(
                              scl.event_start_time,
                              se.start_time,
                              CASE
                                WHEN cb.feature_snapshot ? 'start_time'
                                 AND cb.feature_snapshot->>'start_time' ~ '^[0-9]{4}-'
                                THEN (cb.feature_snapshot->>'start_time')::timestamptz
                                ELSE NULL
                              END
                            ) IS NULL THEN NULL
                            WHEN cb.sport_key = 'football' THEN COALESCE(scl.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '130 minutes'
                            WHEN cb.sport_key = 'basketball' THEN COALESCE(scl.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '150 minutes'
                            WHEN cb.sport_key = 'tennis' THEN COALESCE(scl.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '240 minutes'
                            WHEN cb.sport_key IN ('motorsports', 'golf', 'cycling') THEN COALESCE(scl.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '1 day'
                            ELSE COALESCE(scl.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '4 hours'
                          END
                        ),
                        'event_status', se.status,
                        'event_resulted', se.resulted,
                        'event_settled', se.settled
                      )
                      ORDER BY scl.leg_index
                    ) AS legs
                  FROM simulated_coupons sc
                  JOIN candidate_coupons cc ON cc.id = sc.coupon_id
                  JOIN simulated_coupon_legs scl ON scl.simulated_coupon_id = sc.id
                  JOIN candidate_bets cb ON cb.id = scl.candidate_id
                  LEFT JOIN sport_events se ON se.id = cb.event_id
                  WHERE sc.status = 'open'
                  GROUP BY sc.id, sc.coupon_id, cc.coupon_type, cc.leg_count,
                           cc.combined_decimal_odds, cc.strategy_id, sc.created_at
                  HAVING max(
                    COALESCE(
                      se.start_time,
                      CASE
                        WHEN cb.feature_snapshot ? 'start_time'
                         AND cb.feature_snapshot->>'start_time' ~ '^[0-9]{4}-'
                        THEN (cb.feature_snapshot->>'start_time')::timestamptz
                        ELSE NULL
                      END
                    )
                  ) <= now() - ($1::int * interval '1 minute')
                  ORDER BY event_start_time ASC NULLS LAST, sc.created_at ASC
                  LIMIT $2
                )
                UPDATE simulated_coupons sc
                SET status = 'awaiting_result',
                    latest_event_start_time = COALESCE(
                      sc.latest_event_start_time,
                      eligible.event_start_time
                    ),
                    expected_result_check_after = COALESCE(
                      sc.expected_result_check_after,
                      eligible.expected_result_check_after
                    ),
                    settlement_payload = sc.settlement_payload || jsonb_build_object(
                      'queue_transition',
                      jsonb_build_object(
                        'from_status', 'open',
                        'to_status', 'awaiting_result',
                        'transitioned_at', now(),
                        'source', 'settlement_queue',
                        'coupon_type', eligible.coupon_type,
                        'leg_count', eligible.leg_count,
                        'event_start_time', eligible.event_start_time,
                        'expected_result_check_after', eligible.expected_result_check_after,
                        'legs', eligible.legs,
                        'paper_only', true
                      )
                    )
                FROM eligible
                WHERE sc.id = eligible.id
                RETURNING
                  sc.id,
                  eligible.coupon_id,
                  eligible.coupon_type,
                  eligible.leg_count,
                  eligible.combined_decimal_odds,
                  eligible.strategy_id,
                  eligible.event_start_time,
                  eligible.expected_result_check_after,
                  eligible.legs
                "#,
                &[&(awaiting_grace_minutes as i32), &(limit as i64)],
            )
            .await?;
        for row in &coupon_rows {
            let coupon_id: String = row.get("id");
            client
                .execute(
                    "UPDATE simulated_coupon_legs SET status = 'awaiting_result' WHERE simulated_coupon_id = $1 AND status = 'open'",
                    &[&coupon_id],
                )
                .await?;
        }

        let mut items: Vec<Value> = rows
            .iter()
            .map(|row| {
                let event_start_time: Option<DateTime<Utc>> = row.get("event_start_time");
                let expected_result_check_after: Option<DateTime<Utc>> =
                    row.get("expected_result_check_after");
                json!({
                    "item_type": "single",
                    "bet_id": row.get::<_, String>("id"),
                    "event_id": row.get::<_, Option<String>>("event_id"),
                    "event_name": row.get::<_, Option<String>>("event_name"),
                    "market_name": row.get::<_, Option<String>>("market_name"),
                    "outcome_name": row.get::<_, Option<String>>("outcome_name"),
                    "sport_key": row.get::<_, Option<String>>("sport_key"),
                    "competition": row.get::<_, Option<String>>("competition"),
                    "event_start_time": event_start_time,
                    "expected_result_check_after": expected_result_check_after,
                    "event_status": row.get::<_, Option<String>>("event_status"),
                    "event_resulted": row.get::<_, Option<bool>>("event_resulted"),
                    "event_settled": row.get::<_, Option<bool>>("event_settled"),
                    "new_status": "awaiting_result"
                })
            })
            .collect();
        items.extend(coupon_rows.iter().map(|row| {
            let event_start_time: Option<DateTime<Utc>> = row.get("event_start_time");
            let expected_result_check_after: Option<DateTime<Utc>> =
                row.get("expected_result_check_after");
            json!({
                "item_type": "coupon",
                "coupon_simulation_id": row.get::<_, String>("id"),
                "coupon_id": row.get::<_, Option<String>>("coupon_id"),
                "coupon_type": row.get::<_, String>("coupon_type"),
                "leg_count": row.get::<_, i32>("leg_count"),
                "combined_decimal_odds": row.get::<_, Option<f64>>("combined_decimal_odds"),
                "strategy_id": row.get::<_, String>("strategy_id"),
                "event_start_time": event_start_time,
                "expected_result_check_after": expected_result_check_after,
                "legs": row.get::<_, Value>("legs"),
                "new_status": "awaiting_result"
            })
        }));

        Ok(json!({
            "enabled": true,
            "transitioned_count": rows.len() + coupon_rows.len(),
            "single_transitioned_count": rows.len(),
            "coupon_transitioned_count": coupon_rows.len(),
            "awaiting_grace_minutes": awaiting_grace_minutes,
            "limit": limit,
            "items": items,
            "paper_only": true
        }))
    }

    pub async fn refresh_settlement_review_queue(
        &self,
        limit: usize,
        lookup_cooldown_minutes: i64,
    ) -> anyhow::Result<Value> {
        if limit == 0 {
            return Ok(json!({
                "enabled": true,
                "review_count": 0,
                "skipped": true,
                "reason": "settlement review limit is zero"
            }));
        }

        let settlement_source_policy = self.settlement_sources().await?;
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                WITH review AS (
                  SELECT
                    sb.id AS bet_id,
                    sb.status AS bet_status,
                    sb.hypothetical_stake::float8 AS hypothetical_stake,
                    cb.id AS candidate_id,
                    cb.sport_key,
                    cb.event_id,
                    cb.event_name,
                    cb.competition,
                    cb.market_id,
                    cb.market_name,
                    cb.market_kind,
                    cb.outcome_id,
                    cb.outcome_name,
                    cb.decimal_odds::float8 AS observed_decimal_odds,
                    COALESCE(sb.event_start_time, se.start_time) AS start_time,
                    se.status AS event_status,
                    se.resulted AS event_resulted,
                    se.settled AS event_settled,
                    se.payload AS event_payload,
                    mo.market_id AS latest_market_id,
                    mo.active AS latest_market_active,
                    mo.displayed AS latest_market_displayed,
                    mo.payload AS latest_market_payload,
                    oo.outcome_id AS latest_outcome_id,
                    oo.outcome_name AS latest_outcome_name,
                    oo.active AS latest_outcome_active,
                    oo.displayed AS latest_outcome_displayed,
                    oo.decimal_odds::float8 AS latest_decimal_odds,
                    oo.payload AS latest_outcome_payload,
                    lkp.last_lookup_at,
                    COALESCE(
                      sb.expected_result_check_after,
                      CASE
                        WHEN COALESCE(sb.event_start_time, se.start_time) IS NULL THEN NULL
                        WHEN cb.sport_key = 'football' THEN COALESCE(sb.event_start_time, se.start_time) + interval '130 minutes'
                        WHEN cb.sport_key = 'basketball' THEN COALESCE(sb.event_start_time, se.start_time) + interval '150 minutes'
                        WHEN cb.sport_key = 'tennis' THEN COALESCE(sb.event_start_time, se.start_time) + interval '240 minutes'
                        WHEN cb.sport_key IN ('motorsports', 'golf', 'cycling') THEN COALESCE(sb.event_start_time, se.start_time) + interval '1 day'
                        ELSE COALESCE(sb.event_start_time, se.start_time) + interval '4 hours'
                      END
                    ) AS expected_result_check_after
                  FROM simulated_bets sb
                  JOIN candidate_bets cb ON cb.id = sb.candidate_id
                  LEFT JOIN sport_events se ON se.id = cb.event_id
                  LEFT JOIN LATERAL (
                    SELECT mo.*
                    FROM market_observations mo
                    WHERE mo.event_id = cb.event_id
                      AND mo.market_id = cb.market_id
                    ORDER BY mo.observed_at DESC
                    LIMIT 1
                  ) mo ON true
                  LEFT JOIN LATERAL (
                    SELECT oo.*
                    FROM outcome_observations oo
                    WHERE oo.market_observation_id = mo.id
                      AND oo.outcome_id = cb.outcome_id
                    ORDER BY oo.observed_at DESC
                    LIMIT 1
                  ) oo ON true
                  LEFT JOIN LATERAL (
                    SELECT max(created_at) AS last_lookup_at
                    FROM settlement_lookup_attempts sla
                    WHERE sla.simulated_bet_id = sb.id
                  ) lkp ON true
                  WHERE sb.status IN ('awaiting_result', 'unresolved', 'postponed')
                  ORDER BY se.start_time ASC NULLS LAST, sb.created_at ASC
                  LIMIT $1
                )
                UPDATE simulated_bets sb
                SET event_start_time = COALESCE(sb.event_start_time, review.start_time),
                    expected_result_check_after = COALESCE(
                      sb.expected_result_check_after,
                      review.expected_result_check_after
                    ),
                    settlement_payload = sb.settlement_payload || jsonb_build_object(
                  'review_evidence',
                  jsonb_build_object(
                    'source', 'danskespil_content_service',
                    'reviewed_at', now(),
                    'paper_only', true,
                    'not_auto_graded', true,
                    'requires_manual_grade', true,
                    'settlement_source_policy', $2::jsonb,
                    'bet_status', review.bet_status,
                    'sport_key', review.sport_key,
                    'event_id', review.event_id,
                    'event_name', review.event_name,
                    'competition', review.competition,
                    'market_id', review.market_id,
                    'market_name', review.market_name,
                    'market_kind', review.market_kind,
                    'outcome_id', review.outcome_id,
                    'outcome_name', review.outcome_name,
                    'observed_decimal_odds', review.observed_decimal_odds,
                    'start_time', review.start_time,
                    'expected_result_check_after', review.expected_result_check_after,
                    'event_status', review.event_status,
                    'event_resulted', review.event_resulted,
                    'event_settled', review.event_settled,
                    'latest_market_active', review.latest_market_active,
                    'latest_market_displayed', review.latest_market_displayed,
                    'latest_outcome_active', review.latest_outcome_active,
                    'latest_outcome_displayed', review.latest_outcome_displayed,
                    'latest_decimal_odds', review.latest_decimal_odds,
                    'last_lookup_at', review.last_lookup_at,
                    'latest_outcome_payload', review.latest_outcome_payload
                  )
                )
                FROM review
                WHERE sb.id = review.bet_id
                RETURNING
                  sb.id,
                  review.bet_status,
                  review.hypothetical_stake,
                  review.candidate_id,
                  review.sport_key,
                  review.event_id,
                  review.event_name,
                  review.competition,
                  review.market_id,
                  review.market_name,
                  review.market_kind,
                  review.outcome_id,
                  review.outcome_name,
                  review.observed_decimal_odds,
                  review.start_time,
                  review.expected_result_check_after,
                  review.event_status,
                  review.event_resulted,
                  review.event_settled,
                  review.latest_market_active,
                  review.latest_market_displayed,
                  review.latest_outcome_active,
                  review.latest_outcome_displayed,
                  review.latest_decimal_odds,
                  review.last_lookup_at,
                  review.latest_outcome_payload
                "#,
                &[&(limit as i64), &settlement_source_policy],
            )
            .await?;
        let coupon_rows = client
            .query(
                r#"
                WITH leg_review AS (
                  SELECT
                    sc.id AS simulated_coupon_id,
                    sc.status AS coupon_status,
                    sc.coupon_id,
                    sc.hypothetical_stake::float8 AS hypothetical_stake,
                    sc.observed_combined_decimal_odds::float8 AS combined_decimal_odds,
                    sc.strategy_id,
                    cc.coupon_type,
                    cc.leg_count,
                    scl.leg_index,
                    scl.status AS leg_status,
                    cb.id AS candidate_id,
                    cb.sport_key,
                    cb.event_id,
                    cb.event_name,
                    cb.competition,
                    cb.market_id,
                    cb.market_name,
                    cb.market_kind,
                    cb.outcome_id,
                    cb.outcome_name,
                    cb.decimal_odds::float8 AS observed_decimal_odds,
                    COALESCE(scl.event_start_time, se.start_time) AS start_time,
                    se.status AS event_status,
                    se.resulted AS event_resulted,
                    se.settled AS event_settled,
                    mo.active AS latest_market_active,
                    mo.displayed AS latest_market_displayed,
                    oo.active AS latest_outcome_active,
                    oo.displayed AS latest_outcome_displayed,
                    oo.decimal_odds::float8 AS latest_decimal_odds,
                    oo.payload AS latest_outcome_payload,
                    lkp.last_lookup_at,
                    COALESCE(
                      scl.expected_result_check_after,
                      CASE
                        WHEN COALESCE(scl.event_start_time, se.start_time) IS NULL THEN NULL
                        WHEN cb.sport_key = 'football' THEN COALESCE(scl.event_start_time, se.start_time) + interval '130 minutes'
                        WHEN cb.sport_key = 'basketball' THEN COALESCE(scl.event_start_time, se.start_time) + interval '150 minutes'
                        WHEN cb.sport_key = 'tennis' THEN COALESCE(scl.event_start_time, se.start_time) + interval '240 minutes'
                        WHEN cb.sport_key IN ('motorsports', 'golf', 'cycling') THEN COALESCE(scl.event_start_time, se.start_time) + interval '1 day'
                        ELSE COALESCE(scl.event_start_time, se.start_time) + interval '4 hours'
                      END
                    ) AS expected_result_check_after
                  FROM simulated_coupons sc
                  JOIN candidate_coupons cc ON cc.id = sc.coupon_id
                  JOIN simulated_coupon_legs scl ON scl.simulated_coupon_id = sc.id
                  JOIN candidate_bets cb ON cb.id = scl.candidate_id
                  LEFT JOIN sport_events se ON se.id = cb.event_id
                  LEFT JOIN LATERAL (
                    SELECT mo.*
                    FROM market_observations mo
                    WHERE mo.event_id = cb.event_id
                      AND mo.market_id = cb.market_id
                    ORDER BY mo.observed_at DESC
                    LIMIT 1
                  ) mo ON true
                  LEFT JOIN LATERAL (
                    SELECT oo.*
                    FROM outcome_observations oo
                    WHERE oo.market_observation_id = mo.id
                      AND oo.outcome_id = cb.outcome_id
                    ORDER BY oo.observed_at DESC
                    LIMIT 1
                  ) oo ON true
                  LEFT JOIN LATERAL (
                    SELECT max(created_at) AS last_lookup_at
                    FROM settlement_lookup_attempts sla
                    WHERE sla.simulated_coupon_id = sc.id
                  ) lkp ON true
                  WHERE sc.status IN ('awaiting_result', 'unresolved', 'postponed')
                ),
                review AS (
                  SELECT
                    simulated_coupon_id,
                    coupon_status,
                    coupon_id,
                    hypothetical_stake,
                    combined_decimal_odds,
                    strategy_id,
                    coupon_type,
                    leg_count,
                    max(expected_result_check_after) AS expected_result_check_after,
                    max(last_lookup_at) AS last_lookup_at,
                    jsonb_agg(
                      jsonb_build_object(
                        'leg_index', leg_index,
                        'leg_status', leg_status,
                        'candidate_id', candidate_id,
                        'sport_key', sport_key,
                        'event_id', event_id,
                        'event_name', event_name,
                        'competition', competition,
                        'market_id', market_id,
                        'market_name', market_name,
                        'market_kind', market_kind,
                        'outcome_id', outcome_id,
                        'outcome_name', outcome_name,
                        'observed_decimal_odds', observed_decimal_odds,
                        'start_time', start_time,
                        'expected_result_check_after', expected_result_check_after,
                        'event_status', event_status,
                        'event_resulted', event_resulted,
                        'event_settled', event_settled,
                        'latest_market_active', latest_market_active,
                        'latest_market_displayed', latest_market_displayed,
                        'latest_outcome_active', latest_outcome_active,
                        'latest_outcome_displayed', latest_outcome_displayed,
                        'latest_decimal_odds', latest_decimal_odds,
                        'last_lookup_at', last_lookup_at,
                        'latest_outcome_payload', latest_outcome_payload
                      )
                      ORDER BY leg_index
                    ) AS legs
                  FROM leg_review
                  GROUP BY simulated_coupon_id, coupon_status, coupon_id,
                           hypothetical_stake, combined_decimal_odds, strategy_id, coupon_type, leg_count
                  ORDER BY max(expected_result_check_after) ASC NULLS LAST
                  LIMIT $1
                )
                UPDATE simulated_coupons sc
                SET latest_event_start_time = COALESCE(
                      sc.latest_event_start_time,
                      (SELECT max((leg->>'start_time')::timestamptz) FROM jsonb_array_elements(review.legs) leg WHERE leg ? 'start_time')
                    ),
                    expected_result_check_after = COALESCE(
                      sc.expected_result_check_after,
                      review.expected_result_check_after
                    ),
                    settlement_payload = sc.settlement_payload || jsonb_build_object(
                  'review_evidence',
                  jsonb_build_object(
                    'source', 'danskespil_content_service',
                    'reviewed_at', now(),
                    'paper_only', true,
                    'not_auto_graded', true,
                    'requires_manual_grade', true,
                    'settlement_source_policy', $2::jsonb,
                    'coupon_status', review.coupon_status,
                    'coupon_id', review.coupon_id,
                    'coupon_type', review.coupon_type,
                    'leg_count', review.leg_count,
                    'hypothetical_stake', review.hypothetical_stake,
                    'combined_decimal_odds', review.combined_decimal_odds,
                    'expected_result_check_after', review.expected_result_check_after,
                    'last_lookup_at', review.last_lookup_at,
                    'legs', review.legs
                  )
                )
                FROM review
                WHERE sc.id = review.simulated_coupon_id
                RETURNING
                  sc.id,
                  review.coupon_status,
                  review.coupon_id,
                  review.coupon_type,
                  review.leg_count,
                  review.hypothetical_stake,
                  review.combined_decimal_odds,
                  review.strategy_id,
                  review.expected_result_check_after,
                  review.last_lookup_at,
                  review.legs
                "#,
                &[&(limit as i64), &settlement_source_policy],
            )
            .await?;

        let mut items: Vec<Value> = rows
            .iter()
            .map(|row| {
                let start_time: Option<DateTime<Utc>> = row.get("start_time");
                let expected_result_check_after: Option<DateTime<Utc>> =
                    row.get("expected_result_check_after");
                let last_lookup_at: Option<DateTime<Utc>> = row.get("last_lookup_at");
                let event_name: Option<String> = row.get("event_name");
                let lookup_stale = last_lookup_at
                    .map(|value| {
                        value
                            < Utc::now() - Duration::minutes(lookup_cooldown_minutes.max(0))
                    })
                    .unwrap_or(true);
                let event_status: Option<String> = row.get("event_status");
                let event_resulted: Option<bool> = row.get("event_resulted");
                let event_settled: Option<bool> = row.get("event_settled");
                let recommendation = settlement_review_recommendation(
                    event_status.as_deref(),
                    event_resulted,
                    event_settled,
                    expected_result_check_after,
                );
                let overdue_minutes = settlement_overdue_minutes(expected_result_check_after);
                let recommended_source_key = settlement_recommended_source_key(recommendation);
                let recommended_source_keys = settlement_recommended_source_keys(recommendation);
                let external_result_links: Vec<Value> = event_name
                    .as_deref()
                    .map(|name| external_result_links_for_event(&settlement_source_policy, name))
                    .unwrap_or_default()
                    .iter()
                    .map(external_result_link_json)
                    .collect();
                json!({
                    "item_type": "single",
                    "bet_id": row.get::<_, String>("id"),
                    "bet_status": row.get::<_, String>("bet_status"),
                    "hypothetical_stake": row.get::<_, f64>("hypothetical_stake"),
                    "candidate_id": row.get::<_, String>("candidate_id"),
                    "sport_key": row.get::<_, String>("sport_key"),
                    "event_id": row.get::<_, Option<String>>("event_id"),
                    "event_name": event_name,
                    "competition": row.get::<_, Option<String>>("competition"),
                    "market_id": row.get::<_, Option<String>>("market_id"),
                    "market_name": row.get::<_, Option<String>>("market_name"),
                    "market_kind": row.get::<_, Option<String>>("market_kind"),
                    "outcome_id": row.get::<_, Option<String>>("outcome_id"),
                    "outcome_name": row.get::<_, Option<String>>("outcome_name"),
                    "observed_decimal_odds": row.get::<_, Option<f64>>("observed_decimal_odds"),
                    "start_time": start_time,
                    "expected_result_check_after": expected_result_check_after,
                    "last_lookup_at": last_lookup_at,
                    "lookup_stale": lookup_stale,
                    "lookup_cooldown_minutes": lookup_cooldown_minutes,
                    "overdue_minutes": overdue_minutes,
                    "event_status": event_status,
                    "event_resulted": event_resulted,
                    "event_settled": event_settled,
                    "latest_market_active": row.get::<_, Option<bool>>("latest_market_active"),
                    "latest_market_displayed": row.get::<_, Option<bool>>("latest_market_displayed"),
                    "latest_outcome_active": row.get::<_, Option<bool>>("latest_outcome_active"),
                    "latest_outcome_displayed": row.get::<_, Option<bool>>("latest_outcome_displayed"),
                    "latest_decimal_odds": row.get::<_, Option<f64>>("latest_decimal_odds"),
                    "latest_outcome_payload": row.get::<_, Option<Value>>("latest_outcome_payload"),
                    "settlement_source_policy": settlement_source_policy.clone(),
                    "recommendation": recommendation,
                    "recommended_source_key": recommended_source_key,
                    "recommended_source_keys": recommended_source_keys,
                    "external_result_link": external_result_links.first().cloned().unwrap_or(Value::Null),
                    "external_result_links": external_result_links
                })
            })
            .collect();
        items.extend(coupon_rows.iter().map(|row| {
            let expected_result_check_after: Option<DateTime<Utc>> =
                row.get("expected_result_check_after");
            let last_lookup_at: Option<DateTime<Utc>> = row.get("last_lookup_at");
            let lookup_stale = last_lookup_at
                .map(|value| value < Utc::now() - Duration::minutes(lookup_cooldown_minutes.max(0)))
                .unwrap_or(true);
            let legs: Value = row.get("legs");
            let recommendation =
                coupon_settlement_review_recommendation(&legs, expected_result_check_after);
            let overdue_minutes = settlement_overdue_minutes(expected_result_check_after);
            let recommended_source_key = settlement_recommended_source_key(recommendation);
            let recommended_source_keys = settlement_recommended_source_keys(recommendation);
            let external_result_links: Vec<Value> = legs
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .filter_map(|leg| leg.get("event_name").and_then(Value::as_str))
                .flat_map(|name| external_result_links_for_event(&settlement_source_policy, name))
                .map(|link| external_result_link_json(&link))
                .collect();
            json!({
                "item_type": "coupon",
                "coupon_simulation_id": row.get::<_, String>("id"),
                "coupon_status": row.get::<_, String>("coupon_status"),
                "coupon_id": row.get::<_, Option<String>>("coupon_id"),
                "coupon_type": row.get::<_, String>("coupon_type"),
                "leg_count": row.get::<_, i32>("leg_count"),
                "hypothetical_stake": row.get::<_, f64>("hypothetical_stake"),
                "combined_decimal_odds": row.get::<_, Option<f64>>("combined_decimal_odds"),
                "strategy_id": row.get::<_, String>("strategy_id"),
                "expected_result_check_after": expected_result_check_after,
                "last_lookup_at": last_lookup_at,
                "lookup_stale": lookup_stale,
                "lookup_cooldown_minutes": lookup_cooldown_minutes,
                "overdue_minutes": overdue_minutes,
                "legs": legs,
                "settlement_source_policy": settlement_source_policy.clone(),
                "recommendation": recommendation,
                "recommended_source_key": recommended_source_key,
                "recommended_source_keys": recommended_source_keys,
                "external_result_links": external_result_links
            })
        }));

        let lookup_attempt_count = self
            .record_settlement_lookup_attempts(
                &client,
                &items,
                &settlement_source_policy,
                lookup_cooldown_minutes,
            )
            .await?;

        Ok(json!({
            "enabled": true,
            "review_count": rows.len() + coupon_rows.len(),
            "single_review_count": rows.len(),
            "coupon_review_count": coupon_rows.len(),
            "lookup_attempt_count": lookup_attempt_count,
            "lookup_cooldown_minutes": lookup_cooldown_minutes,
            "limit": limit,
            "items": items,
            "settlement_source_policy": settlement_source_policy,
            "paper_only": true,
            "not_auto_graded": true
        }))
    }

    pub async fn auto_settle_external_overdue(
        &self,
        min_overdue_minutes: i64,
        limit: usize,
    ) -> anyhow::Result<Value> {
        if limit == 0 {
            return Ok(json!({
                "enabled": true,
                "checked_count": 0,
                "settled_count": 0,
                "skipped": true,
                "reason": "settlement auto-check limit is zero"
            }));
        }

        let source_policy = self.settlement_sources().await?;
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                WITH base AS (
                  SELECT
                    sb.id AS bet_id,
                    sb.created_at,
                    cb.event_name,
                    cb.sport_key,
                    cb.market_kind,
                    cb.market_name,
                    cb.outcome_name,
                    sb.expected_result_check_after AS stored_expected_result_check_after,
                    COALESCE(
                      sb.event_start_time,
                      se.start_time,
                      CASE
                        WHEN cb.feature_snapshot ? 'start_time'
                         AND cb.feature_snapshot->>'start_time' ~ '^[0-9]{4}-'
                        THEN (cb.feature_snapshot->>'start_time')::timestamptz
                        ELSE NULL
                      END
                    ) AS event_start_time
                  FROM simulated_bets sb
                  JOIN candidate_bets cb ON cb.id = sb.candidate_id
                  LEFT JOIN sport_events se ON se.id = cb.event_id
                  WHERE sb.status IN ('awaiting_result', 'unresolved', 'postponed')
                ),
                candidates AS (
                  SELECT
                    *,
                    COALESCE(
                      CASE
                        WHEN event_start_time IS NULL THEN NULL
                        WHEN sport_key = 'football' THEN event_start_time + interval '130 minutes'
                        WHEN sport_key = 'basketball' THEN event_start_time + interval '150 minutes'
                        WHEN sport_key = 'tennis' THEN event_start_time + interval '240 minutes'
                        WHEN sport_key IN ('motorsports', 'golf', 'cycling') THEN event_start_time + interval '1 day'
                        ELSE event_start_time + interval '4 hours'
                      END,
                      stored_expected_result_check_after
                    ) AS expected_event_finish_at
                  FROM base
                )
                SELECT
                  bet_id,
                  event_name,
                  sport_key,
                  market_kind,
                  market_name,
                  outcome_name,
                  event_start_time,
                  expected_event_finish_at AS expected_result_check_after
                FROM candidates
                WHERE expected_event_finish_at <= now() - ($1::int * interval '1 minute')
                ORDER BY expected_event_finish_at ASC NULLS LAST, created_at ASC
                LIMIT $2
                "#,
                &[&(min_overdue_minutes.max(0) as i32), &(limit as i64)],
            )
            .await?;
        drop(client);

        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
            ),
        );
        headers.insert(
            ACCEPT_LANGUAGE,
            HeaderValue::from_static("da-DK,da;q=0.9,en-US;q=0.8,en;q=0.7"),
        );
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        headers.insert(PRAGMA, HeaderValue::from_static("no-cache"));
        headers.insert(
            HeaderName::from_static("upgrade-insecure-requests"),
            HeaderValue::from_static("1"),
        );
        let http = HttpClient::builder()
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
            .default_headers(headers)
            .timeout(StdDuration::from_secs(12))
            .build()?;

        let mut checked = Vec::new();
        let mut settled = Vec::new();
        let mut skipped = Vec::new();

        for row in rows {
            let bet_id: String = row.get("bet_id");
            let event_name: Option<String> = row.get("event_name");
            let market_kind: Option<String> = row.get("market_kind");
            let market_name: Option<String> = row.get("market_name");
            let outcome_name: Option<String> = row.get("outcome_name");
            let event_start_time: Option<DateTime<Utc>> = row.get("event_start_time");
            let expected_result_check_after: Option<DateTime<Utc>> =
                row.get("expected_result_check_after");
            let overdue_minutes = settlement_overdue_minutes(expected_result_check_after);
            let Some(event_name) = event_name else {
                skipped.push(json!({"bet_id": bet_id, "reason": "missing_event_name"}));
                continue;
            };
            let Some(outcome_name) = outcome_name else {
                skipped.push(json!({"bet_id": bet_id, "event_name": event_name, "reason": "missing_outcome_name"}));
                continue;
            };
            if !is_auto_settle_external_market(market_kind.as_deref(), market_name.as_deref()) {
                skipped.push(json!({
                    "bet_id": bet_id,
                    "event_name": event_name,
                    "market_kind": market_kind,
                    "market_name": market_name,
                    "reason": "unsupported_market_for_auto_settlement"
                }));
                continue;
            }
            let links = external_result_links_for_event(&source_policy, &event_name);
            if links.is_empty() {
                skipped.push(json!({
                    "bet_id": bet_id,
                    "event_name": event_name,
                    "reason": "no_configured_external_result_link"
                }));
                continue;
            };
            let mut link_attempts = Vec::new();
            let mut evidence_pair = None;
            for link in links {
                if link.requires_browser_automation {
                    link_attempts.push(json!({
                        "source_key": link.source_key,
                        "source_url": link.url,
                        "reason": "browser_automation_required_for_source",
                        "direct_http_fetch": "blocked_in_local_testing",
                        "manual_browser_verification": "agent-browser can access the page"
                    }));
                    continue;
                }
                match fetch_external_match_result(&http, &link).await {
                    Ok(evidence) => {
                        evidence_pair = Some((link, evidence));
                        break;
                    }
                    Err(error) => link_attempts.push(json!({
                        "source_key": link.source_key,
                        "source_url": link.url,
                        "reason": "external_fetch_or_parse_failed",
                        "error": error.to_string()
                    })),
                }
            }
            let Some((link, evidence)) = evidence_pair else {
                skipped.push(json!({
                    "bet_id": bet_id,
                    "event_name": event_name,
                    "reason": "no_direct_external_result_evidence",
                    "attempts": link_attempts,
                    "event_start_time": event_start_time,
                    "expected_event_finish_at": expected_result_check_after,
                    "overdue_minutes": overdue_minutes
                }));
                continue;
            };
            let evidence_id = match self
                .record_external_result_evidence_from_link(&event_name, &link, &evidence)
                .await
            {
                Ok(evidence_id) => evidence_id,
                Err(error) => {
                    skipped.push(json!({
                        "bet_id": bet_id,
                        "event_name": event_name,
                        "source_key": evidence.source_key,
                        "source_url": evidence.url,
                        "source_title": evidence.title,
                        "reason": "external_result_evidence_persist_failed",
                        "error": error.to_string()
                    }));
                    continue;
                }
            };
            checked.push(json!({
                "bet_id": bet_id,
                "event_name": event_name,
                "external_result_evidence_id": evidence_id,
                "source_key": evidence.source_key,
                "source_url": evidence.url,
                "source_title": evidence.title,
                "attempts": link_attempts,
                "event_start_time": event_start_time,
                "expected_event_finish_at": expected_result_check_after,
                "score": {"home": evidence.home_score, "away": evidence.away_score}
            }));
            let Some(result) = grade_external_outcome(
                &outcome_name,
                market_kind.as_deref(),
                market_name.as_deref(),
                &link,
                &evidence,
            ) else {
                skipped.push(json!({
                    "bet_id": bet_id,
                    "event_name": event_name,
                    "outcome_name": outcome_name,
                    "source_key": evidence.source_key,
                    "source_url": evidence.url,
                    "source_title": evidence.title,
                    "score": {"home": evidence.home_score, "away": evidence.away_score},
                    "event_start_time": event_start_time,
                    "expected_event_finish_at": expected_result_check_after,
                    "reason": "unable_to_map_outcome_to_external_result"
                }));
                continue;
            };
            let notes = json!({
                "mode": "auto_external_result_settlement",
                "external_result_evidence_id": evidence_id,
                "source_key": evidence.source_key,
                "source_url": evidence.url,
                "source_title": evidence.title,
                "source_home_name": evidence.home_name,
                "source_away_name": evidence.away_name,
                "event_start_time": event_start_time,
                "expected_event_finish_at": expected_result_check_after,
                "external_check_grace_minutes": min_overdue_minutes,
                "home_score": evidence.home_score,
                "away_score": evidence.away_score,
                "outcome_name": outcome_name,
                "overdue_minutes": overdue_minutes,
                "paper_only": true
            })
            .to_string();
            match self
                .settle_simulated_bet(
                    &bet_id,
                    result,
                    &evidence.source_key,
                    evidence.confidence,
                    &notes,
                )
                .await
            {
                Ok(item) => {
                    let audit = json!({
                        "external_result_evidence_id": evidence_id,
                        "bet_id": item.id,
                        "event_name": event_name,
                        "outcome_name": outcome_name,
                        "result": result,
                        "status": item.status,
                        "source_key": evidence.source_key,
                        "source_url": evidence.url,
                        "source_title": evidence.title,
                        "score": {"home": evidence.home_score, "away": evidence.away_score},
                        "event_start_time": event_start_time,
                        "expected_event_finish_at": expected_result_check_after,
                        "external_check_grace_minutes": min_overdue_minutes,
                        "overdue_minutes": overdue_minutes,
                        "paper_only": true
                    });
                    self.record_audit("paper_bet_auto_settled_external", audit.clone())
                        .await
                        .ok();
                    self.mark_external_result_evidence_used(&evidence_id)
                        .await
                        .ok();
                    settled.push(audit);
                }
                Err(error) => skipped.push(json!({
                    "bet_id": bet_id,
                    "event_name": event_name,
                    "source_key": evidence.source_key,
                    "reason": "settlement_write_failed",
                    "error": error.to_string()
                })),
            }
        }

        Ok(json!({
            "enabled": true,
            "paper_only": true,
            "min_overdue_minutes": min_overdue_minutes,
            "overdue_basis": "expected_event_finish_at",
            "candidate_count": checked.len() + skipped.len(),
            "checked_count": checked.len(),
            "settled_count": settled.len(),
            "skipped_count": skipped.len(),
            "checked": checked,
            "settled": settled,
            "skipped": skipped
        }))
    }

    pub async fn auto_settle_external_result_task(&self, task: &Value) -> anyhow::Result<Value> {
        let bet_id = task
            .get("ids")
            .and_then(|ids| ids.get("bet_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("task ids.bet_id is required"))?;
        let selection = task.get("selection").unwrap_or(&Value::Null);
        let event_name = selection
            .get("event_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("task selection.event_name is required"))?;
        let outcome_name = selection
            .get("outcome_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("task selection.outcome_name is required"))?;
        let market_kind = selection.get("market_kind").and_then(Value::as_str);
        let market_name = selection.get("market_name").and_then(Value::as_str);
        let expected_result_check_after = task
            .get("expected_result_check_after")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_utc);
        let overdue_minutes = settlement_overdue_minutes(expected_result_check_after);
        if !is_auto_settle_external_market(market_kind, market_name) {
            return Ok(json!({
                "enabled": true,
                "paper_only": true,
                "settled_count": 0,
                "checked_count": 0,
                "skipped_count": 1,
                "skipped": [{
                    "bet_id": bet_id,
                    "event_name": event_name,
                    "market_kind": market_kind,
                    "market_name": market_name,
                    "reason": "unsupported_market_for_auto_settlement"
                }]
            }));
        }
        let links = task
            .get("source_links")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(external_result_link_from_task_source)
            .collect::<Vec<_>>();
        if links.is_empty() {
            return Ok(json!({
                "enabled": true,
                "paper_only": true,
                "settled_count": 0,
                "checked_count": 0,
                "skipped_count": 1,
                "skipped": [{
                    "bet_id": bet_id,
                    "event_name": event_name,
                    "reason": "no_configured_external_result_link"
                }]
            }));
        }

        let http = external_result_http_client()?;
        let mut link_attempts = Vec::new();
        let mut evidence_pair = None;
        for link in links {
            if link.requires_browser_automation {
                link_attempts.push(json!({
                    "source_key": link.source_key,
                    "source_url": link.url,
                    "reason": "browser_automation_required_for_source",
                    "direct_http_fetch": "blocked_in_local_testing",
                    "manual_browser_verification": "agent-browser can access the page"
                }));
                continue;
            }
            match fetch_external_match_result(&http, &link).await {
                Ok(evidence) => {
                    evidence_pair = Some((link, evidence));
                    break;
                }
                Err(error) => link_attempts.push(json!({
                    "source_key": link.source_key,
                    "source_url": link.url,
                    "reason": "external_fetch_or_parse_failed",
                    "error": error.to_string()
                })),
            }
        }

        let Some((link, evidence)) = evidence_pair else {
            return Ok(json!({
                "enabled": true,
                "paper_only": true,
                "settled_count": 0,
                "checked_count": 0,
                "skipped_count": 1,
                "skipped": [{
                    "bet_id": bet_id,
                    "event_name": event_name,
                    "reason": "no_direct_external_result_evidence",
                    "attempts": link_attempts,
                    "expected_event_finish_at": expected_result_check_after,
                    "overdue_minutes": overdue_minutes
                }]
            }));
        };
        let evidence_id = self
            .record_external_result_evidence_from_link(event_name, &link, &evidence)
            .await?;
        let checked = json!({
            "bet_id": bet_id,
            "event_name": event_name,
            "external_result_evidence_id": evidence_id,
            "source_key": evidence.source_key,
            "source_url": evidence.url,
            "source_title": evidence.title,
            "attempts": link_attempts,
            "expected_event_finish_at": expected_result_check_after,
            "score": {"home": evidence.home_score, "away": evidence.away_score}
        });
        let Some(result) =
            grade_external_outcome(outcome_name, market_kind, market_name, &link, &evidence)
        else {
            return Ok(json!({
                "enabled": true,
                "paper_only": true,
                "settled_count": 0,
                "checked_count": 1,
                "checked": [checked],
                "skipped_count": 1,
                "skipped": [{
                    "bet_id": bet_id,
                    "event_name": event_name,
                    "outcome_name": outcome_name,
                    "source_key": evidence.source_key,
                    "source_url": evidence.url,
                    "source_title": evidence.title,
                    "score": {"home": evidence.home_score, "away": evidence.away_score},
                    "expected_event_finish_at": expected_result_check_after,
                    "reason": "unable_to_map_outcome_to_external_result"
                }]
            }));
        };
        let notes = json!({
            "mode": "queued_external_result_task_settlement",
            "external_result_evidence_id": evidence_id,
            "source_key": evidence.source_key,
            "source_url": evidence.url,
            "source_title": evidence.title,
            "source_home_name": evidence.home_name,
            "source_away_name": evidence.away_name,
            "expected_event_finish_at": expected_result_check_after,
            "home_score": evidence.home_score,
            "away_score": evidence.away_score,
            "outcome_name": outcome_name,
            "overdue_minutes": overdue_minutes,
            "paper_only": true
        })
        .to_string();
        let item = self
            .settle_simulated_bet(
                bet_id,
                result,
                &evidence.source_key,
                evidence.confidence,
                &notes,
            )
            .await?;
        let audit = json!({
            "external_result_evidence_id": evidence_id,
            "bet_id": item.id,
            "event_name": event_name,
            "outcome_name": outcome_name,
            "result": result,
            "status": item.status,
            "source_key": evidence.source_key,
            "source_url": evidence.url,
            "source_title": evidence.title,
            "score": {"home": evidence.home_score, "away": evidence.away_score},
            "expected_event_finish_at": expected_result_check_after,
            "overdue_minutes": overdue_minutes,
            "paper_only": true
        });
        self.record_audit("paper_bet_auto_settled_external_task", audit.clone())
            .await
            .ok();
        self.mark_external_result_evidence_used(&evidence_id)
            .await
            .ok();
        Ok(json!({
            "enabled": true,
            "paper_only": true,
            "settled_count": 1,
            "checked_count": 1,
            "skipped_count": 0,
            "checked": [checked],
            "settled": [audit]
        }))
    }

    async fn record_external_result_evidence_from_link(
        &self,
        event_name: &str,
        link: &ExternalResultLink,
        evidence: &ExternalMatchResult,
    ) -> anyhow::Result<String> {
        let evidence_id = new_id();
        let payload = json!({
            "source_key": evidence.source_key,
            "source_url": evidence.url,
            "source_title": evidence.title,
            "event_name": event_name,
            "home_name": evidence.home_name,
            "away_name": evidence.away_name,
            "home_aliases": link.home_aliases,
            "away_aliases": link.away_aliases,
            "home_score": evidence.home_score,
            "away_score": evidence.away_score,
            "confidence": evidence.confidence,
            "sport_key": link.sport_key,
            "gender_scope": link.gender_scope,
            "known_result": match (link.known_home_score, link.known_away_score) {
                (Some(home), Some(away)) => json!({
                    "home_score": home,
                    "away_score": away,
                    "status": link.known_result_status,
                    "notes": link.known_result_notes
                }),
                _ => Value::Null
            },
            "browser_automation": {
                "tool": "rust_reqwest",
                "source": evidence.source_key,
                "direct_http_or_known_result": true
            },
            "raw_text_excerpt": format!(
                "{} - {} {}:{}",
                evidence.home_name, evidence.away_name, evidence.home_score, evidence.away_score
            ),
            "mode": "auto_external_result_settlement",
            "paper_only": true
        });
        let source_url = Some(evidence.url.clone());
        let client = self.connect().await?;
        client
            .execute(
                r#"
                INSERT INTO external_result_evidence (
                  id, source_key, source_url, event_name, home_name, away_name,
                  home_score, away_score, confidence, payload
                )
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8,($9::float8)::numeric,$10)
                "#,
                &[
                    &evidence_id,
                    &evidence.source_key,
                    &source_url,
                    &event_name,
                    &evidence.home_name,
                    &evidence.away_name,
                    &evidence.home_score,
                    &evidence.away_score,
                    &evidence.confidence,
                    &payload,
                ],
            )
            .await?;
        Ok(evidence_id)
    }

    async fn mark_external_result_evidence_used(&self, evidence_id: &str) -> anyhow::Result<()> {
        let client = self.connect().await?;
        client
            .execute(
                "UPDATE external_result_evidence SET used_for_settlement = true WHERE id = $1",
                &[&evidence_id],
            )
            .await?;
        Ok(())
    }

    pub async fn ingest_external_result_evidence(&self, payload: &Value) -> anyhow::Result<Value> {
        let source_key = payload
            .get("source_key")
            .or_else(|| payload.get("source"))
            .and_then(Value::as_str)
            .unwrap_or("documented_third_party_results");
        let source_record = self.settlement_source_record(source_key).await?;
        let source_url = payload
            .get("source_url")
            .or_else(|| payload.get("url"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let title = payload
            .get("source_title")
            .or_else(|| payload.get("title"))
            .and_then(Value::as_str)
            .map(str::to_string);
        if source_key == "danskespil_account_history" {
            if let Some(settlement_result) = account_history_settlement_result(payload) {
                return self
                    .ingest_account_history_settlement_evidence(
                        payload,
                        source_key,
                        source_record,
                        source_url,
                        title,
                        settlement_result,
                    )
                    .await;
            }
        }
        let event_name = payload
            .get("event_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                title
                    .as_deref()
                    .and_then(parse_score_title)
                    .map(|(home, away, _, _)| format!("{home} - {away}"))
            })
            .ok_or_else(|| anyhow!("event_name is required for external result evidence"))?;
        let parsed_title = title.as_deref().and_then(parse_score_title);
        let home_name = payload
            .get("home_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| parsed_title.as_ref().map(|(home, _, _, _)| home.clone()))
            .ok_or_else(|| anyhow!("home_name is required for external result evidence"))?;
        let away_name = payload
            .get("away_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| parsed_title.as_ref().map(|(_, away, _, _)| away.clone()))
            .ok_or_else(|| anyhow!("away_name is required for external result evidence"))?;
        let home_score = json_i32(payload.get("home_score"))
            .or_else(|| {
                parsed_title
                    .as_ref()
                    .map(|(_, _, home_score, _)| *home_score)
            })
            .ok_or_else(|| anyhow!("home_score is required for external result evidence"))?;
        let away_score = json_i32(payload.get("away_score"))
            .or_else(|| {
                parsed_title
                    .as_ref()
                    .map(|(_, _, _, away_score)| *away_score)
            })
            .ok_or_else(|| anyhow!("away_score is required for external result evidence"))?;
        let confidence = payload
            .get("confidence")
            .and_then(Value::as_f64)
            .or_else(|| source_record.get("reliability").and_then(Value::as_f64))
            .unwrap_or(0.7)
            .clamp(0.0, 1.0);
        let settle = payload
            .get("settle")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let bet_id_filter = payload
            .get("bet_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let evidence_id = new_id();
        let evidence_payload = json!({
            "source_key": source_key,
            "source_record": source_record,
            "source_url": source_url,
            "source_title": title,
            "event_name": event_name,
            "home_name": home_name,
            "away_name": away_name,
            "home_score": home_score,
            "away_score": away_score,
            "confidence": confidence,
            "sport_key": payload.get("sport_key").cloned().unwrap_or(Value::Null),
            "gender_scope": payload.get("gender_scope").or_else(|| payload.get("gender")).cloned().unwrap_or(Value::Null),
            "browser_automation": payload.get("browser_automation").cloned().unwrap_or(Value::Null),
            "browser_session": payload.get("browser_session").cloned().unwrap_or(Value::Null),
            "raw_text_excerpt": payload.get("raw_text_excerpt").cloned().unwrap_or(Value::Null),
            "paper_only": true
        });
        let client = self.connect().await?;
        client
            .execute(
                r#"
                INSERT INTO external_result_evidence (
                  id, source_key, source_url, event_name, home_name, away_name,
                  home_score, away_score, confidence, payload
                )
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8,($9::float8)::numeric,$10)
                "#,
                &[
                    &evidence_id,
                    &source_key,
                    &source_url,
                    &event_name,
                    &home_name,
                    &away_name,
                    &home_score,
                    &away_score,
                    &confidence,
                    &evidence_payload,
                ],
            )
            .await?;
        let sport_key = payload
            .get("sport_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let gender_scope = payload_gender_scope(payload)?;
        let alias_notes = json!({
            "event_name": event_name,
            "source_url": source_url,
            "method": "external_result_evidence",
            "external_result_evidence_id": evidence_id,
            "paper_only": true
        });
        let recorded_home_aliases = record_entity_aliases(
            &client,
            "participant",
            sport_key,
            gender_scope.as_deref(),
            &home_name,
            json_string_array(payload.get("home_aliases"))
                .into_iter()
                .chain(std::iter::once(home_name.clone()))
                .collect(),
            Some(source_key),
            None,
            confidence,
            alias_notes.clone(),
        )
        .await?;
        let recorded_away_aliases = record_entity_aliases(
            &client,
            "participant",
            sport_key,
            gender_scope.as_deref(),
            &away_name,
            json_string_array(payload.get("away_aliases"))
                .into_iter()
                .chain(std::iter::once(away_name.clone()))
                .collect(),
            Some(source_key),
            None,
            confidence,
            alias_notes,
        )
        .await?;

        let rows = client
            .query(
                r#"
                SELECT
                  sb.id AS bet_id,
                  cb.event_name,
                  cb.market_kind,
                  cb.market_name,
                  cb.outcome_name
                FROM simulated_bets sb
                JOIN candidate_bets cb ON cb.id = sb.candidate_id
                WHERE sb.status IN ('awaiting_result', 'unresolved', 'postponed')
                  AND (
                    ($1::text IS NOT NULL AND sb.id = $1)
                    OR ($1::text IS NULL AND cb.event_name = ANY($2))
                  )
                ORDER BY sb.created_at ASC
                LIMIT 25
                "#,
                &[&bet_id_filter, &event_name_variants(&event_name)],
            )
            .await?;
        drop(client);

        let source_policy = self.settlement_sources().await?;
        let mut link =
            external_result_link_for_event(&source_policy, &event_name).unwrap_or_else(|| {
                ExternalResultLink {
                    source_key: source_key.to_string(),
                    url: source_url.clone().unwrap_or_default(),
                    sport_key: sport_key.map(str::to_string),
                    gender_scope: gender_scope.clone(),
                    home_aliases: json_string_array(payload.get("home_aliases")),
                    away_aliases: json_string_array(payload.get("away_aliases")),
                    requires_browser_automation: false,
                    known_home_score: json_i32(payload.get("home_score")),
                    known_away_score: json_i32(payload.get("away_score")),
                    known_result_status: payload
                        .get("result_status")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    known_result_notes: payload
                        .get("notes")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                }
            });
        link.home_aliases.push(home_name.clone());
        link.away_aliases.push(away_name.clone());
        let alias_client = self.connect().await?;
        link.home_aliases = expand_aliases_from_registry(
            &alias_client,
            "participant",
            link.sport_key.as_deref(),
            link.gender_scope.as_deref(),
            link.home_aliases,
        )
        .await?;
        link.away_aliases = expand_aliases_from_registry(
            &alias_client,
            "participant",
            link.sport_key.as_deref(),
            link.gender_scope.as_deref(),
            link.away_aliases,
        )
        .await?;
        let evidence = ExternalMatchResult {
            source_key: source_key.to_string(),
            url: source_url.clone().unwrap_or_default(),
            title: title
                .clone()
                .unwrap_or_else(|| format!("{home_name} - {away_name} {home_score}:{away_score}")),
            home_name: home_name.clone(),
            away_name: away_name.clone(),
            home_score,
            away_score,
            confidence,
        };

        let mut settled = Vec::new();
        let mut skipped = Vec::new();
        for row in rows {
            let bet_id: String = row.get("bet_id");
            let market_kind: Option<String> = row.get("market_kind");
            let market_name: Option<String> = row.get("market_name");
            let outcome_name: Option<String> = row.get("outcome_name");
            let row_event_name: Option<String> = row.get("event_name");
            let Some(outcome_name) = outcome_name else {
                skipped.push(json!({
                    "bet_id": bet_id,
                    "reason": "missing_outcome_name"
                }));
                continue;
            };
            if !is_auto_settle_external_market(market_kind.as_deref(), market_name.as_deref()) {
                skipped.push(json!({
                    "bet_id": bet_id,
                    "event_name": row_event_name,
                    "market_kind": market_kind,
                    "market_name": market_name,
                    "reason": "unsupported_market_for_external_evidence_settlement"
                }));
                continue;
            }
            let Some(result) = grade_external_outcome(
                &outcome_name,
                market_kind.as_deref(),
                market_name.as_deref(),
                &link,
                &evidence,
            ) else {
                skipped.push(json!({
                    "bet_id": bet_id,
                    "event_name": row_event_name,
                    "outcome_name": outcome_name,
                    "reason": "unable_to_map_outcome_to_external_result"
                }));
                continue;
            };
            if !settle {
                skipped.push(json!({
                    "bet_id": bet_id,
                    "event_name": row_event_name,
                    "outcome_name": outcome_name,
                    "mapped_result": result,
                    "reason": "settle_false"
                }));
                continue;
            }
            let notes = json!({
                "mode": "browser_external_result_evidence_settlement",
                "external_result_evidence_id": evidence_id,
                "source_key": source_key,
                "source_url": source_url,
                "source_title": title,
                "home_name": home_name,
                "away_name": away_name,
                "home_score": home_score,
                "away_score": away_score,
                "outcome_name": outcome_name,
                "paper_only": true
            })
            .to_string();
            match self
                .settle_simulated_bet(&bet_id, result, source_key, confidence, &notes)
                .await
            {
                Ok(item) => {
                    let audit = json!({
                        "external_result_evidence_id": evidence_id,
                        "bet_id": item.id,
                        "event_name": row_event_name,
                        "outcome_name": outcome_name,
                        "result": result,
                        "status": item.status,
                        "source_key": source_key,
                        "source_url": source_url,
                        "score": {"home": home_score, "away": away_score},
                        "paper_only": true
                    });
                    self.record_audit("paper_bet_settled_from_external_evidence", audit.clone())
                        .await
                        .ok();
                    settled.push(audit);
                }
                Err(error) => skipped.push(json!({
                    "bet_id": bet_id,
                    "event_name": row_event_name,
                    "source_key": source_key,
                    "reason": "settlement_write_failed",
                    "error": error.to_string()
                })),
            }
        }
        if !settled.is_empty() {
            let client = self.connect().await?;
            client
                .execute(
                    "UPDATE external_result_evidence SET used_for_settlement = true WHERE id = $1",
                    &[&evidence_id],
                )
                .await?;
        }

        Ok(json!({
            "paper_only": true,
            "external_result_evidence_id": evidence_id,
            "source_key": source_key,
            "source_url": source_url,
            "event_name": event_name,
            "score": {"home": home_score, "away": away_score},
            "matched_bet_count": settled.len() + skipped.len(),
            "settled_count": settled.len(),
            "skipped_count": skipped.len(),
            "settled": settled,
            "skipped": skipped,
            "recorded_aliases": {
                "home": recorded_home_aliases,
                "away": recorded_away_aliases
            }
        }))
    }

    async fn ingest_account_history_settlement_evidence(
        &self,
        payload: &Value,
        source_key: &str,
        source_record: Value,
        source_url: Option<String>,
        title: Option<String>,
        settlement_result: &'static str,
    ) -> anyhow::Result<Value> {
        let event_name = payload
            .get("event_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                payload
                    .get("event_names")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .find(|value| !value.is_empty())
                    .map(str::to_string)
            })
            .or_else(|| {
                title
                    .as_deref()
                    .and_then(parse_score_title)
                    .map(|(home, away, _, _)| format!("{home} - {away}"))
            })
            .ok_or_else(|| anyhow!("event_name is required for account-history evidence"))?;
        let parsed_title = title.as_deref().and_then(parse_score_title);
        let (event_home, event_away) = event_name
            .split_once(" - ")
            .map(|(home, away)| (home.trim().to_string(), away.trim().to_string()))
            .unwrap_or_else(|| {
                parsed_title
                    .as_ref()
                    .map(|(home, away, _, _)| (home.clone(), away.clone()))
                    .unwrap_or_else(|| {
                        (
                            "bookmaker_account_history".to_string(),
                            "paper_selection".to_string(),
                        )
                    })
            });
        let home_name = payload
            .get("home_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or(event_home);
        let away_name = payload
            .get("away_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or(event_away);
        let home_score = json_i32(payload.get("home_score")).unwrap_or(0);
        let away_score = json_i32(payload.get("away_score")).unwrap_or(0);
        let confidence = payload
            .get("confidence")
            .and_then(Value::as_f64)
            .or_else(|| source_record.get("reliability").and_then(Value::as_f64))
            .unwrap_or(0.95)
            .clamp(0.0, 1.0);
        let settle = payload
            .get("settle")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let bet_id = payload
            .get("bet_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let coupon_id = payload
            .get("coupon_simulation_id")
            .or_else(|| payload.get("coupon_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if bet_id.is_none() && coupon_id.is_none() {
            return Err(anyhow!(
                "bet_id or coupon_simulation_id is required for account-history settlement evidence"
            ));
        }
        let result_status_raw = account_history_raw_status(payload);
        let event_names = account_history_event_names(payload, &event_name);
        let evidence_id = new_id();
        let evidence_payload = json!({
            "mode": "account_history_settlement_evidence",
            "source_key": source_key,
            "source_record": source_record,
            "source_url": source_url,
            "source_title": title,
            "event_name": event_name,
            "event_names": event_names,
            "home_name": home_name,
            "away_name": away_name,
            "home_score": home_score,
            "away_score": away_score,
            "score_available": payload.get("home_score").is_some() && payload.get("away_score").is_some(),
            "settlement_result": settlement_result,
            "result_status_raw": result_status_raw,
            "bet_id": bet_id,
            "coupon_simulation_id": coupon_id,
            "market_name": payload.get("market_name").cloned().unwrap_or(Value::Null),
            "outcome_name": payload.get("outcome_name").cloned().unwrap_or(Value::Null),
            "sport_key": payload.get("sport_key").cloned().unwrap_or(Value::Null),
            "gender_scope": payload.get("gender_scope").or_else(|| payload.get("gender")).cloned().unwrap_or(Value::Null),
            "browser_automation": payload.get("browser_automation").cloned().unwrap_or(Value::Null),
            "browser_session": payload.get("browser_session").cloned().unwrap_or(Value::Null),
            "raw_text_excerpt": payload.get("raw_text_excerpt").cloned().unwrap_or(Value::Null),
            "paper_only": true
        });
        let client = self.connect().await?;
        client
            .execute(
                r#"
                INSERT INTO external_result_evidence (
                  id, source_key, source_url, event_name, home_name, away_name,
                  home_score, away_score, confidence, payload
                )
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8,($9::float8)::numeric,$10)
                "#,
                &[
                    &evidence_id,
                    &source_key,
                    &source_url,
                    &event_name,
                    &home_name,
                    &away_name,
                    &home_score,
                    &away_score,
                    &confidence,
                    &evidence_payload,
                ],
            )
            .await?;
        drop(client);

        let mut settled = Vec::new();
        let mut skipped = Vec::new();
        if let Some(bet_id) = bet_id.as_deref() {
            if settle {
                let notes = json!({
                    "mode": "account_history_settlement_evidence",
                    "external_result_evidence_id": evidence_id,
                    "source_key": source_key,
                    "source_url": source_url,
                    "source_title": title,
                    "event_name": event_name,
                    "event_names": event_names,
                    "settlement_result": settlement_result,
                    "result_status_raw": result_status_raw,
                    "paper_only": true
                })
                .to_string();
                match self
                    .settle_simulated_bet(bet_id, settlement_result, source_key, confidence, &notes)
                    .await
                {
                    Ok(item) => {
                        let audit = json!({
                            "external_result_evidence_id": evidence_id,
                            "bet_id": item.id,
                            "event_name": event_name,
                            "result": settlement_result,
                            "status": item.status,
                            "source_key": source_key,
                            "source_url": source_url,
                            "paper_only": true
                        });
                        self.record_audit("paper_bet_settled_from_account_history", audit.clone())
                            .await
                            .ok();
                        settled.push(audit);
                    }
                    Err(error) => skipped.push(json!({
                        "bet_id": bet_id,
                        "source_key": source_key,
                        "reason": "settlement_write_failed",
                        "error": error.to_string()
                    })),
                }
            } else {
                skipped.push(json!({
                    "bet_id": bet_id,
                    "mapped_result": settlement_result,
                    "reason": "settle_false"
                }));
            }
        }
        if let Some(coupon_id) = coupon_id.as_deref() {
            if settle {
                let notes = json!({
                    "mode": "account_history_settlement_evidence",
                    "external_result_evidence_id": evidence_id,
                    "source_key": source_key,
                    "source_url": source_url,
                    "source_title": title,
                    "event_name": event_name,
                    "event_names": event_names,
                    "settlement_result": settlement_result,
                    "result_status_raw": result_status_raw,
                    "coupon_level": true,
                    "paper_only": true
                })
                .to_string();
                match self
                    .settle_simulated_coupon(
                        coupon_id,
                        settlement_result,
                        source_key,
                        confidence,
                        &notes,
                    )
                    .await
                {
                    Ok(item) => {
                        let status = item
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let audit = json!({
                            "external_result_evidence_id": evidence_id,
                            "coupon_simulation_id": coupon_id,
                            "event_name": event_name,
                            "result": settlement_result,
                            "status": status,
                            "source_key": source_key,
                            "source_url": source_url,
                            "coupon_level": true,
                            "paper_only": true
                        });
                        self.record_audit(
                            "paper_coupon_settled_from_account_history",
                            audit.clone(),
                        )
                        .await
                        .ok();
                        settled.push(audit);
                    }
                    Err(error) => skipped.push(json!({
                        "coupon_simulation_id": coupon_id,
                        "source_key": source_key,
                        "reason": "settlement_write_failed",
                        "error": error.to_string()
                    })),
                }
            } else {
                skipped.push(json!({
                    "coupon_simulation_id": coupon_id,
                    "mapped_result": settlement_result,
                    "reason": "settle_false"
                }));
            }
        }
        if !settled.is_empty() {
            let client = self.connect().await?;
            client
                .execute(
                    "UPDATE external_result_evidence SET used_for_settlement = true WHERE id = $1",
                    &[&evidence_id],
                )
                .await?;
        }

        Ok(json!({
            "paper_only": true,
            "mode": "account_history_settlement_evidence",
            "external_result_evidence_id": evidence_id,
            "source_key": source_key,
            "source_url": source_url,
            "event_name": event_name,
            "event_names": event_names,
            "score_available": payload.get("home_score").is_some() && payload.get("away_score").is_some(),
            "settlement_result": settlement_result,
            "result_status_raw": result_status_raw,
            "matched_item_count": settled.len() + skipped.len(),
            "settled_count": settled.len(),
            "skipped_count": skipped.len(),
            "settled": settled,
            "skipped": skipped
        }))
    }

    pub async fn simulated_bets(&self, limit: i64) -> anyhow::Result<Vec<SimulatedBet>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT sb.id, sb.candidate_id, sb.created_at,
                       cb.sport_key, cb.event_name, cb.competition, cb.market_name,
                       cb.market_kind, cb.outcome_name,
                       sb.hypothetical_stake::float8 AS hypothetical_stake,
                       sb.observed_decimal_odds::float8 AS observed_decimal_odds, sb.status,
                       sb.strategy_id, sb.event_start_time, sb.expected_result_check_after, sb.settled_at,
                       sb.simulated_return::float8 AS simulated_return,
                       sb.profit_loss::float8 AS profit_loss,
                       sb.settlement_payload, sb.payload
                FROM simulated_bets sb
                LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                ORDER BY sb.created_at DESC
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        Ok(rows.iter().map(simulated_bet_from_row).collect())
    }

    pub async fn ledger_summary(&self) -> anyhow::Result<LedgerSummary> {
        let bets = self.simulated_bets(1000).await?;
        let mut summary = LedgerSummary {
            count: bets.len(),
            open_count: 0,
            settled_count: 0,
            turnover: 0.0,
            open_exposure: 0.0,
            simulated_return: 0.0,
            profit_loss: 0.0,
            hit_rate: None,
            average_odds: None,
            by_status: BTreeMap::new(),
        };
        let mut won = 0usize;
        let mut decided = 0usize;
        let mut odds_total = 0.0;
        let mut odds_count = 0usize;
        for bet in &bets {
            *summary.by_status.entry(bet.status.clone()).or_default() += 1;
            if bet.status == "duplicate_void" {
                continue;
            }
            summary.turnover += bet.hypothetical_stake;
            summary.simulated_return += bet.simulated_return.unwrap_or_default();
            summary.profit_loss += bet.profit_loss.unwrap_or_default();
            if let Some(odds) = bet.observed_decimal_odds {
                odds_total += odds;
                odds_count += 1;
            }
            if is_open_settlement_status(&bet.status) {
                summary.open_count += 1;
                summary.open_exposure += bet.hypothetical_stake;
            }
            if is_closed_settlement_status(&bet.status) {
                summary.settled_count += 1;
            }
            if matches!(bet.status.as_str(), "settled_won" | "settled_lost") {
                decided += 1;
                if bet.status == "settled_won" {
                    won += 1;
                }
            }
        }
        let client = self.connect().await?;
        let coupon_rows = client
            .query(
                r#"
                SELECT hypothetical_stake::float8 AS hypothetical_stake,
                       observed_combined_decimal_odds::float8 AS observed_combined_decimal_odds,
                       status,
                       simulated_return::float8 AS simulated_return,
                       profit_loss::float8 AS profit_loss
                FROM simulated_coupons
                ORDER BY created_at DESC
                LIMIT 1000
                "#,
                &[],
            )
            .await?;
        summary.count += coupon_rows.len();
        for row in coupon_rows {
            let status: String = row.get("status");
            *summary.by_status.entry(status.clone()).or_default() += 1;
            if status == "duplicate_void" {
                continue;
            }
            let stake: f64 = row.get("hypothetical_stake");
            summary.turnover += stake;
            summary.simulated_return += row
                .get::<_, Option<f64>>("simulated_return")
                .unwrap_or_default();
            summary.profit_loss += row.get::<_, Option<f64>>("profit_loss").unwrap_or_default();
            if let Some(odds) = row.get::<_, Option<f64>>("observed_combined_decimal_odds") {
                odds_total += odds;
                odds_count += 1;
            }
            if is_open_settlement_status(&status) {
                summary.open_count += 1;
                summary.open_exposure += stake;
            }
            if is_closed_settlement_status(&status) {
                summary.settled_count += 1;
            }
            if matches!(status.as_str(), "settled_won" | "settled_lost") {
                decided += 1;
                if status == "settled_won" {
                    won += 1;
                }
            }
        }
        if decided > 0 {
            summary.hit_rate = Some(won as f64 / decided as f64);
        }
        if odds_count > 0 {
            summary.average_odds = Some(odds_total / odds_count as f64);
        }
        Ok(summary)
    }

    pub async fn paper_performance_today(&self) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let row = client
            .query_one(
                "SELECT (now() AT TIME ZONE 'Europe/Copenhagen')::date AS local_date",
                &[],
            )
            .await?;
        let local_date: NaiveDate = row.get("local_date");
        self.paper_performance_for_local_date(local_date).await
    }

    pub async fn paper_performance_yesterday(&self) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let row = client
            .query_one(
                "SELECT ((now() AT TIME ZONE 'Europe/Copenhagen')::date - 1) AS local_date",
                &[],
            )
            .await?;
        let local_date: NaiveDate = row.get("local_date");
        self.paper_performance_for_local_date(local_date).await
    }

    pub async fn paper_performance_for_local_date(
        &self,
        local_date_value: NaiveDate,
    ) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let window = client
            .query_one(
                r#"
                SELECT
                  $1::date::text AS local_date,
                  ($1::date::timestamp AT TIME ZONE 'Europe/Copenhagen') AS window_start,
                  (($1::date + interval '1 day')::timestamp AT TIME ZONE 'Europe/Copenhagen') AS window_end
                "#,
                &[&local_date_value],
            )
            .await?;
        let local_date: String = window.get("local_date");
        let window_start: DateTime<Utc> = window.get("window_start");
        let window_end: DateTime<Utc> = window.get("window_end");

        let summary_row = client
            .query_one(
                r#"
                WITH coupon_sports AS (
                  SELECT
                    sc.id,
                    CASE
                      WHEN count(DISTINCT cb.sport_key) = 1 THEN min(cb.sport_key)
                      ELSE 'mixed'
                    END AS sport_key
                  FROM simulated_coupons sc
                  LEFT JOIN simulated_coupon_legs scl ON scl.simulated_coupon_id = sc.id
                  LEFT JOIN candidate_bets cb ON cb.id = scl.candidate_id
                  GROUP BY sc.id
                ),
                positions AS (
                  SELECT
                    'single'::text AS item_type,
                    sb.id::text AS item_id,
                    sb.status,
                    sb.hypothetical_stake::float8 AS stake,
                    sb.observed_decimal_odds::float8 AS odds,
                    sb.profit_loss::float8 AS profit_loss
                  FROM simulated_bets sb
                  WHERE sb.created_at >= $1 AND sb.created_at < $2
                  UNION ALL
                  SELECT
                    'coupon'::text AS item_type,
                    sc.id::text AS item_id,
                    sc.status,
                    sc.hypothetical_stake::float8 AS stake,
                    sc.observed_combined_decimal_odds::float8 AS odds,
                    sc.profit_loss::float8 AS profit_loss
                  FROM simulated_coupons sc
                  LEFT JOIN coupon_sports cs ON cs.id = sc.id
                  WHERE sc.created_at >= $1 AND sc.created_at < $2
                ),
                positions_with_truth AS (
                  SELECT
                    positions.*,
                    observation.id AS latest_observation_id
                  FROM positions
                  LEFT JOIN LATERAL (
                    SELECT so.id
                    FROM settlement_observations so
                    WHERE
                      (positions.item_type = 'single' AND so.simulated_bet_id = positions.item_id)
                      OR (positions.item_type = 'coupon' AND so.simulated_coupon_id = positions.item_id)
                    ORDER BY so.created_at DESC
                    LIMIT 1
                  ) observation ON true
                )
                SELECT
                  count(*) FILTER (WHERE status <> 'duplicate_void')::int AS placed_count,
                  count(*) FILTER (WHERE item_type = 'single' AND status <> 'duplicate_void')::int AS single_count,
                  count(*) FILTER (WHERE item_type = 'coupon' AND status <> 'duplicate_void')::int AS coupon_count,
                  count(*) FILTER (WHERE status LIKE 'settled_%' OR status IN ('void', 'pushed', 'cancelled', 'abandoned', 'refunded'))::int AS settled_count,
                  count(*) FILTER (WHERE status = 'settled_won')::int AS won_count,
                  count(*) FILTER (WHERE status = 'settled_lost')::int AS lost_count,
                  count(*) FILTER (WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed'))::int AS open_count,
                  count(*) FILTER (WHERE status = 'awaiting_result')::int AS awaiting_result_count,
                  count(*) FILTER (WHERE status <> 'duplicate_void' AND latest_observation_id IS NOT NULL)::int AS truth_observation_count,
                  COALESCE(sum(stake) FILTER (WHERE status <> 'duplicate_void'), 0)::float8 AS turnover,
                  COALESCE(sum(stake) FILTER (WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed')), 0)::float8 AS open_exposure,
                  COALESCE(sum(stake) FILTER (WHERE status = 'awaiting_result'), 0)::float8 AS awaiting_result_exposure,
                  COALESCE(sum(profit_loss) FILTER (WHERE status <> 'duplicate_void'), 0)::float8 AS realized_profit_loss,
                  avg(odds) FILTER (WHERE status <> 'duplicate_void')::float8 AS average_odds
                FROM positions_with_truth
                "#,
                &[&window_start, &window_end],
            )
            .await?;
        let won_count: i32 = summary_row.get("won_count");
        let lost_count: i32 = summary_row.get("lost_count");
        let decided_count = won_count + lost_count;
        let hit_rate = if decided_count > 0 {
            Some(won_count as f64 / decided_count as f64)
        } else {
            None
        };

        let sport_rows = client
            .query(
                r#"
                WITH coupon_sports AS (
                  SELECT
                    sc.id,
                    CASE
                      WHEN count(DISTINCT cb.sport_key) = 1 THEN min(cb.sport_key)
                      ELSE 'mixed'
                    END AS sport_key
                  FROM simulated_coupons sc
                  LEFT JOIN simulated_coupon_legs scl ON scl.simulated_coupon_id = sc.id
                  LEFT JOIN candidate_bets cb ON cb.id = scl.candidate_id
                  GROUP BY sc.id
                ),
                positions AS (
                  SELECT
                    COALESCE(cb.sport_key, 'unknown') AS sport_key,
                    'single'::text AS item_type,
                    sb.id::text AS item_id,
                    sb.status,
                    sb.hypothetical_stake::float8 AS stake,
                    sb.observed_decimal_odds::float8 AS odds,
                    sb.profit_loss::float8 AS profit_loss
                  FROM simulated_bets sb
                  LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                  WHERE sb.created_at >= $1 AND sb.created_at < $2
                  UNION ALL
                  SELECT
                    COALESCE(cs.sport_key, 'unknown') AS sport_key,
                    'coupon'::text AS item_type,
                    sc.id::text AS item_id,
                    sc.status,
                    sc.hypothetical_stake::float8 AS stake,
                    sc.observed_combined_decimal_odds::float8 AS odds,
                    sc.profit_loss::float8 AS profit_loss
                  FROM simulated_coupons sc
                  LEFT JOIN coupon_sports cs ON cs.id = sc.id
                  WHERE sc.created_at >= $1 AND sc.created_at < $2
                ),
                positions_with_truth AS (
                  SELECT
                    positions.*,
                    observation.id AS latest_observation_id
                  FROM positions
                  LEFT JOIN LATERAL (
                    SELECT so.id
                    FROM settlement_observations so
                    WHERE
                      (positions.item_type = 'single' AND so.simulated_bet_id = positions.item_id)
                      OR (positions.item_type = 'coupon' AND so.simulated_coupon_id = positions.item_id)
                    ORDER BY so.created_at DESC
                    LIMIT 1
                  ) observation ON true
                )
                SELECT
                  sport_key,
                  count(*) FILTER (WHERE status <> 'duplicate_void')::int AS placed_count,
                  count(*) FILTER (WHERE item_type = 'single' AND status <> 'duplicate_void')::int AS single_count,
                  count(*) FILTER (WHERE item_type = 'coupon' AND status <> 'duplicate_void')::int AS coupon_count,
                  count(*) FILTER (WHERE status LIKE 'settled_%' OR status IN ('void', 'pushed', 'cancelled', 'abandoned', 'refunded'))::int AS settled_count,
                  count(*) FILTER (WHERE status = 'settled_won')::int AS won_count,
                  count(*) FILTER (WHERE status = 'settled_lost')::int AS lost_count,
                  count(*) FILTER (WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed'))::int AS open_count,
                  count(*) FILTER (WHERE status = 'awaiting_result')::int AS awaiting_result_count,
                  count(*) FILTER (WHERE status <> 'duplicate_void' AND latest_observation_id IS NOT NULL)::int AS truth_observation_count,
                  COALESCE(sum(stake) FILTER (WHERE status <> 'duplicate_void'), 0)::float8 AS turnover,
                  COALESCE(sum(stake) FILTER (WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed')), 0)::float8 AS open_exposure,
                  COALESCE(sum(stake) FILTER (WHERE status = 'awaiting_result'), 0)::float8 AS awaiting_result_exposure,
                  COALESCE(sum(profit_loss) FILTER (WHERE status <> 'duplicate_void'), 0)::float8 AS realized_profit_loss,
                  avg(odds) FILTER (WHERE status <> 'duplicate_void')::float8 AS average_odds
                FROM positions_with_truth
                GROUP BY sport_key
                ORDER BY placed_count DESC, sport_key
                "#,
                &[&window_start, &window_end],
            )
            .await?;
        let by_sport: Vec<Value> = sport_rows
            .iter()
            .map(|row| {
                let won: i32 = row.get("won_count");
                let lost: i32 = row.get("lost_count");
                let decided = won + lost;
                json!({
                    "sport_key": row.get::<_, String>("sport_key"),
                    "placed_count": row.get::<_, i32>("placed_count"),
                    "single_count": row.get::<_, i32>("single_count"),
                    "coupon_count": row.get::<_, i32>("coupon_count"),
                    "settled_count": row.get::<_, i32>("settled_count"),
                    "won_count": won,
                    "lost_count": lost,
                    "open_count": row.get::<_, i32>("open_count"),
                    "awaiting_result_count": row.get::<_, i32>("awaiting_result_count"),
                    "truth_observation_count": row.get::<_, i32>("truth_observation_count"),
                    "turnover": row.get::<_, f64>("turnover"),
                    "open_exposure": row.get::<_, f64>("open_exposure"),
                    "awaiting_result_exposure": row.get::<_, f64>("awaiting_result_exposure"),
                    "realized_profit_loss": row.get::<_, f64>("realized_profit_loss"),
                    "hit_rate": if decided > 0 { Some(won as f64 / decided as f64) } else { None },
                    "average_odds": row.get::<_, Option<f64>>("average_odds")
                })
            })
            .collect();

        let recent_rows = client
            .query(
                r#"
                WITH coupon_details AS (
                  SELECT
                    sc.id,
                    CASE
                      WHEN count(DISTINCT cb.sport_key) = 1 THEN min(cb.sport_key)
                      ELSE 'mixed'
                    END AS sport_key,
                    min(cb.event_name) AS event_name,
                    min(cb.competition) AS competition,
                    min(cb.market_name) AS market_name,
                    min(cb.market_kind) AS market_kind,
                    string_agg(cb.outcome_name, ' + ' ORDER BY scl.leg_index) AS outcome_name,
                    jsonb_agg(
                      jsonb_build_object(
                        'event_name', cb.event_name,
                        'competition', cb.competition,
                        'market_name', cb.market_name,
                        'market_kind', cb.market_kind,
                        'outcome_name', cb.outcome_name,
                        'sport_key', cb.sport_key
                      )
                      ORDER BY scl.leg_index
                    ) FILTER (WHERE cb.id IS NOT NULL) AS legs
                  FROM simulated_coupons sc
                  LEFT JOIN simulated_coupon_legs scl ON scl.simulated_coupon_id = sc.id
                  LEFT JOIN candidate_bets cb ON cb.id = scl.candidate_id
                  GROUP BY sc.id
                ),
                positions AS (
                  SELECT
                    'single'::text AS item_type,
                    sb.id::text AS item_id,
                    sb.created_at,
                    COALESCE(cb.sport_key, 'unknown') AS sport_key,
                    cb.event_name,
                    cb.competition,
                    cb.market_name,
                    cb.market_kind,
                    cb.outcome_name,
                    sb.hypothetical_stake::float8 AS stake,
                    sb.observed_decimal_odds::float8 AS odds,
                    sb.status,
                    sb.profit_loss::float8 AS profit_loss,
                    sb.expected_result_check_after,
                    '[]'::jsonb AS legs
                  FROM simulated_bets sb
                  LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                  WHERE sb.created_at >= $1 AND sb.created_at < $2
                  UNION ALL
                  SELECT
                    'coupon'::text AS item_type,
                    sc.id::text AS item_id,
                    sc.created_at,
                    COALESCE(cd.sport_key, 'unknown') AS sport_key,
                    cd.event_name,
                    cd.competition,
                    cd.market_name,
                    cd.market_kind,
                    cd.outcome_name,
                    sc.hypothetical_stake::float8 AS stake,
                    sc.observed_combined_decimal_odds::float8 AS odds,
                    sc.status,
                    sc.profit_loss::float8 AS profit_loss,
                    sc.expected_result_check_after,
                    COALESCE(cd.legs, '[]'::jsonb) AS legs
                  FROM simulated_coupons sc
                  LEFT JOIN coupon_details cd ON cd.id = sc.id
                  WHERE sc.created_at >= $1 AND sc.created_at < $2
                ),
                latest_lookup AS (
                  SELECT
                    positions.*,
                    lookup.created_at AS last_lookup_at,
                    lookup.source_key AS last_lookup_source_key,
                    lookup.recommendation AS last_lookup_recommendation,
                    observation.created_at AS latest_observation_at,
                    observation.source AS latest_observation_source,
                    observation.observed_result AS latest_observation_result,
                    observation.confidence::float8 AS latest_observation_confidence
                  FROM positions
                  LEFT JOIN LATERAL (
                    SELECT
                      sla.created_at,
                      sla.source_key,
                      sla.recommendation
                    FROM settlement_lookup_attempts sla
                    WHERE
                      (positions.item_type = 'single' AND sla.simulated_bet_id = positions.item_id)
                      OR (positions.item_type = 'coupon' AND sla.simulated_coupon_id = positions.item_id)
                    ORDER BY sla.created_at DESC
                    LIMIT 1
                  ) lookup ON true
                  LEFT JOIN LATERAL (
                    SELECT
                      so.created_at,
                      so.source,
                      so.observed_result,
                      so.confidence
                    FROM settlement_observations so
                    WHERE
                      (positions.item_type = 'single' AND so.simulated_bet_id = positions.item_id)
                      OR (positions.item_type = 'coupon' AND so.simulated_coupon_id = positions.item_id)
                    ORDER BY so.created_at DESC
                    LIMIT 1
                  ) observation ON true
                )
                SELECT *
                FROM latest_lookup
                WHERE status <> 'duplicate_void'
                ORDER BY created_at DESC
                LIMIT 25
                "#,
                &[&window_start, &window_end],
            )
            .await?;
        let recent: Vec<Value> = recent_rows
            .iter()
            .map(|row| {
                let created_at: DateTime<Utc> = row.get("created_at");
                let expected_result_check_after: Option<DateTime<Utc>> =
                    row.get("expected_result_check_after");
                let last_lookup_at: Option<DateTime<Utc>> = row.get("last_lookup_at");
                let last_lookup_source_key: Option<String> = row.get("last_lookup_source_key");
                let last_lookup_recommendation: Option<String> =
                    row.get("last_lookup_recommendation");
                let latest_observation_at: Option<DateTime<Utc>> =
                    row.get("latest_observation_at");
                let latest_observation_source: Option<String> =
                    row.get("latest_observation_source");
                let latest_observation_result: Option<String> =
                    row.get("latest_observation_result");
                let latest_observation_confidence: Option<f64> =
                    row.get("latest_observation_confidence");
                let overdue_minutes = expected_result_check_after
                    .filter(|expected| Utc::now() > *expected)
                    .map(|expected| (Utc::now() - expected).num_minutes());
                json!({
                    "item_type": row.get::<_, String>("item_type"),
                    "item_id": row.get::<_, String>("item_id"),
                    "created_at": created_at.to_rfc3339(),
                    "sport_key": row.get::<_, String>("sport_key"),
                    "event_name": row.get::<_, Option<String>>("event_name"),
                    "competition": row.get::<_, Option<String>>("competition"),
                    "market_name": row.get::<_, Option<String>>("market_name"),
                    "market_kind": row.get::<_, Option<String>>("market_kind"),
                    "outcome_name": row.get::<_, Option<String>>("outcome_name"),
                    "stake": row.get::<_, f64>("stake"),
                    "observed_odds": row.get::<_, Option<f64>>("odds"),
                    "status": row.get::<_, String>("status"),
                    "profit_loss": row.get::<_, Option<f64>>("profit_loss"),
                    "expected_result_check_after": expected_result_check_after.map(|value| value.to_rfc3339()),
                    "last_lookup_at": last_lookup_at.map(|value| value.to_rfc3339()),
                    "last_lookup_source_key": last_lookup_source_key,
                    "last_lookup_recommendation": last_lookup_recommendation,
                    "latest_observation_at": latest_observation_at.map(|value| value.to_rfc3339()),
                    "latest_observation_source": latest_observation_source,
                    "latest_observation_result": latest_observation_result,
                    "latest_observation_confidence": latest_observation_confidence,
                    "overdue_minutes": overdue_minutes,
                    "legs": row.get::<_, Value>("legs")
                })
            })
            .collect();

        let observation_rows = client
            .query(
                r#"
                SELECT observed_result, count(*)::int AS count
                FROM settlement_observations
                WHERE created_at >= $1 AND created_at < $2
                GROUP BY observed_result
                ORDER BY count DESC, observed_result
                "#,
                &[&window_start, &window_end],
            )
            .await?;
        let observations: Vec<Value> = observation_rows
            .iter()
            .map(|row| {
                json!({
                    "observed_result": row.get::<_, String>("observed_result"),
                    "count": row.get::<_, i32>("count")
                })
            })
            .collect();

        Ok(json!({
            "paper_only": true,
            "timezone": "Europe/Copenhagen",
            "local_date": local_date,
            "window": {
                "start": window_start.to_rfc3339(),
                "end": window_end.to_rfc3339()
            },
            "summary": {
                "placed_count": summary_row.get::<_, i32>("placed_count"),
                "single_count": summary_row.get::<_, i32>("single_count"),
                "coupon_count": summary_row.get::<_, i32>("coupon_count"),
                "settled_count": summary_row.get::<_, i32>("settled_count"),
                "won_count": won_count,
                "lost_count": lost_count,
                "open_count": summary_row.get::<_, i32>("open_count"),
                "awaiting_result_count": summary_row.get::<_, i32>("awaiting_result_count"),
                "truth_observation_count": summary_row.get::<_, i32>("truth_observation_count"),
                "turnover": summary_row.get::<_, f64>("turnover"),
                "open_exposure": summary_row.get::<_, f64>("open_exposure"),
                "awaiting_result_exposure": summary_row.get::<_, f64>("awaiting_result_exposure"),
                "realized_profit_loss": summary_row.get::<_, f64>("realized_profit_loss"),
                "hit_rate": hit_rate,
                "average_odds": summary_row.get::<_, Option<f64>>("average_odds")
            },
            "by_sport": by_sport,
            "recent": recent,
            "settlement_observations": {
                "items": observations
            }
        }))
    }

    pub async fn strategy_played_summary(&self) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let by_strategy = client
            .query(
                r#"
                WITH positions AS (
                  SELECT
                    'single'::text AS item_type,
                    sb.strategy_id,
                    sb.status,
                    sb.hypothetical_stake,
                    sb.profit_loss
                  FROM simulated_bets sb
                  UNION ALL
                  SELECT
                    'coupon'::text AS item_type,
                    sc.strategy_id,
                    sc.status,
                    sc.hypothetical_stake,
                    sc.profit_loss
                  FROM simulated_coupons sc
                )
                SELECT
                  strategy_id,
                  count(*) FILTER (WHERE status <> 'duplicate_void')::int AS played_count,
                  count(*) FILTER (WHERE item_type = 'single' AND status <> 'duplicate_void')::int AS single_count,
                  count(*) FILTER (WHERE item_type = 'coupon' AND status <> 'duplicate_void')::int AS coupon_count,
                  count(*) FILTER (WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed'))::int AS open_count,
                  count(*) FILTER (WHERE status = 'awaiting_result')::int AS awaiting_result_count,
                  count(*) FILTER (WHERE status = 'duplicate_void')::int AS duplicate_void_count,
                  COALESCE(sum(hypothetical_stake) FILTER (WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed')), 0)::float8 AS open_exposure,
                  COALESCE(sum(hypothetical_stake) FILTER (WHERE status = 'awaiting_result'), 0)::float8 AS awaiting_result_exposure,
                  COALESCE(sum(profit_loss) FILTER (WHERE status <> 'duplicate_void'), 0)::float8 AS profit_loss
                FROM positions
                GROUP BY strategy_id
                ORDER BY played_count DESC, strategy_id
                "#,
                &[],
            )
            .await?;
        let by_sport = client
            .query(
                r#"
                WITH coupon_sports AS (
                  SELECT
                    sc.id,
                    CASE
                      WHEN count(DISTINCT cb.sport_key) = 1 THEN min(cb.sport_key)
                      ELSE 'mixed'
                    END AS sport_key
                  FROM simulated_coupons sc
                  LEFT JOIN simulated_coupon_legs scl ON scl.simulated_coupon_id = sc.id
                  LEFT JOIN candidate_bets cb ON cb.id = scl.candidate_id
                  GROUP BY sc.id
                ),
                positions AS (
                  SELECT
                    COALESCE(cb.sport_key, 'unknown') AS sport_key,
                    sb.status,
                    sb.hypothetical_stake
                  FROM simulated_bets sb
                  LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                  UNION ALL
                  SELECT
                    COALESCE(cs.sport_key, 'unknown') AS sport_key,
                    sc.status,
                    sc.hypothetical_stake
                  FROM simulated_coupons sc
                  LEFT JOIN coupon_sports cs ON cs.id = sc.id
                )
                SELECT
                  sport_key,
                  status,
                  count(*)::int AS count,
                  COALESCE(sum(hypothetical_stake), 0)::float8 AS stake
                FROM positions
                WHERE status <> 'duplicate_void'
                GROUP BY sport_key, status
                ORDER BY sport_key, status
                "#,
                &[],
            )
            .await?;
        let by_risk_flag = client
            .query(
                r#"
                WITH single_flags AS (
                  SELECT
                    'single'::text AS item_type,
                    sb.id AS item_id,
                    sb.status,
                    sb.hypothetical_stake,
                    sb.profit_loss,
                    flags.risk_flag
                  FROM simulated_bets sb
                  LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                  LEFT JOIN LATERAL (
                    SELECT value AS risk_flag
                    FROM jsonb_array_elements_text(COALESCE(cb.risk_flags, '[]'::jsonb)) AS flag(value)
                    UNION ALL
                    SELECT 'none'
                    WHERE jsonb_array_length(COALESCE(cb.risk_flags, '[]'::jsonb)) = 0
                  ) flags ON true
                ),
                coupon_flags AS (
                  SELECT
                    'coupon'::text AS item_type,
                    sc.id AS item_id,
                    sc.status,
                    sc.hypothetical_stake,
                    sc.profit_loss,
                    flags.risk_flag
                  FROM simulated_coupons sc
                  LEFT JOIN LATERAL (
                    SELECT DISTINCT flag.value AS risk_flag
                    FROM simulated_coupon_legs scl
                    JOIN candidate_bets cb ON cb.id = scl.candidate_id
                    CROSS JOIN LATERAL jsonb_array_elements_text(COALESCE(cb.risk_flags, '[]'::jsonb)) AS flag(value)
                    WHERE scl.simulated_coupon_id = sc.id
                    UNION ALL
                    SELECT 'none'
                    WHERE NOT EXISTS (
                      SELECT 1
                      FROM simulated_coupon_legs scl
                      JOIN candidate_bets cb ON cb.id = scl.candidate_id
                      CROSS JOIN LATERAL jsonb_array_elements_text(COALESCE(cb.risk_flags, '[]'::jsonb)) AS flag(value)
                      WHERE scl.simulated_coupon_id = sc.id
                    )
                  ) flags ON true
                ),
                positions AS (
                  SELECT * FROM single_flags
                  UNION ALL
                  SELECT * FROM coupon_flags
                )
                SELECT
                  risk_flag,
                  count(*) FILTER (WHERE status <> 'duplicate_void')::int AS played_count,
                  count(*) FILTER (WHERE item_type = 'single' AND status <> 'duplicate_void')::int AS single_count,
                  count(*) FILTER (WHERE item_type = 'coupon' AND status <> 'duplicate_void')::int AS coupon_count,
                  count(*) FILTER (WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed'))::int AS open_count,
                  count(*) FILTER (WHERE status = 'awaiting_result')::int AS awaiting_result_count,
                  count(*) FILTER (WHERE status IN ('settled_won', 'settled_lost'))::int AS decided_count,
                  count(*) FILTER (WHERE status = 'settled_won')::int AS won_count,
                  COALESCE(sum(hypothetical_stake) FILTER (WHERE status <> 'duplicate_void'), 0)::float8 AS turnover,
                  COALESCE(sum(hypothetical_stake) FILTER (WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed')), 0)::float8 AS open_exposure,
                  COALESCE(sum(hypothetical_stake) FILTER (WHERE status = 'awaiting_result'), 0)::float8 AS awaiting_result_exposure,
                  COALESCE(sum(profit_loss) FILTER (WHERE status <> 'duplicate_void'), 0)::float8 AS profit_loss
                FROM positions
                GROUP BY risk_flag
                ORDER BY played_count DESC, risk_flag
                "#,
                &[],
            )
            .await?;
        let recent = client
            .query(
                r#"
                WITH coupon_sports AS (
                  SELECT
                    sc.id,
                    CASE
                      WHEN count(DISTINCT cb.sport_key) = 1 THEN min(cb.sport_key)
                      ELSE 'mixed'
                    END AS sport_key,
                    string_agg(cb.event_name, ' + ' ORDER BY scl.leg_index) AS event_name,
                    min(cb.competition) AS competition
                  FROM simulated_coupons sc
                  LEFT JOIN simulated_coupon_legs scl ON scl.simulated_coupon_id = sc.id
                  LEFT JOIN candidate_bets cb ON cb.id = scl.candidate_id
                  GROUP BY sc.id
                ),
                recent_positions AS (
                  SELECT
                    'single'::text AS item_type,
                    sb.id,
                    sb.created_at,
                    sb.strategy_id,
                    sb.status,
                    sb.hypothetical_stake::float8 AS hypothetical_stake,
                    sb.observed_decimal_odds::float8 AS observed_decimal_odds,
                    cb.sport_key,
                    cb.event_name,
                    cb.competition,
                    cb.market_kind,
                    cb.market_name,
                    cb.outcome_name,
                    cb.score::float8 AS score,
                    cb.confidence::float8 AS confidence
                  FROM simulated_bets sb
                  LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                  UNION ALL
                  SELECT
                    'coupon'::text AS item_type,
                    sc.id,
                    sc.created_at,
                    sc.strategy_id,
                    sc.status,
                    sc.hypothetical_stake::float8 AS hypothetical_stake,
                    sc.observed_combined_decimal_odds::float8 AS observed_decimal_odds,
                    cs.sport_key,
                    COALESCE(cs.event_name, cc.coupon_type || ' coupon') AS event_name,
                    cs.competition,
                    cc.coupon_type AS market_kind,
                    cc.coupon_type || ' coupon' AS market_name,
                    cc.leg_count::text || ' legs' AS outcome_name,
                    cc.score::float8 AS score,
                    cc.confidence::float8 AS confidence
                  FROM simulated_coupons sc
                  LEFT JOIN candidate_coupons cc ON cc.id = sc.coupon_id
                  LEFT JOIN coupon_sports cs ON cs.id = sc.id
                )
                SELECT
                  item_type,
                  id,
                  created_at,
                  strategy_id,
                  status,
                  hypothetical_stake,
                  observed_decimal_odds,
                  sport_key,
                  event_name,
                  competition,
                  market_kind,
                  market_name,
                  outcome_name,
                  score,
                  confidence
                FROM recent_positions
                ORDER BY created_at DESC
                LIMIT 25
                "#,
                &[],
            )
            .await?;
        Ok(json!({
            "by_strategy": by_strategy.iter().map(|row| json!({
                "strategy_id": row.get::<_, String>("strategy_id"),
                "played_count": row.get::<_, i32>("played_count"),
                "single_count": row.get::<_, i32>("single_count"),
                "coupon_count": row.get::<_, i32>("coupon_count"),
                "open_count": row.get::<_, i32>("open_count"),
                "awaiting_result_count": row.get::<_, i32>("awaiting_result_count"),
                "duplicate_void_count": row.get::<_, i32>("duplicate_void_count"),
                "open_exposure": row.get::<_, f64>("open_exposure"),
                "awaiting_result_exposure": row.get::<_, f64>("awaiting_result_exposure"),
                "profit_loss": row.get::<_, f64>("profit_loss")
            })).collect::<Vec<_>>(),
            "by_sport_status": by_sport.iter().map(|row| json!({
                "sport_key": row.get::<_, Option<String>>("sport_key"),
                "status": row.get::<_, String>("status"),
                "count": row.get::<_, i32>("count"),
                "stake": row.get::<_, f64>("stake")
            })).collect::<Vec<_>>(),
            "by_risk_flag": by_risk_flag.iter().map(|row| {
                let decided_count: i32 = row.get("decided_count");
                let won_count: i32 = row.get("won_count");
                json!({
                    "risk_flag": row.get::<_, String>("risk_flag"),
                    "played_count": row.get::<_, i32>("played_count"),
                    "single_count": row.get::<_, i32>("single_count"),
                    "coupon_count": row.get::<_, i32>("coupon_count"),
                    "open_count": row.get::<_, i32>("open_count"),
                    "awaiting_result_count": row.get::<_, i32>("awaiting_result_count"),
                    "decided_count": decided_count,
                    "won_count": won_count,
                    "hit_rate": if decided_count > 0 { Some(won_count as f64 / decided_count as f64) } else { None },
                    "turnover": row.get::<_, f64>("turnover"),
                    "open_exposure": row.get::<_, f64>("open_exposure"),
                    "awaiting_result_exposure": row.get::<_, f64>("awaiting_result_exposure"),
                    "profit_loss": row.get::<_, f64>("profit_loss")
                })
            }).collect::<Vec<_>>(),
            "recent": recent.iter().map(|row| {
                let created_at: DateTime<Utc> = row.get("created_at");
                json!({
                    "item_type": row.get::<_, String>("item_type"),
                    "id": row.get::<_, String>("id"),
                    "created_at": created_at,
                    "strategy_id": row.get::<_, String>("strategy_id"),
                    "status": row.get::<_, String>("status"),
                    "hypothetical_stake": row.get::<_, f64>("hypothetical_stake"),
                    "observed_decimal_odds": row.get::<_, Option<f64>>("observed_decimal_odds"),
                    "sport_key": row.get::<_, Option<String>>("sport_key"),
                    "event_name": row.get::<_, Option<String>>("event_name"),
                    "competition": row.get::<_, Option<String>>("competition"),
                    "market_kind": row.get::<_, Option<String>>("market_kind"),
                    "market_name": row.get::<_, Option<String>>("market_name"),
                    "outcome_name": row.get::<_, Option<String>>("outcome_name"),
                    "score": row.get::<_, Option<f64>>("score"),
                    "confidence": row.get::<_, Option<f64>>("confidence")
                })
            }).collect::<Vec<_>>(),
            "paper_only": true
        }))
    }

    pub async fn performance_report(
        &self,
        default_stake: f64,
        per_scan_limit: usize,
        max_open_exposure: f64,
        lookup_cooldown_minutes: i64,
    ) -> anyhow::Result<Value> {
        let ledger = self.ledger_summary().await?;
        let played = self.strategy_played_summary().await?;
        let client = self.connect().await?;

        let latest_snapshot = client
            .query_opt(
                r#"
                SELECT id, observed_at, event_count
                FROM odds_snapshots
                ORDER BY observed_at DESC
                LIMIT 1
                "#,
                &[],
            )
            .await?;

        let candidate_status_rows = client
            .query(
                r#"
                WITH latest AS (
                  SELECT id FROM odds_snapshots ORDER BY observed_at DESC LIMIT 1
                )
                SELECT
                  cb.status,
                  count(*)::int AS count,
                  avg(cb.score)::float8 AS average_score,
                  avg(cb.confidence)::float8 AS average_confidence
                FROM candidate_bets cb
                JOIN latest ON latest.id = cb.snapshot_id
                GROUP BY cb.status
                ORDER BY cb.status
                "#,
                &[],
            )
            .await?;

        let selected_unplaced_rows = client
            .query(
                r#"
                WITH latest AS (
                  SELECT id FROM odds_snapshots ORDER BY observed_at DESC LIMIT 1
                )
                SELECT
                  (count(*) OVER ())::int AS selected_unplaced_total_count,
                  cb.id,
                  cb.sport_key,
                  cb.event_name,
                  cb.competition,
                  cb.market_kind,
                  cb.market_name,
                  cb.outcome_name,
                  cb.decimal_odds::float8 AS decimal_odds,
                  cb.score::float8 AS score,
                  cb.confidence::float8 AS confidence,
                  cb.event_id,
                  cb.market_id,
                  cb.outcome_id
                FROM candidate_bets cb
                JOIN latest ON latest.id = cb.snapshot_id
                WHERE cb.status = 'selected'
                  AND NOT EXISTS (
                    SELECT 1
                    FROM simulated_bets sb
                    WHERE sb.candidate_id = cb.id
                      AND sb.status <> 'duplicate_void'
                  )
                  AND NOT EXISTS (
                    SELECT 1
                    FROM simulated_bets sb
                    JOIN candidate_bets existing ON existing.id = sb.candidate_id
                    WHERE existing.event_id = cb.event_id
                      AND existing.market_id = cb.market_id
                      AND existing.outcome_id = cb.outcome_id
                      AND sb.status <> 'duplicate_void'
                  )
                ORDER BY cb.score DESC NULLS LAST, cb.created_at ASC
                LIMIT 20
                "#,
                &[],
            )
            .await?;

        let due_rows = client
            .query(
                r#"
                SELECT
                  count(*) FILTER (
                    WHERE status IN ('awaiting_result', 'unresolved', 'postponed')
                      AND expected_result_check_after <= now()
                  )::int AS due_single_count,
                  COALESCE(sum(hypothetical_stake) FILTER (
                    WHERE status IN ('awaiting_result', 'unresolved', 'postponed')
                      AND expected_result_check_after <= now()
                  ), 0)::float8 AS due_single_exposure,
                  min(expected_result_check_after) FILTER (
                    WHERE status IN ('awaiting_result', 'unresolved', 'postponed')
                      AND expected_result_check_after <= now()
                  ) AS oldest_due_single
                FROM simulated_bets
                "#,
                &[],
            )
            .await?;
        let coupon_due_rows = client
            .query(
                r#"
                SELECT
                  count(*) FILTER (
                    WHERE status IN ('awaiting_result', 'unresolved', 'postponed')
                      AND expected_result_check_after <= now()
                  )::int AS due_coupon_count,
                  COALESCE(sum(hypothetical_stake) FILTER (
                    WHERE status IN ('awaiting_result', 'unresolved', 'postponed')
                      AND expected_result_check_after <= now()
                  ), 0)::float8 AS due_coupon_exposure,
                  min(expected_result_check_after) FILTER (
                    WHERE status IN ('awaiting_result', 'unresolved', 'postponed')
                      AND expected_result_check_after <= now()
                  ) AS oldest_due_coupon
                FROM simulated_coupons
                "#,
                &[],
            )
            .await?;
        let lookup_cadence_row = client
            .query_one(
                r#"
                WITH due AS (
                  SELECT
                    'single'::text AS item_type,
                    id AS simulated_bet_id,
                    NULL::text AS simulated_coupon_id,
                    hypothetical_stake::float8 AS stake
                  FROM simulated_bets
                  WHERE status IN ('awaiting_result', 'unresolved', 'postponed')
                    AND expected_result_check_after <= now()
                  UNION ALL
                  SELECT
                    'coupon'::text AS item_type,
                    NULL::text AS simulated_bet_id,
                    id AS simulated_coupon_id,
                    hypothetical_stake::float8 AS stake
                  FROM simulated_coupons
                  WHERE status IN ('awaiting_result', 'unresolved', 'postponed')
                    AND expected_result_check_after <= now()
                ),
                due_with_attempt AS (
                  SELECT
                    due.item_type,
                    due.simulated_bet_id,
                    due.simulated_coupon_id,
                    due.stake,
                    max(sla.created_at) AS last_lookup_at
                  FROM due
                  LEFT JOIN settlement_lookup_attempts sla
                    ON (
                     (due.simulated_bet_id IS NOT NULL AND sla.simulated_bet_id = due.simulated_bet_id)
                     OR (due.simulated_coupon_id IS NOT NULL AND sla.simulated_coupon_id = due.simulated_coupon_id)
                   )
                  GROUP BY due.item_type, due.simulated_bet_id, due.simulated_coupon_id, due.stake
                ),
                attempt_window AS (
                  SELECT
                    count(*)::int AS total_lookup_attempt_count,
                    count(*) FILTER (
                      WHERE created_at >= now() - ($1::int * interval '1 minute')
                    )::int AS lookup_attempts_in_cooldown,
                    max(created_at) AS last_lookup_attempt_at
                  FROM settlement_lookup_attempts
                )
                SELECT
                  (SELECT count(*)::int FROM due_with_attempt) AS due_lookup_item_count,
                  COALESCE(sum(stake), 0)::float8 AS due_lookup_exposure,
                  count(*) FILTER (
                    WHERE last_lookup_at >= now() - ($1::int * interval '1 minute')
                  )::int AS recently_checked_due_count,
                  COALESCE(sum(stake) FILTER (
                    WHERE last_lookup_at >= now() - ($1::int * interval '1 minute')
                  ), 0)::float8 AS recently_checked_due_exposure,
                  count(*) FILTER (
                    WHERE last_lookup_at IS NULL
                       OR last_lookup_at < now() - ($1::int * interval '1 minute')
                  )::int AS due_without_recent_lookup_count,
                  COALESCE(sum(stake) FILTER (
                    WHERE last_lookup_at IS NULL
                       OR last_lookup_at < now() - ($1::int * interval '1 minute')
                  ), 0)::float8 AS due_without_recent_lookup_exposure,
                  max(last_lookup_at) AS newest_due_lookup_at,
                  min(last_lookup_at) AS oldest_due_lookup_at,
                  CASE
                    WHEN count(*) FILTER (
                      WHERE last_lookup_at IS NULL
                         OR last_lookup_at < now() - ($1::int * interval '1 minute')
                    ) > 0
                    THEN now()
                    ELSE min(last_lookup_at + ($1::int * interval '1 minute'))
                  END AS next_lookup_due_at,
                  (SELECT total_lookup_attempt_count FROM attempt_window) AS total_lookup_attempt_count,
                  (SELECT lookup_attempts_in_cooldown FROM attempt_window) AS lookup_attempts_in_cooldown,
                  (SELECT last_lookup_attempt_at FROM attempt_window) AS last_lookup_attempt_at
                FROM due_with_attempt
                "#,
                &[&(lookup_cooldown_minutes.max(0) as i32)],
            )
            .await?;

        let by_sport = client
            .query(
                r#"
                WITH coupon_sports AS (
                  SELECT
                    sc.id,
                    CASE
                      WHEN count(DISTINCT cb.sport_key) = 1 THEN min(cb.sport_key)
                      ELSE 'mixed'
                    END AS sport_key
                  FROM simulated_coupons sc
                  LEFT JOIN simulated_coupon_legs scl ON scl.simulated_coupon_id = sc.id
                  LEFT JOIN candidate_bets cb ON cb.id = scl.candidate_id
                  GROUP BY sc.id
                ),
                positions AS (
                  SELECT
                    COALESCE(cb.sport_key, 'unknown') AS sport_key,
                    sb.status,
                    sb.hypothetical_stake,
                    sb.profit_loss,
                    sb.expected_result_check_after,
                    sb.observed_decimal_odds AS observed_odds
                  FROM simulated_bets sb
                  LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                  UNION ALL
                  SELECT
                    COALESCE(cs.sport_key, 'unknown') AS sport_key,
                    sc.status,
                    sc.hypothetical_stake,
                    sc.profit_loss,
                    sc.expected_result_check_after,
                    sc.observed_combined_decimal_odds AS observed_odds
                  FROM simulated_coupons sc
                  LEFT JOIN coupon_sports cs ON cs.id = sc.id
                )
                SELECT
                  sport_key,
                  count(*) FILTER (WHERE status <> 'duplicate_void')::int AS played_count,
                  count(*) FILTER (WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed'))::int AS open_count,
                  count(*) FILTER (WHERE status = 'awaiting_result')::int AS awaiting_result_count,
                  count(*) FILTER (
                    WHERE status IN ('awaiting_result', 'unresolved', 'postponed')
                      AND expected_result_check_after <= now()
                  )::int AS due_count,
                  count(*) FILTER (WHERE status IN ('settled_won', 'settled_lost'))::int AS decided_count,
                  count(*) FILTER (WHERE status = 'settled_won')::int AS won_count,
                  COALESCE(sum(hypothetical_stake) FILTER (WHERE status <> 'duplicate_void'), 0)::float8 AS turnover,
                  COALESCE(sum(hypothetical_stake) FILTER (WHERE status IN ('open', 'awaiting_result', 'unresolved', 'postponed')), 0)::float8 AS open_exposure,
                  COALESCE(sum(hypothetical_stake) FILTER (WHERE status = 'awaiting_result'), 0)::float8 AS awaiting_result_exposure,
                  COALESCE(sum(hypothetical_stake) FILTER (
                    WHERE status IN ('awaiting_result', 'unresolved', 'postponed')
                      AND expected_result_check_after <= now()
                  ), 0)::float8 AS due_exposure,
                  COALESCE(sum(profit_loss) FILTER (WHERE status <> 'duplicate_void'), 0)::float8 AS profit_loss,
                  avg(observed_odds) FILTER (WHERE status <> 'duplicate_void')::float8 AS average_odds
                FROM positions
                GROUP BY sport_key
                ORDER BY played_count DESC, sport_key
                "#,
                &[],
            )
            .await?;

        let stale_awaiting_rows = client
            .query(
                r#"
                SELECT
                  sb.id,
                  sb.status,
                  sb.expected_result_check_after,
                  cb.sport_key,
                  cb.event_name,
                  cb.market_name,
                  cb.outcome_name
                FROM simulated_bets sb
                LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                WHERE sb.status IN ('awaiting_result', 'unresolved', 'postponed')
                ORDER BY sb.expected_result_check_after ASC NULLS LAST, sb.created_at ASC
                LIMIT 10
                "#,
                &[],
            )
            .await?;
        let lookup_due_rows = client
            .query(
                r#"
                WITH due_items AS (
                  SELECT
                    'single'::text AS item_type,
                    sb.id AS item_id,
                    sb.status,
                    sb.expected_result_check_after,
                    sb.hypothetical_stake::float8 AS stake,
                    cb.sport_key,
                    cb.event_name,
                    cb.market_name,
                    cb.outcome_name,
                    NULL::text AS coupon_type,
                    NULL::int AS leg_count
                  FROM simulated_bets sb
                  LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                  WHERE sb.status IN ('awaiting_result', 'unresolved', 'postponed')
                    AND sb.expected_result_check_after <= now()
                  UNION ALL
                  SELECT
                    'coupon'::text AS item_type,
                    sc.id AS item_id,
                    sc.status,
                    sc.expected_result_check_after,
                    sc.hypothetical_stake::float8 AS stake,
                    COALESCE(min(cb.sport_key), 'mixed') AS sport_key,
                    cc.coupon_type || ' coupon' AS event_name,
                    NULL::text AS market_name,
                    NULL::text AS outcome_name,
                    cc.coupon_type,
                    cc.leg_count
                  FROM simulated_coupons sc
                  JOIN candidate_coupons cc ON cc.id = sc.coupon_id
                  LEFT JOIN simulated_coupon_legs scl ON scl.simulated_coupon_id = sc.id
                  LEFT JOIN candidate_bets cb ON cb.id = scl.candidate_id
                  WHERE sc.status IN ('awaiting_result', 'unresolved', 'postponed')
                    AND sc.expected_result_check_after <= now()
                  GROUP BY sc.id, sc.status, sc.expected_result_check_after, sc.hypothetical_stake, cc.coupon_type, cc.leg_count
                ),
                latest_lookup AS (
                  SELECT
                    due_items.*,
                    max(sla.created_at) AS last_lookup_at
                  FROM due_items
                  LEFT JOIN settlement_lookup_attempts sla
                    ON (
                     (due_items.item_type = 'single' AND sla.simulated_bet_id = due_items.item_id)
                     OR (due_items.item_type = 'coupon' AND sla.simulated_coupon_id = due_items.item_id)
                   )
                  GROUP BY
                    due_items.item_type,
                    due_items.item_id,
                    due_items.status,
                    due_items.expected_result_check_after,
                    due_items.stake,
                    due_items.sport_key,
                    due_items.event_name,
                    due_items.market_name,
                    due_items.outcome_name,
                    due_items.coupon_type,
                    due_items.leg_count
                )
                SELECT *
                FROM latest_lookup
                WHERE last_lookup_at IS NULL
                   OR last_lookup_at < now() - ($1::int * interval '1 minute')
                ORDER BY expected_result_check_after ASC NULLS LAST, item_id
                LIMIT 10
                "#,
                &[&(lookup_cooldown_minutes.max(0) as i32)],
            )
            .await?;

        let remaining_exposure = (max_open_exposure - ledger.open_exposure).max(0.0);
        let capacity_slots = if default_stake > 0.0 {
            (remaining_exposure / default_stake).floor() as usize
        } else {
            0
        };
        let next_scan_capacity = per_scan_limit.min(capacity_slots);
        let due_single = &due_rows[0];
        let due_coupon = &coupon_due_rows[0];
        let oldest_due_single: Option<DateTime<Utc>> = due_single.get("oldest_due_single");
        let oldest_due_coupon: Option<DateTime<Utc>> = due_coupon.get("oldest_due_coupon");
        let oldest_due = [oldest_due_single, oldest_due_coupon]
            .into_iter()
            .flatten()
            .min();
        let newest_due_lookup_at: Option<DateTime<Utc>> =
            lookup_cadence_row.get("newest_due_lookup_at");
        let oldest_due_lookup_at: Option<DateTime<Utc>> =
            lookup_cadence_row.get("oldest_due_lookup_at");
        let next_lookup_due_at: Option<DateTime<Utc>> =
            lookup_cadence_row.get("next_lookup_due_at");
        let last_lookup_attempt_at: Option<DateTime<Utc>> =
            lookup_cadence_row.get("last_lookup_attempt_at");

        let latest_snapshot_json = latest_snapshot.map(|row| {
            let observed_at: DateTime<Utc> = row.get("observed_at");
            json!({
                "id": row.get::<_, String>("id"),
                "observed_at": observed_at,
                "event_count": row.get::<_, i32>("event_count")
            })
        });
        let selected_unplaced_total_count = selected_unplaced_rows
            .first()
            .map(|row| row.get::<_, i32>("selected_unplaced_total_count"))
            .unwrap_or(0);

        Ok(json!({
            "paper_only": true,
            "latest_snapshot": latest_snapshot_json,
            "ledger": ledger,
            "played": played,
            "opportunity_intake": {
                "latest_candidate_status": candidate_status_rows.iter().map(|row| json!({
                    "status": row.get::<_, String>("status"),
                    "count": row.get::<_, i32>("count"),
                    "average_score": row.get::<_, Option<f64>>("average_score"),
                    "average_confidence": row.get::<_, Option<f64>>("average_confidence")
                })).collect::<Vec<_>>(),
                "selected_unplaced_count": selected_unplaced_total_count,
                "selected_unplaced": selected_unplaced_rows.iter().map(|row| json!({
                    "candidate_id": row.get::<_, String>("id"),
                    "sport_key": row.get::<_, String>("sport_key"),
                    "event_name": row.get::<_, Option<String>>("event_name"),
                    "competition": row.get::<_, Option<String>>("competition"),
                    "market_kind": row.get::<_, Option<String>>("market_kind"),
                    "market_name": row.get::<_, Option<String>>("market_name"),
                    "outcome_name": row.get::<_, Option<String>>("outcome_name"),
                    "decimal_odds": row.get::<_, Option<f64>>("decimal_odds"),
                    "score": row.get::<_, Option<f64>>("score"),
                    "confidence": row.get::<_, Option<f64>>("confidence")
                })).collect::<Vec<_>>()
            },
            "placement_capacity": {
                "default_stake": default_stake,
                "per_scan_limit": per_scan_limit,
                "max_open_exposure": max_open_exposure,
                "open_exposure": ledger.open_exposure,
                "remaining_exposure": remaining_exposure,
                "capacity_slots": capacity_slots,
                "next_scan_capacity": next_scan_capacity,
                "blocked": next_scan_capacity == 0,
                "block_reason": if next_scan_capacity == 0 { "open_exposure_cap_reached" } else { "" }
            },
            "settlement_work": {
                "due_single_count": due_single.get::<_, i32>("due_single_count"),
                "due_single_exposure": due_single.get::<_, f64>("due_single_exposure"),
                "due_coupon_count": due_coupon.get::<_, i32>("due_coupon_count"),
                "due_coupon_exposure": due_coupon.get::<_, f64>("due_coupon_exposure"),
                "due_total": due_single.get::<_, i32>("due_single_count") + due_coupon.get::<_, i32>("due_coupon_count"),
                "due_exposure": due_single.get::<_, f64>("due_single_exposure") + due_coupon.get::<_, f64>("due_coupon_exposure"),
                "oldest_due": oldest_due,
                "lookup_cadence": {
                    "source_scope": "settlement_review_sources",
                    "cooldown_minutes": lookup_cooldown_minutes,
                    "due_lookup_item_count": lookup_cadence_row.get::<_, i32>("due_lookup_item_count"),
                    "due_lookup_exposure": lookup_cadence_row.get::<_, f64>("due_lookup_exposure"),
                    "recently_checked_due_count": lookup_cadence_row.get::<_, i32>("recently_checked_due_count"),
                    "recently_checked_due_exposure": lookup_cadence_row.get::<_, f64>("recently_checked_due_exposure"),
                    "due_without_recent_lookup_count": lookup_cadence_row.get::<_, i32>("due_without_recent_lookup_count"),
                    "due_without_recent_lookup_exposure": lookup_cadence_row.get::<_, f64>("due_without_recent_lookup_exposure"),
                    "newest_due_lookup_at": newest_due_lookup_at,
                    "oldest_due_lookup_at": oldest_due_lookup_at,
                    "next_lookup_due_at": next_lookup_due_at,
                    "total_lookup_attempt_count": lookup_cadence_row.get::<_, i32>("total_lookup_attempt_count"),
                    "lookup_attempts_in_cooldown": lookup_cadence_row.get::<_, i32>("lookup_attempts_in_cooldown"),
                    "last_lookup_attempt_at": last_lookup_attempt_at
                },
                "lookup_due_items": lookup_due_rows.iter().map(|row| {
                    let expected: Option<DateTime<Utc>> = row.get("expected_result_check_after");
                    let last_lookup_at: Option<DateTime<Utc>> = row.get("last_lookup_at");
                    json!({
                        "item_type": row.get::<_, String>("item_type"),
                        "id": row.get::<_, String>("item_id"),
                        "status": row.get::<_, String>("status"),
                        "hypothetical_stake": row.get::<_, f64>("stake"),
                        "expected_result_check_after": expected,
                        "last_lookup_at": last_lookup_at,
                        "sport_key": row.get::<_, Option<String>>("sport_key"),
                        "event_name": row.get::<_, Option<String>>("event_name"),
                        "market_name": row.get::<_, Option<String>>("market_name"),
                        "outcome_name": row.get::<_, Option<String>>("outcome_name"),
                        "coupon_type": row.get::<_, Option<String>>("coupon_type"),
                        "leg_count": row.get::<_, Option<i32>>("leg_count")
                    })
                }).collect::<Vec<_>>(),
                "stale_awaiting": stale_awaiting_rows.iter().map(|row| {
                    let expected: Option<DateTime<Utc>> = row.get("expected_result_check_after");
                    json!({
                        "id": row.get::<_, String>("id"),
                        "status": row.get::<_, String>("status"),
                        "expected_result_check_after": expected,
                        "sport_key": row.get::<_, Option<String>>("sport_key"),
                        "event_name": row.get::<_, Option<String>>("event_name"),
                        "market_name": row.get::<_, Option<String>>("market_name"),
                        "outcome_name": row.get::<_, Option<String>>("outcome_name")
                    })
                }).collect::<Vec<_>>()
            },
            "by_sport": by_sport.iter().map(|row| {
                let decided_count: i32 = row.get("decided_count");
                let won_count: i32 = row.get("won_count");
                json!({
                    "sport_key": row.get::<_, String>("sport_key"),
                    "played_count": row.get::<_, i32>("played_count"),
                    "open_count": row.get::<_, i32>("open_count"),
                    "awaiting_result_count": row.get::<_, i32>("awaiting_result_count"),
                    "due_count": row.get::<_, i32>("due_count"),
                    "decided_count": decided_count,
                    "won_count": won_count,
                    "hit_rate": if decided_count > 0 { Some(won_count as f64 / decided_count as f64) } else { None },
                    "turnover": row.get::<_, f64>("turnover"),
                    "open_exposure": row.get::<_, f64>("open_exposure"),
                    "awaiting_result_exposure": row.get::<_, f64>("awaiting_result_exposure"),
                    "due_exposure": row.get::<_, f64>("due_exposure"),
                    "profit_loss": row.get::<_, f64>("profit_loss"),
                    "average_odds": row.get::<_, Option<f64>>("average_odds")
                })
            }).collect::<Vec<_>>()
        }))
    }

    pub async fn record_performance_snapshot(
        &self,
        source: &str,
        odds_snapshot_id: Option<&str>,
        performance: &Value,
    ) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let id = new_id();
        let ledger = performance.get("ledger").cloned().unwrap_or(Value::Null);
        let played = performance.get("played").cloned().unwrap_or(Value::Null);
        let payload = json!({
            "paper_only": true,
            "source": source,
            "latest_snapshot": performance.get("latest_snapshot").cloned().unwrap_or(Value::Null),
            "placement_capacity": performance.get("placement_capacity").cloned().unwrap_or(Value::Null),
            "settlement_work": performance.get("settlement_work").cloned().unwrap_or(Value::Null)
        });
        let row = client
            .query_one(
                r#"
                INSERT INTO simulation_performance_snapshots (
                  id, source, odds_snapshot_id, ledger, played, performance, payload
                )
                VALUES ($1,$2,$3,$4,$5,$6,$7)
                RETURNING id, created_at, source, odds_snapshot_id, ledger, played, performance, payload
                "#,
                &[
                    &id,
                    &source,
                    &odds_snapshot_id,
                    &ledger,
                    &played,
                    &performance,
                    &payload,
                ],
            )
            .await?;
        Ok(performance_snapshot_from_row(&row))
    }

    pub async fn performance_history(&self, limit: i64) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT id, created_at, source, odds_snapshot_id, ledger, played, performance, payload
                FROM simulation_performance_snapshots
                ORDER BY created_at DESC
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        Ok(json!({
            "items": rows.iter().map(performance_snapshot_from_row).collect::<Vec<_>>()
        }))
    }

    pub async fn market_catalog_coverage(&self) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let sports = client
            .query(
                r#"
                WITH event_counts AS (
                  SELECT sport_key, count(*)::int AS event_count
                  FROM sport_events
                  GROUP BY sport_key
                ),
                competition_counts AS (
                  SELECT sport_key, count(*)::int AS competition_count
                  FROM competitions
                  GROUP BY sport_key
                ),
                market_counts AS (
                  SELECT e.sport_key, count(mo.id)::int AS market_count
                  FROM sport_events e
                  JOIN market_observations mo ON mo.event_id = e.id
                  GROUP BY e.sport_key
                ),
                outcome_counts AS (
                  SELECT e.sport_key, count(oo.id)::int AS outcome_count
                  FROM sport_events e
                  JOIN market_observations mo ON mo.event_id = e.id
                  JOIN outcome_observations oo ON oo.market_observation_id = mo.id
                  GROUP BY e.sport_key
                ),
                candidate_counts AS (
                  SELECT sport_key, count(*)::int AS candidate_count
                  FROM candidate_bets
                  GROUP BY sport_key
                )
                SELECT
                  s.sport_key,
                  s.label,
                  s.last_seen_at,
                  COALESCE(ec.event_count, 0) AS event_count,
                  COALESCE(cc.competition_count, 0) AS competition_count,
                  COALESCE(mc.market_count, 0) AS market_count,
                  COALESCE(oc.outcome_count, 0) AS outcome_count,
                  COALESCE(cbc.candidate_count, 0) AS candidate_count
                FROM sports s
                LEFT JOIN event_counts ec ON ec.sport_key = s.sport_key
                LEFT JOIN competition_counts cc ON cc.sport_key = s.sport_key
                LEFT JOIN market_counts mc ON mc.sport_key = s.sport_key
                LEFT JOIN outcome_counts oc ON oc.sport_key = s.sport_key
                LEFT JOIN candidate_counts cbc ON cbc.sport_key = s.sport_key
                ORDER BY s.sport_key
                "#,
                &[],
            )
            .await?;
        let market_kinds = client
            .query(
                r#"
                SELECT e.sport_key, mo.market_kind, count(*)::int AS market_count
                FROM market_observations mo
                JOIN sport_events e ON e.id = mo.event_id
                GROUP BY e.sport_key, mo.market_kind
                ORDER BY e.sport_key, market_count DESC, mo.market_kind
                "#,
                &[],
            )
            .await?;
        let competitions = client
            .query(
                r#"
                SELECT c.sport_key, c.name, c.class_name, c.last_seen_at, count(e.id)::int AS event_count
                FROM competitions c
                LEFT JOIN sport_events e ON e.sport_key = c.sport_key AND e.competition_name = c.name
                GROUP BY c.sport_key, c.name, c.class_name, c.last_seen_at
                ORDER BY event_count DESC, c.last_seen_at DESC
                LIMIT 30
                "#,
                &[],
            )
            .await?;
        Ok(json!({
            "sports": sports.iter().map(|row| {
                let last_seen_at: DateTime<Utc> = row.get("last_seen_at");
                json!({
                    "sport_key": row.get::<_, String>("sport_key"),
                    "label": row.get::<_, Option<String>>("label"),
                    "last_seen_at": last_seen_at,
                    "event_count": row.get::<_, i32>("event_count"),
                    "competition_count": row.get::<_, i32>("competition_count"),
                    "market_count": row.get::<_, i32>("market_count"),
                    "outcome_count": row.get::<_, i32>("outcome_count"),
                    "candidate_count": row.get::<_, i32>("candidate_count")
                })
            }).collect::<Vec<_>>(),
            "market_kinds": market_kinds.iter().map(|row| {
                json!({
                    "sport_key": row.get::<_, String>("sport_key"),
                    "market_kind": row.get::<_, Option<String>>("market_kind"),
                    "market_count": row.get::<_, i32>("market_count")
                })
            }).collect::<Vec<_>>(),
            "competitions": competitions.iter().map(|row| {
                let last_seen_at: DateTime<Utc> = row.get("last_seen_at");
                json!({
                    "sport_key": row.get::<_, String>("sport_key"),
                    "name": row.get::<_, String>("name"),
                    "class_name": row.get::<_, Option<String>>("class_name"),
                    "last_seen_at": last_seen_at,
                    "event_count": row.get::<_, i32>("event_count")
                })
            }).collect::<Vec<_>>()
        }))
    }

    pub async fn coupon_rule_observations(&self, limit: i64) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let summary = client
            .query(
                r#"
                SELECT
                  sport_key,
                  count(*)::int AS rule_count,
                  count(DISTINCT event_id)::int AS event_count,
                  min(minimum_accumulator)::int AS min_accumulator,
                  max(maximum_accumulator)::int AS max_accumulator,
                  max(observed_at) AS last_observed_at
                FROM coupon_rule_observations
                GROUP BY sport_key
                ORDER BY sport_key
                "#,
                &[],
            )
            .await?;
        let rows = client
            .query(
                r#"
                SELECT
                  id, snapshot_id, sport_key, event_id, market_observation_id,
                  market_id, market_name, market_kind, group_code, competition_name,
                  minimum_accumulator, maximum_accumulator, restriction_scope,
                  observed_at, payload
                FROM coupon_rule_observations
                ORDER BY observed_at DESC
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        Ok(json!({
            "summary": summary.iter().map(|row| {
                let last_observed_at: DateTime<Utc> = row.get("last_observed_at");
                json!({
                    "sport_key": row.get::<_, String>("sport_key"),
                    "rule_count": row.get::<_, i32>("rule_count"),
                    "event_count": row.get::<_, i32>("event_count"),
                    "min_accumulator": row.get::<_, Option<i32>>("min_accumulator"),
                    "max_accumulator": row.get::<_, Option<i32>>("max_accumulator"),
                    "last_observed_at": last_observed_at
                })
            }).collect::<Vec<_>>(),
            "items": rows.iter().map(|row| {
                let observed_at: DateTime<Utc> = row.get("observed_at");
                json!({
                    "id": row.get::<_, String>("id"),
                    "snapshot_id": row.get::<_, String>("snapshot_id"),
                    "sport_key": row.get::<_, String>("sport_key"),
                    "event_id": row.get::<_, String>("event_id"),
                    "market_observation_id": row.get::<_, String>("market_observation_id"),
                    "market_id": row.get::<_, Option<String>>("market_id"),
                    "market_name": row.get::<_, Option<String>>("market_name"),
                    "market_kind": row.get::<_, Option<String>>("market_kind"),
                    "group_code": row.get::<_, Option<String>>("group_code"),
                    "competition_name": row.get::<_, Option<String>>("competition_name"),
                    "minimum_accumulator": row.get::<_, Option<i32>>("minimum_accumulator"),
                    "maximum_accumulator": row.get::<_, Option<i32>>("maximum_accumulator"),
                    "restriction_scope": row.get::<_, String>("restriction_scope"),
                    "observed_at": observed_at,
                    "payload": row.get::<_, Value>("payload")
                })
            }).collect::<Vec<_>>()
        }))
    }

    pub async fn odds_movement(&self, limit: i64) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                WITH observed AS (
                  SELECT
                    os.id AS snapshot_id,
                    os.observed_at AS current_observed_at,
                    se.sport_key,
                    se.event_name,
                    se.competition_name,
                    se.start_time,
                    mo.event_id,
                    mo.market_id,
                    mo.market_name,
                    mo.market_kind,
                    mo.group_code,
                    oo.outcome_id,
                    oo.outcome_name,
                    oo.decimal_odds::float8 AS current_decimal_odds,
                    oo.active AS current_active,
                    oo.displayed AS current_displayed,
                    lag(oo.decimal_odds::float8) OVER (
                      PARTITION BY mo.event_id, mo.market_id, oo.outcome_id
                      ORDER BY os.observed_at ASC
                    ) AS previous_decimal_odds,
                    lag(os.observed_at) OVER (
                      PARTITION BY mo.event_id, mo.market_id, oo.outcome_id
                      ORDER BY os.observed_at ASC
                    ) AS previous_observed_at,
                    row_number() OVER (
                      PARTITION BY mo.event_id, mo.market_id, oo.outcome_id
                      ORDER BY os.observed_at DESC
                    ) AS latest_rank
                  FROM outcome_observations oo
                  JOIN market_observations mo ON mo.id = oo.market_observation_id
                  JOIN odds_snapshots os ON os.id = oo.snapshot_id
                  JOIN sport_events se ON se.id = mo.event_id
                  WHERE oo.decimal_odds IS NOT NULL
                    AND mo.market_id IS NOT NULL
                    AND oo.outcome_id IS NOT NULL
                )
                SELECT
                  snapshot_id,
                  current_observed_at,
                  previous_observed_at,
                  sport_key,
                  event_id,
                  event_name,
                  competition_name,
                  start_time,
                  market_id,
                  market_name,
                  market_kind,
                  group_code,
                  outcome_id,
                  outcome_name,
                  current_decimal_odds,
                  previous_decimal_odds,
                  current_decimal_odds - previous_decimal_odds AS decimal_odds_delta,
                  CASE
                    WHEN previous_decimal_odds = 0 THEN NULL
                    ELSE (current_decimal_odds - previous_decimal_odds) / previous_decimal_odds
                  END AS decimal_odds_delta_pct,
                  current_active,
                  current_displayed
                FROM observed
                WHERE latest_rank = 1
                  AND previous_decimal_odds IS NOT NULL
                ORDER BY abs(current_decimal_odds - previous_decimal_odds) DESC,
                         current_observed_at DESC
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        let items = rows
            .iter()
            .map(|row| {
                let current_observed_at: DateTime<Utc> = row.get("current_observed_at");
                let previous_observed_at: DateTime<Utc> = row.get("previous_observed_at");
                let current_decimal_odds: f64 = row.get("current_decimal_odds");
                let previous_decimal_odds: f64 = row.get("previous_decimal_odds");
                let decimal_odds_delta: f64 = row.get("decimal_odds_delta");
                let direction = if decimal_odds_delta > 0.0 {
                    "up"
                } else if decimal_odds_delta < 0.0 {
                    "down"
                } else {
                    "flat"
                };
                json!({
                    "snapshot_id": row.get::<_, String>("snapshot_id"),
                    "current_observed_at": current_observed_at,
                    "previous_observed_at": previous_observed_at,
                    "sport_key": row.get::<_, String>("sport_key"),
                    "event_id": row.get::<_, String>("event_id"),
                    "event_name": row.get::<_, Option<String>>("event_name"),
                    "competition_name": row.get::<_, Option<String>>("competition_name"),
                    "start_time": row.get::<_, Option<DateTime<Utc>>>("start_time"),
                    "market_id": row.get::<_, Option<String>>("market_id"),
                    "market_name": row.get::<_, Option<String>>("market_name"),
                    "market_kind": row.get::<_, Option<String>>("market_kind"),
                    "group_code": row.get::<_, Option<String>>("group_code"),
                    "outcome_id": row.get::<_, Option<String>>("outcome_id"),
                    "outcome_name": row.get::<_, Option<String>>("outcome_name"),
                    "current_decimal_odds": current_decimal_odds,
                    "previous_decimal_odds": previous_decimal_odds,
                    "decimal_odds_delta": decimal_odds_delta,
                    "decimal_odds_delta_pct": row.get::<_, Option<f64>>("decimal_odds_delta_pct"),
                    "direction": direction,
                    "current_active": row.get::<_, Option<bool>>("current_active"),
                    "current_displayed": row.get::<_, Option<bool>>("current_displayed"),
                    "paper_only": true
                })
            })
            .collect::<Vec<_>>();
        let up_count = items
            .iter()
            .filter(|item| item.get("direction").and_then(Value::as_str) == Some("up"))
            .count();
        let down_count = items
            .iter()
            .filter(|item| item.get("direction").and_then(Value::as_str) == Some("down"))
            .count();
        Ok(json!({
            "items": items,
            "summary": {
                "returned_count": rows.len(),
                "up_count": up_count,
                "down_count": down_count,
                "limit": limit,
                "source": "outcome_observations_latest_vs_previous",
                "paper_only": true
            }
        }))
    }

    pub async fn intelligence_coverage(&self) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let sources = client
            .query(
                r#"
                SELECT source_key, source_name, source_type, reliability::float8 AS reliability,
                       can_settle, manual_review_required, last_seen_at
                FROM source_registry
                ORDER BY source_key
                "#,
                &[],
            )
            .await?;
        let features = client
            .query(
                r#"
                SELECT
                  sport_key,
                  count(*)::int AS feature_count,
                  count(DISTINCT event_id)::int AS event_count,
                  avg(confidence)::float8 AS average_confidence,
                  count(*) FILTER (WHERE missing_signals ? 'weather')::int AS missing_weather_count,
                  count(*) FILTER (WHERE missing_signals ? 'news')::int AS missing_news_count,
                  count(*) FILTER (WHERE missing_signals ? 'rankings')::int AS missing_rankings_count,
                  count(*) FILTER (WHERE missing_signals ? 'form')::int AS missing_form_count,
                  max(created_at) AS last_created_at
                FROM feature_snapshots
                GROUP BY sport_key
                ORDER BY sport_key
                "#,
                &[],
            )
            .await?;
        let motorsports_series = client
            .query(
                r#"
                WITH base AS (
                  SELECT
                    event_id,
                    confidence,
                    created_at,
                    COALESCE(NULLIF(features #>> '{sport_context,series_family}', ''), 'unknown') AS stored_series_family,
                    lower(concat_ws(' ', features->>'competition', features->>'class_name', features->>'event_name')) AS haystack
                  FROM feature_snapshots
                  WHERE sport_key = 'motorsports'
                ),
                effective AS (
                  SELECT
                    event_id,
                    confidence,
                    created_at,
                    stored_series_family,
                    CASE
                      WHEN stored_series_family != 'unknown' THEN stored_series_family
                      WHEN haystack LIKE '%indycar%' OR haystack LIKE '%indy car%' THEN 'indycar'
                      WHEN haystack LIKE '%nascar%' THEN 'nascar'
                      WHEN haystack LIKE '%le mans%' OR haystack LIKE '%wec%' OR haystack LIKE '%endurance%' OR haystack LIKE '%imsa%' THEN 'endurance'
                      WHEN haystack LIKE '%motogp%' OR haystack LIKE '%moto gp%' OR haystack LIKE '%superbike%' OR haystack LIKE '%motorbike%' OR haystack LIKE '%motorcycle%' THEN 'motorbike'
                      WHEN haystack LIKE '%formula 1%' OR haystack LIKE '%formel 1%' OR haystack ~ '(^|[^a-z0-9])f1([^a-z0-9]|$)' THEN 'formula_1'
                      WHEN haystack LIKE '%rally%' OR haystack LIKE '%wrc%' THEN 'rally'
                      ELSE 'unknown'
                    END AS series_family
                  FROM base
                )
                SELECT
                  series_family,
                  CASE
                    WHEN series_family = 'motorbike' THEN 'motorbike'
                    WHEN series_family = 'unknown' THEN 'unknown'
                    ELSE 'car'
                  END AS vehicle_type,
                  count(*)::int AS feature_count,
                  count(DISTINCT event_id)::int AS event_count,
                  avg(confidence)::float8 AS average_confidence,
                  count(*) FILTER (WHERE series_family = 'unknown')::int AS missing_series_count,
                  count(*) FILTER (WHERE stored_series_family = 'unknown')::int AS stored_unknown_count,
                  count(*) FILTER (WHERE stored_series_family = 'unknown' AND series_family != 'unknown')::int AS recovered_series_count,
                  max(created_at) AS last_created_at
                FROM effective
                GROUP BY series_family, vehicle_type
                ORDER BY feature_count DESC, series_family
                "#,
                &[],
            )
            .await?;
        let runs = client
            .query(
                r#"
                SELECT id, source_key, snapshot_id, completed_at, status, sport_keys, event_count, payload
                FROM ingestion_runs
                ORDER BY completed_at DESC
                LIMIT 10
                "#,
                &[],
            )
            .await?;
        Ok(json!({
            "sources": sources.iter().map(|row| {
                let last_seen_at: DateTime<Utc> = row.get("last_seen_at");
                json!({
                    "source_key": row.get::<_, String>("source_key"),
                    "source_name": row.get::<_, String>("source_name"),
                    "source_type": row.get::<_, String>("source_type"),
                    "reliability": row.get::<_, f64>("reliability"),
                    "can_settle": row.get::<_, bool>("can_settle"),
                    "manual_review_required": row.get::<_, bool>("manual_review_required"),
                    "last_seen_at": last_seen_at
                })
            }).collect::<Vec<_>>(),
            "features": features.iter().map(|row| {
                let last_created_at: DateTime<Utc> = row.get("last_created_at");
                json!({
                    "sport_key": row.get::<_, String>("sport_key"),
                    "feature_count": row.get::<_, i32>("feature_count"),
                    "event_count": row.get::<_, i32>("event_count"),
                    "average_confidence": row.get::<_, Option<f64>>("average_confidence"),
                    "missing_weather_count": row.get::<_, i32>("missing_weather_count"),
                    "missing_news_count": row.get::<_, i32>("missing_news_count"),
                    "missing_rankings_count": row.get::<_, i32>("missing_rankings_count"),
                    "missing_form_count": row.get::<_, i32>("missing_form_count"),
                    "last_created_at": last_created_at
                })
            }).collect::<Vec<_>>(),
            "motorsports_series": motorsports_series.iter().map(|row| {
                let last_created_at: DateTime<Utc> = row.get("last_created_at");
                json!({
                    "series_family": row.get::<_, String>("series_family"),
                    "vehicle_type": row.get::<_, String>("vehicle_type"),
                    "feature_count": row.get::<_, i32>("feature_count"),
                    "event_count": row.get::<_, i32>("event_count"),
                    "average_confidence": row.get::<_, Option<f64>>("average_confidence"),
                    "missing_series_count": row.get::<_, i32>("missing_series_count"),
                    "stored_unknown_count": row.get::<_, i32>("stored_unknown_count"),
                    "recovered_series_count": row.get::<_, i32>("recovered_series_count"),
                    "last_created_at": last_created_at
                })
            }).collect::<Vec<_>>(),
            "recent_runs": runs.iter().map(|row| {
                let completed_at: DateTime<Utc> = row.get("completed_at");
                json!({
                    "id": row.get::<_, String>("id"),
                    "source_key": row.get::<_, Option<String>>("source_key"),
                    "snapshot_id": row.get::<_, Option<String>>("snapshot_id"),
                    "completed_at": completed_at,
                    "status": row.get::<_, String>("status"),
                    "sport_keys": row.get::<_, Vec<String>>("sport_keys"),
                    "event_count": row.get::<_, i32>("event_count"),
                    "payload": row.get::<_, Value>("payload")
                })
            }).collect::<Vec<_>>()
        }))
    }

    pub async fn entity_aliases(&self, limit: i64) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT
                  id,
                  entity_kind,
                  sport_key,
                  gender_scope,
                  canonical_name,
                  canonical_key,
                  alias_name,
                  alias_key,
                  source_key,
                  external_id,
                  confidence::float8 AS confidence,
                  payload,
                  first_seen_at,
                  last_seen_at
                FROM entity_aliases
                ORDER BY last_seen_at DESC, entity_kind, canonical_name, alias_name
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        Ok(json!({
            "items": rows.iter().map(|row| {
                let first_seen_at: DateTime<Utc> = row.get("first_seen_at");
                let last_seen_at: DateTime<Utc> = row.get("last_seen_at");
                json!({
                    "id": row.get::<_, String>("id"),
                    "entity_kind": row.get::<_, String>("entity_kind"),
                    "sport_key": row.get::<_, Option<String>>("sport_key"),
                    "gender_scope": row.get::<_, Option<String>>("gender_scope"),
                    "canonical_name": row.get::<_, String>("canonical_name"),
                    "canonical_key": row.get::<_, String>("canonical_key"),
                    "alias_name": row.get::<_, String>("alias_name"),
                    "alias_key": row.get::<_, String>("alias_key"),
                    "source_key": row.get::<_, Option<String>>("source_key"),
                    "external_id": row.get::<_, Option<String>>("external_id"),
                    "confidence": row.get::<_, f64>("confidence"),
                    "payload": row.get::<_, Value>("payload"),
                    "first_seen_at": first_seen_at,
                    "last_seen_at": last_seen_at
                })
            }).collect::<Vec<_>>(),
            "paper_only": true
        }))
    }

    pub async fn add_entity_alias(&self, payload: &Value) -> anyhow::Result<Value> {
        let entity_kind = payload
            .get("entity_kind")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("participant");
        validate_entity_kind(entity_kind)?;
        let sport_key = payload
            .get("sport_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let gender_scope = payload_gender_scope(payload)?;
        let canonical_name = payload
            .get("canonical_name")
            .or_else(|| payload.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("canonical_name is required"))?;
        let alias_name = payload
            .get("alias_name")
            .or_else(|| payload.get("alias"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("alias_name is required"))?;
        let source_key = payload
            .get("source_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let external_id = payload
            .get("external_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let confidence = payload
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.75)
            .clamp(0.0, 1.0);
        let alias_payload = json!({
            "notes": payload.get("notes").cloned().unwrap_or(Value::Null),
            "paper_only": true
        });
        let client = self.connect().await?;
        let row = upsert_entity_alias(
            &client,
            EntityAliasInput {
                entity_kind,
                sport_key: sport_key.as_deref(),
                gender_scope: gender_scope.as_deref(),
                canonical_name,
                alias_name,
                source_key: source_key.as_deref(),
                external_id: external_id.as_deref(),
                confidence,
                payload: alias_payload,
            },
        )
        .await?;
        Ok(row)
    }

    pub async fn settlement_sources(&self) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT
                  source_key,
                  source_name,
                  source_type,
                  url_pattern,
                  sport_scope,
                  reliability::float8 AS reliability,
                  manual_review_required,
                  notes,
                  last_seen_at,
                  payload
                FROM source_registry
                WHERE can_settle = true
                ORDER BY
                  COALESCE((payload->>'priority')::int, 999),
                  reliability DESC,
                  source_key
                "#,
                &[],
            )
            .await?;
        let link_rows = client
            .query(
                r#"
                SELECT
                  source_key,
                  event_name,
                  source_url,
                  home_aliases,
                  away_aliases,
                  requires_browser_automation,
                  payload,
                  updated_at
                FROM external_result_links
                ORDER BY updated_at DESC, event_name
                "#,
                &[],
            )
            .await?;
        let mut links_by_source: HashMap<String, Vec<Value>> = HashMap::new();
        for row in link_rows {
            let updated_at: DateTime<Utc> = row.get("updated_at");
            let payload = row.get::<_, Value>("payload");
            let sport_scope = payload
                .get("sport_key")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let gender_scope = payload_gender_scope(&payload).ok().flatten();
            let event_name = row.get::<_, String>("event_name");
            let home_aliases = row.get::<_, Vec<String>>("home_aliases");
            let away_aliases = row.get::<_, Vec<String>>("away_aliases");
            let use_alias_registry =
                use_external_result_alias_registry(sport_scope.as_deref(), &event_name);
            let home_aliases = if use_alias_registry {
                expand_aliases_from_registry(
                    &client,
                    "participant",
                    sport_scope.as_deref(),
                    gender_scope.as_deref(),
                    home_aliases,
                )
                .await?
            } else {
                home_aliases
            };
            let away_aliases = if use_alias_registry {
                expand_aliases_from_registry(
                    &client,
                    "participant",
                    sport_scope.as_deref(),
                    gender_scope.as_deref(),
                    away_aliases,
                )
                .await?
            } else {
                away_aliases
            };
            links_by_source
                .entry(row.get::<_, String>("source_key"))
                .or_default()
                .push(json!({
                    "event_name": event_name,
                    "url": row.get::<_, String>("source_url"),
                    "home_aliases": home_aliases,
                    "away_aliases": away_aliases,
                    "sport_key": sport_scope,
                    "gender_scope": gender_scope,
                    "requires_browser_automation": row.get::<_, bool>("requires_browser_automation"),
                    "operator_configured": true,
                    "updated_at": updated_at,
                    "payload": payload
                }));
        }
        Ok(json!({
            "paper_only": true,
            "manual_review_required": true,
            "items": rows.iter().map(|row| {
                let last_seen_at: DateTime<Utc> = row.get("last_seen_at");
                let source_key = row.get::<_, String>("source_key");
                let payload = merge_operator_result_links(
                    row.get::<_, Value>("payload"),
                    links_by_source.get(&source_key).map(Vec::as_slice).unwrap_or(&[])
                );
                json!({
                    "source_key": source_key,
                    "source_name": row.get::<_, String>("source_name"),
                    "source_type": row.get::<_, String>("source_type"),
                    "url_pattern": row.get::<_, Option<String>>("url_pattern"),
                    "sport_scope": row.get::<_, Vec<String>>("sport_scope"),
                    "reliability": row.get::<_, f64>("reliability"),
                    "manual_review_required": row.get::<_, bool>("manual_review_required"),
                    "notes": row.get::<_, Option<String>>("notes"),
                    "last_seen_at": last_seen_at,
                    "payload": payload
                })
            }).collect::<Vec<_>>()
        }))
    }

    pub async fn add_external_result_link(&self, payload: &Value) -> anyhow::Result<Value> {
        let source_key = payload
            .get("source_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("source_key is required"))?;
        let source_record = self.settlement_source_record(source_key).await?;
        if !is_external_result_source(source_key) {
            return Err(anyhow!(
                "source_key is not an external result source: {source_key}"
            ));
        }
        let source_url = payload
            .get("source_url")
            .or_else(|| payload.get("url"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("source_url is required"))?;
        validate_external_result_url(source_key, source_url)?;
        let event_name = payload
            .get("event_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("event_name is required"))?;
        let (default_home_aliases, default_away_aliases) = event_aliases_from_name(event_name);
        let home_aliases = json_string_array(payload.get("home_aliases"))
            .into_iter()
            .chain(default_home_aliases)
            .collect::<Vec<_>>();
        let away_aliases = json_string_array(payload.get("away_aliases"))
            .into_iter()
            .chain(default_away_aliases)
            .collect::<Vec<_>>();
        let home_aliases = dedup_strings(home_aliases);
        let away_aliases = dedup_strings(away_aliases);
        let sport_key = payload
            .get("sport_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let gender_scope = payload_gender_scope(payload)?;
        let source_requires_browser = source_record
            .get("payload")
            .and_then(|payload| payload.get("requires_browser_automation"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let requires_browser_automation = payload
            .get("requires_browser_automation")
            .and_then(Value::as_bool)
            .unwrap_or(source_requires_browser);
        let link_payload = json!({
            "paper_only": true,
            "operator_configured": true,
            "sport_key": sport_key,
            "gender_scope": gender_scope.as_deref(),
            "source_record": source_record,
            "notes": payload.get("notes").cloned().unwrap_or(Value::Null)
        });
        let client = self.connect().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO external_result_links (
                  id, source_key, event_name, source_url, home_aliases, away_aliases,
                  requires_browser_automation, payload
                )
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                ON CONFLICT (source_key, event_name, source_url) DO UPDATE
                SET home_aliases = EXCLUDED.home_aliases,
                    away_aliases = EXCLUDED.away_aliases,
                    requires_browser_automation = EXCLUDED.requires_browser_automation,
                    payload = EXCLUDED.payload,
                    updated_at = now()
                RETURNING id, created_at, updated_at
                "#,
                &[
                    &new_id(),
                    &source_key,
                    &event_name,
                    &source_url,
                    &home_aliases,
                    &away_aliases,
                    &requires_browser_automation,
                    &link_payload,
                ],
            )
            .await?;
        let created_at: DateTime<Utc> = row.get("created_at");
        let updated_at: DateTime<Utc> = row.get("updated_at");
        let home_canonical = home_aliases
            .first()
            .cloned()
            .or_else(|| event_aliases_from_name(event_name).0.into_iter().next())
            .unwrap_or_else(|| event_name.to_string());
        let away_canonical = away_aliases
            .first()
            .cloned()
            .or_else(|| event_aliases_from_name(event_name).1.into_iter().next())
            .unwrap_or_else(|| event_name.to_string());
        let alias_notes = json!({
            "event_name": event_name,
            "source_url": source_url,
            "method": "external_result_link",
            "paper_only": true
        });
        let use_alias_registry = use_external_result_alias_registry(sport_key, event_name);
        let (recorded_home_aliases, recorded_away_aliases) = if use_alias_registry {
            (
                record_entity_aliases(
                    &client,
                    "participant",
                    sport_key,
                    gender_scope.as_deref(),
                    &home_canonical,
                    home_aliases.clone(),
                    Some(source_key),
                    None,
                    0.82,
                    alias_notes.clone(),
                )
                .await?,
                record_entity_aliases(
                    &client,
                    "participant",
                    sport_key,
                    gender_scope.as_deref(),
                    &away_canonical,
                    away_aliases.clone(),
                    Some(source_key),
                    None,
                    0.82,
                    alias_notes,
                )
                .await?,
            )
        } else {
            (Vec::new(), Vec::new())
        };
        let expanded_home_aliases = if use_alias_registry {
            expand_aliases_from_registry(
                &client,
                "participant",
                sport_key,
                gender_scope.as_deref(),
                home_aliases.clone(),
            )
            .await?
        } else {
            home_aliases.clone()
        };
        let expanded_away_aliases = if use_alias_registry {
            expand_aliases_from_registry(
                &client,
                "participant",
                sport_key,
                gender_scope.as_deref(),
                away_aliases.clone(),
            )
            .await?
        } else {
            away_aliases.clone()
        };
        let link = ExternalResultLink {
            source_key: source_key.to_string(),
            url: source_url.to_string(),
            sport_key: sport_key.map(str::to_string),
            gender_scope: gender_scope.clone(),
            home_aliases: expanded_home_aliases,
            away_aliases: expanded_away_aliases,
            requires_browser_automation,
            known_home_score: json_i32(payload.get("home_score")),
            known_away_score: json_i32(payload.get("away_score")),
            known_result_status: payload
                .get("result_status")
                .and_then(Value::as_str)
                .map(str::to_string),
            known_result_notes: payload
                .get("notes")
                .and_then(Value::as_str)
                .map(str::to_string),
        };
        Ok(json!({
            "paper_only": true,
            "id": row.get::<_, String>("id"),
            "created_at": created_at,
            "updated_at": updated_at,
            "event_name": event_name,
            "link": external_result_link_json(&link),
            "recorded_aliases": {
                "home": recorded_home_aliases,
                "away": recorded_away_aliases
            }
        }))
    }

    pub async fn external_result_links(&self, limit: i64) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT
                  erl.id,
                  erl.source_key,
                  sr.source_name,
                  erl.event_name,
                  erl.source_url,
                  erl.home_aliases,
                  erl.away_aliases,
                  erl.requires_browser_automation,
                  erl.payload,
                  erl.created_at,
                  erl.updated_at
                FROM external_result_links erl
                JOIN source_registry sr ON sr.source_key = erl.source_key
                ORDER BY erl.updated_at DESC, erl.event_name
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        let mut items = Vec::new();
        for row in rows {
            let created_at: DateTime<Utc> = row.get("created_at");
            let updated_at: DateTime<Utc> = row.get("updated_at");
            let payload = row.get::<_, Value>("payload");
            let sport_scope = payload
                .get("sport_key")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let gender_scope = payload_gender_scope(&payload).ok().flatten();
            let event_name = row.get::<_, String>("event_name");
            let home_aliases = row.get::<_, Vec<String>>("home_aliases");
            let away_aliases = row.get::<_, Vec<String>>("away_aliases");
            let use_alias_registry =
                use_external_result_alias_registry(sport_scope.as_deref(), &event_name);
            let home_aliases = if use_alias_registry {
                expand_aliases_from_registry(
                    &client,
                    "participant",
                    sport_scope.as_deref(),
                    gender_scope.as_deref(),
                    home_aliases,
                )
                .await?
            } else {
                home_aliases
            };
            let away_aliases = if use_alias_registry {
                expand_aliases_from_registry(
                    &client,
                    "participant",
                    sport_scope.as_deref(),
                    gender_scope.as_deref(),
                    away_aliases,
                )
                .await?
            } else {
                away_aliases
            };
            items.push(json!({
                "id": row.get::<_, String>("id"),
                "source_key": row.get::<_, String>("source_key"),
                "source_name": row.get::<_, String>("source_name"),
                "event_name": event_name,
                "source_url": row.get::<_, String>("source_url"),
                "sport_key": sport_scope,
                "gender_scope": gender_scope,
                "home_aliases": home_aliases,
                "away_aliases": away_aliases,
                "requires_browser_automation": row.get::<_, bool>("requires_browser_automation"),
                "created_at": created_at,
                "updated_at": updated_at,
                "payload": payload
            }));
        }
        Ok(json!({
            "paper_only": true,
            "items": items
        }))
    }

    pub async fn settlement_observations(&self, limit: i64) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT
                  so.id,
                  so.created_at,
                  CASE WHEN so.simulated_coupon_id IS NULL THEN 'single' ELSE 'coupon' END AS item_type,
                  so.simulated_bet_id,
                  so.simulated_coupon_id,
                  so.source,
                  so.observed_result,
                  so.confidence::float8 AS confidence,
                  so.payload,
                  sb.status AS bet_status,
                  sb.strategy_id AS bet_strategy_id,
                  cb.sport_key AS bet_sport_key,
                  cb.event_name AS bet_event_name,
                  cb.competition AS bet_competition,
                  cb.market_name AS bet_market_name,
                  cb.outcome_name AS bet_outcome_name,
                  sc.status AS coupon_status,
                  sc.strategy_id AS coupon_strategy_id,
                  cc.coupon_type,
                  cc.leg_count
                FROM settlement_observations so
                LEFT JOIN simulated_bets sb ON sb.id = so.simulated_bet_id
                LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                LEFT JOIN simulated_coupons sc ON sc.id = so.simulated_coupon_id
                LEFT JOIN candidate_coupons cc ON cc.id = sc.coupon_id
                ORDER BY so.created_at DESC
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        Ok(json!({
            "paper_only": true,
            "items": rows.iter().map(|row| {
                let created_at: DateTime<Utc> = row.get("created_at");
                let item_type: String = row.get("item_type");
                let payload: Value = row.get("payload");
                let source_policy = payload
                    .get("manual_settlement")
                    .and_then(|manual| manual.get("source_policy"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let status: Option<String> = if item_type == "coupon" {
                    row.get("coupon_status")
                } else {
                    row.get("bet_status")
                };
                let strategy_id: Option<String> = if item_type == "coupon" {
                    row.get("coupon_strategy_id")
                } else {
                    row.get("bet_strategy_id")
                };
                json!({
                    "id": row.get::<_, String>("id"),
                    "created_at": created_at,
                    "item_type": item_type,
                    "simulated_bet_id": row.get::<_, Option<String>>("simulated_bet_id"),
                    "simulated_coupon_id": row.get::<_, Option<String>>("simulated_coupon_id"),
                    "source": row.get::<_, String>("source"),
                    "observed_result": row.get::<_, String>("observed_result"),
                    "confidence": row.get::<_, f64>("confidence"),
                    "status": status,
                    "strategy_id": strategy_id,
                    "sport_key": row.get::<_, Option<String>>("bet_sport_key"),
                    "event_name": row.get::<_, Option<String>>("bet_event_name"),
                    "competition": row.get::<_, Option<String>>("bet_competition"),
                    "market_name": row.get::<_, Option<String>>("bet_market_name"),
                    "outcome_name": row.get::<_, Option<String>>("bet_outcome_name"),
                    "coupon_type": row.get::<_, Option<String>>("coupon_type"),
                    "leg_count": row.get::<_, Option<i32>>("leg_count"),
                    "source_policy": source_policy,
                    "payload": payload
                })
            }).collect::<Vec<_>>()
        }))
    }

    pub async fn settlement_lookup_attempts(&self, limit: i64) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT
                  sla.id,
                  sla.created_at,
                  sla.item_type,
                  sla.simulated_bet_id,
                  sla.simulated_coupon_id,
                  sla.source_key,
                  sla.recommendation,
                  sla.outcome_state,
                  sla.payload,
                  cb.event_name AS bet_event_name,
                  cb.market_name AS bet_market_name,
                  cb.outcome_name AS bet_outcome_name,
                  cc.coupon_type,
                  cc.leg_count
                FROM settlement_lookup_attempts sla
                LEFT JOIN simulated_bets sb ON sb.id = sla.simulated_bet_id
                LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                LEFT JOIN simulated_coupons sc ON sc.id = sla.simulated_coupon_id
                LEFT JOIN candidate_coupons cc ON cc.id = sc.coupon_id
                ORDER BY sla.created_at DESC
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        Ok(json!({
            "paper_only": true,
            "not_auto_graded": true,
            "items": rows.iter().map(|row| {
                let created_at: DateTime<Utc> = row.get("created_at");
                let item_type: String = row.get("item_type");
                json!({
                    "id": row.get::<_, String>("id"),
                    "created_at": created_at,
                    "item_type": item_type,
                    "simulated_bet_id": row.get::<_, Option<String>>("simulated_bet_id"),
                    "simulated_coupon_id": row.get::<_, Option<String>>("simulated_coupon_id"),
                    "source_key": row.get::<_, String>("source_key"),
                    "recommendation": row.get::<_, String>("recommendation"),
                    "outcome_state": row.get::<_, Value>("outcome_state"),
                    "payload": row.get::<_, Value>("payload"),
                    "event_name": row.get::<_, Option<String>>("bet_event_name"),
                    "market_name": row.get::<_, Option<String>>("bet_market_name"),
                    "outcome_name": row.get::<_, Option<String>>("bet_outcome_name"),
                    "coupon_type": row.get::<_, Option<String>>("coupon_type"),
                    "leg_count": row.get::<_, Option<i32>>("leg_count")
                })
            }).collect::<Vec<_>>()
        }))
    }

    pub async fn external_result_evidence(&self, limit: i64) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT
                  id,
                  created_at,
                  source_key,
                  source_url,
                  event_name,
                  home_name,
                  away_name,
                  home_score,
                  away_score,
                  confidence::float8 AS confidence,
                  used_for_settlement,
                  payload
                FROM external_result_evidence
                ORDER BY created_at DESC
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        Ok(json!({
            "paper_only": true,
            "items": rows.iter().map(|row| {
                let created_at: DateTime<Utc> = row.get("created_at");
                json!({
                    "id": row.get::<_, String>("id"),
                    "created_at": created_at,
                    "source_key": row.get::<_, Option<String>>("source_key"),
                    "source_url": row.get::<_, Option<String>>("source_url"),
                    "event_name": row.get::<_, String>("event_name"),
                    "home_name": row.get::<_, String>("home_name"),
                    "away_name": row.get::<_, String>("away_name"),
                    "home_score": row.get::<_, i32>("home_score"),
                    "away_score": row.get::<_, i32>("away_score"),
                    "confidence": row.get::<_, f64>("confidence"),
                    "used_for_settlement": row.get::<_, bool>("used_for_settlement"),
                    "payload": row.get::<_, Value>("payload")
                })
            }).collect::<Vec<_>>()
        }))
    }

    async fn record_settlement_lookup_attempts(
        &self,
        client: &Client,
        items: &[Value],
        settlement_source_policy: &Value,
        lookup_cooldown_minutes: i64,
    ) -> anyhow::Result<usize> {
        let mut recorded = 0;
        let cooldown_minutes = lookup_cooldown_minutes.max(0) as i32;
        for item in items {
            let item_type = item
                .get("item_type")
                .and_then(Value::as_str)
                .unwrap_or("single");
            let simulated_bet_id = if item_type == "single" {
                item.get("bet_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            } else {
                None
            };
            let simulated_coupon_id = if item_type == "coupon" {
                item.get("coupon_simulation_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            } else {
                None
            };
            if simulated_bet_id.is_none() && simulated_coupon_id.is_none() {
                continue;
            }
            let recommendation = item
                .get("recommendation")
                .and_then(Value::as_str)
                .unwrap_or("await_more_evidence")
                .to_string();
            let source_key = item
                .get("recommended_source_key")
                .and_then(Value::as_str)
                .unwrap_or("danskespil_content_service")
                .to_string();
            let outcome_state = json!({
                "event_status": item.get("event_status").cloned().unwrap_or(Value::Null),
                "event_resulted": item.get("event_resulted").cloned().unwrap_or(Value::Null),
                "event_settled": item.get("event_settled").cloned().unwrap_or(Value::Null),
                "expected_result_check_after": item.get("expected_result_check_after").cloned().unwrap_or(Value::Null),
                "overdue_minutes": item.get("overdue_minutes").cloned().unwrap_or(Value::Null),
                "recommended_source_key": source_key,
                "latest_outcome_active": item.get("latest_outcome_active").cloned().unwrap_or(Value::Null),
                "latest_outcome_displayed": item.get("latest_outcome_displayed").cloned().unwrap_or(Value::Null)
            });
            let payload = json!({
                "paper_only": true,
                "not_auto_graded": true,
                "source": source_key,
                "settlement_source_policy": settlement_source_policy,
                "review_item": item
            });
            client
                .execute(
                    r#"
                    INSERT INTO settlement_lookup_attempts (
                      id, item_type, simulated_bet_id, simulated_coupon_id, source_key,
                      recommendation, outcome_state, payload
                    )
                    SELECT $1,$2,$3,$4,$5,$6,$7,$8
                    WHERE NOT EXISTS (
                      SELECT 1
                      FROM settlement_lookup_attempts previous
                      WHERE previous.source_key = $5
                        AND previous.created_at >= now() - ($9::int * interval '1 minute')
                        AND (
                          ($3::text IS NOT NULL AND previous.simulated_bet_id = $3)
                          OR ($4::text IS NOT NULL AND previous.simulated_coupon_id = $4)
                        )
                    )
                    "#,
                    &[
                        &new_id(),
                        &item_type,
                        &simulated_bet_id,
                        &simulated_coupon_id,
                        &source_key,
                        &recommendation,
                        &outcome_state,
                        &payload,
                        &cooldown_minutes,
                    ],
                )
                .await
                .map(|count| recorded += count as usize)?;
        }
        Ok(recorded)
    }

    async fn settlement_source_record(&self, source_key: &str) -> anyhow::Result<Value> {
        if source_key.is_empty() {
            return Err(anyhow!("settlement source is required"));
        }
        let client = self.connect().await?;
        let row = client
            .query_opt(
                r#"
                SELECT
                  source_key,
                  source_name,
                  source_type,
                  url_pattern,
                  sport_scope,
                  reliability::float8 AS reliability,
                  manual_review_required,
                  notes,
                  last_seen_at,
                  payload
                FROM source_registry
                WHERE source_key = $1
                  AND can_settle = true
                "#,
                &[&source_key],
            )
            .await?
            .ok_or_else(|| anyhow!("unsupported settlement source: {source_key}"))?;
        let last_seen_at: DateTime<Utc> = row.get("last_seen_at");
        Ok(json!({
            "source_key": row.get::<_, String>("source_key"),
            "source_name": row.get::<_, String>("source_name"),
            "source_type": row.get::<_, String>("source_type"),
            "url_pattern": row.get::<_, Option<String>>("url_pattern"),
            "sport_scope": row.get::<_, Vec<String>>("sport_scope"),
            "reliability": row.get::<_, f64>("reliability"),
            "manual_review_required": row.get::<_, bool>("manual_review_required"),
            "notes": row.get::<_, Option<String>>("notes"),
            "last_seen_at": last_seen_at,
            "payload": row.get::<_, Value>("payload")
        }))
    }

    pub async fn settle_simulated_bet(
        &self,
        bet_id: &str,
        result: &str,
        source: &str,
        confidence: f64,
        notes: &str,
    ) -> anyhow::Result<SimulatedBet> {
        let status = match result {
            "won" => "settled_won",
            "lost" => "settled_lost",
            "void" => "void",
            "pushed" => "pushed",
            "cancelled" => "cancelled",
            "abandoned" => "abandoned",
            "refunded" => "refunded",
            "postponed" => "postponed",
            "unresolved" => "unresolved",
            _ => return Err(anyhow!("unsupported settlement result: {result}")),
        };
        let bet = self
            .simulated_bets(1000)
            .await?
            .into_iter()
            .find(|bet| bet.id == bet_id)
            .ok_or_else(|| anyhow!("simulated bet not found: {bet_id}"))?;
        if !matches!(
            bet.status.as_str(),
            "open" | "awaiting_result" | "unresolved" | "postponed"
        ) {
            return Err(anyhow!("simulated bet is already settled: {bet_id}"));
        }
        let source_record = self.settlement_source_record(source).await?;
        let (simulated_return, profit_loss) = match result {
            "won" => {
                let returned =
                    bet.hypothetical_stake * bet.observed_decimal_odds.unwrap_or_default();
                (Some(returned), Some(returned - bet.hypothetical_stake))
            }
            "lost" => (Some(0.0), Some(-bet.hypothetical_stake)),
            "void" | "pushed" | "cancelled" | "abandoned" | "refunded" => {
                (Some(bet.hypothetical_stake), Some(0.0))
            }
            _ => (None, None),
        };
        let manual_settlement = json!({
            "source": source,
            "source_policy": source_record,
            "observed_result": result,
            "confidence": confidence,
            "notes": notes,
            "settled_at": Utc::now(),
            "paper_only": true
        });
        let mut settlement_payload = bet.settlement_payload.clone();
        merge_json_object(
            &mut settlement_payload,
            json!({
                "source": source,
                "observed_result": result,
                "confidence": confidence,
                "notes": notes,
                "paper_only": true,
                "manual_settlement": manual_settlement
            }),
        );
        let settled_at = Utc::now();
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        transaction
            .execute(
                r#"
                UPDATE simulated_bets
                SET status = $1,
                    settled_at = $2,
                    simulated_return = ($3::float8)::numeric,
                    profit_loss = ($4::float8)::numeric,
                    settlement_payload = $5
                WHERE id = $6
                "#,
                &[
                    &status,
                    &settled_at,
                    &simulated_return,
                    &profit_loss,
                    &settlement_payload,
                    &bet_id,
                ],
            )
            .await?;
        transaction
            .execute(
                r#"
                INSERT INTO settlement_observations (id, simulated_bet_id, source, observed_result, confidence, payload)
                VALUES ($1,$2,$3,$4,($5::float8)::numeric,$6)
                "#,
                &[&new_id(), &bet_id, &source, &result, &confidence, &settlement_payload],
            )
            .await?;
        transaction.commit().await?;
        self.simulated_bets(1000)
            .await?
            .into_iter()
            .find(|bet| bet.id == bet_id)
            .ok_or_else(|| anyhow!("settled simulated bet not found"))
    }

    pub async fn settle_simulated_coupon(
        &self,
        coupon_id: &str,
        result: &str,
        source: &str,
        confidence: f64,
        notes: &str,
    ) -> anyhow::Result<Value> {
        let status = match result {
            "won" => "settled_won",
            "lost" => "settled_lost",
            "void" => "void",
            "pushed" => "pushed",
            "cancelled" => "cancelled",
            "abandoned" => "abandoned",
            "refunded" => "refunded",
            "postponed" => "postponed",
            "unresolved" => "unresolved",
            _ => return Err(anyhow!("unsupported settlement result: {result}")),
        };
        let coupon = self
            .simulated_coupons(1000)
            .await?
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|item| item.get("id").and_then(Value::as_str) == Some(coupon_id))
            .cloned()
            .ok_or_else(|| anyhow!("simulated coupon not found: {coupon_id}"))?;
        let current_status = coupon
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !matches!(
            current_status,
            "open" | "awaiting_result" | "unresolved" | "postponed"
        ) {
            return Err(anyhow!("simulated coupon is already settled: {coupon_id}"));
        }
        let stake = coupon
            .get("hypothetical_stake")
            .and_then(Value::as_f64)
            .unwrap_or_default();
        let combined_decimal_odds = coupon
            .get("observed_combined_decimal_odds")
            .and_then(Value::as_f64)
            .unwrap_or_default();
        let (simulated_return, profit_loss) = match result {
            "won" => {
                let returned = stake * combined_decimal_odds;
                (Some(returned), Some(returned - stake))
            }
            "lost" => (Some(0.0), Some(-stake)),
            "void" | "pushed" | "cancelled" | "abandoned" | "refunded" => (Some(stake), Some(0.0)),
            _ => (None, None),
        };
        let source_record = self.settlement_source_record(source).await?;
        let manual_settlement = json!({
            "source": source,
            "source_policy": source_record,
            "observed_result": result,
            "confidence": confidence,
            "notes": notes,
            "settled_at": Utc::now(),
            "paper_only": true,
            "coupon_level": true
        });
        let mut settlement_payload = coupon
            .get("settlement_payload")
            .cloned()
            .unwrap_or_else(|| json!({}));
        merge_json_object(
            &mut settlement_payload,
            json!({
                "source": source,
                "observed_result": result,
                "confidence": confidence,
                "notes": notes,
                "paper_only": true,
                "coupon_level": true,
                "manual_settlement": manual_settlement
            }),
        );
        let settled_at = Utc::now();
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        transaction
            .execute(
                r#"
                UPDATE simulated_coupons
                SET status = $1,
                    settled_at = $2,
                    simulated_return = ($3::float8)::numeric,
                    profit_loss = ($4::float8)::numeric,
                    settlement_payload = $5
                WHERE id = $6
                "#,
                &[
                    &status,
                    &settled_at,
                    &simulated_return,
                    &profit_loss,
                    &settlement_payload,
                    &coupon_id,
                ],
            )
            .await?;
        transaction
            .execute(
                r#"
                UPDATE simulated_coupon_legs
                SET status = $1,
                    settlement_payload = settlement_payload || $2
                WHERE simulated_coupon_id = $3
                "#,
                &[&status, &settlement_payload, &coupon_id],
            )
            .await?;
        transaction
            .execute(
                r#"
                INSERT INTO settlement_observations (
                  id, simulated_coupon_id, source, observed_result, confidence, payload
                )
                VALUES ($1,$2,$3,$4,($5::float8)::numeric,$6)
                "#,
                &[
                    &new_id(),
                    &coupon_id,
                    &source,
                    &result,
                    &confidence,
                    &settlement_payload,
                ],
            )
            .await?;
        transaction.commit().await?;
        self.simulated_coupons(1000)
            .await?
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|item| item.get("id").and_then(Value::as_str) == Some(coupon_id))
            .cloned()
            .ok_or_else(|| anyhow!("settled simulated coupon not found"))
    }

    pub async fn hermes_reflections(&self, limit: i64) -> anyhow::Result<Vec<HermesReflection>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT id, created_at, title, summary, evidence, status FROM hermes_reflections ORDER BY created_at DESC LIMIT $1",
                &[&limit],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| HermesReflection {
                id: row.get("id"),
                created_at: row.get("created_at"),
                title: row.get("title"),
                summary: row.get("summary"),
                evidence: row.get("evidence"),
                status: row.get("status"),
            })
            .collect())
    }

    pub async fn record_daily_reflection(&self, local_date: Option<&str>) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let local_date = match local_date {
            Some(value) if !value.trim().is_empty() => value.trim().to_string(),
            _ => {
                let row = client
                    .query_one(
                        "SELECT to_char(((now() AT TIME ZONE 'Europe/Copenhagen')::date - 1), 'YYYY-MM-DD') AS local_date",
                        &[],
                    )
                    .await?;
                row.get("local_date")
            }
        };
        let local_date_value = NaiveDate::parse_from_str(&local_date, "%Y-%m-%d")
            .with_context(|| format!("invalid reflection date: {local_date}"))?;
        let reflection_id = format!("daily-paper-reflection-{local_date}");

        let performance_row = client
            .query_one(
                r#"
                SELECT
                  count(*)::int AS snapshot_count,
                  min(created_at) AS first_snapshot_at,
                  max(created_at) AS last_snapshot_at
                FROM simulation_performance_snapshots
                WHERE (created_at AT TIME ZONE 'Europe/Copenhagen')::date = $1
                "#,
                &[&local_date_value],
            )
            .await?;
        let latest_performance = client
            .query_opt(
                r#"
                SELECT performance
                FROM simulation_performance_snapshots
                WHERE (created_at AT TIME ZONE 'Europe/Copenhagen')::date = $1
                ORDER BY created_at DESC
                LIMIT 1
                "#,
                &[&local_date_value],
            )
            .await?
            .map(|row| row.get::<_, Value>("performance"))
            .unwrap_or(Value::Null);
        let bet_row = client
            .query_one(
                r#"
                WITH scoped AS (
                  SELECT *
                  FROM simulated_bets
                  WHERE (created_at AT TIME ZONE 'Europe/Copenhagen')::date = $1
                ),
                status_counts AS (
                  SELECT status, count(*)::int AS count
                  FROM scoped
                  GROUP BY status
                )
                SELECT
                  (SELECT count(*)::int FROM scoped) AS placed_count,
                  COALESCE((SELECT sum(hypothetical_stake)::float8 FROM scoped), 0) AS turnover,
                  COALESCE((SELECT sum(profit_loss)::float8 FROM scoped), 0) AS profit_loss,
                  COALESCE((SELECT jsonb_object_agg(status, count) FROM status_counts), '{}'::jsonb) AS by_status
                "#,
                &[&local_date_value],
            )
            .await?;
        let coupon_row = client
            .query_one(
                r#"
                WITH scoped AS (
                  SELECT *
                  FROM simulated_coupons
                  WHERE (created_at AT TIME ZONE 'Europe/Copenhagen')::date = $1
                ),
                status_counts AS (
                  SELECT status, count(*)::int AS count
                  FROM scoped
                  GROUP BY status
                )
                SELECT
                  (SELECT count(*)::int FROM scoped) AS placed_count,
                  COALESCE((SELECT sum(hypothetical_stake)::float8 FROM scoped), 0) AS turnover,
                  COALESCE((SELECT sum(profit_loss)::float8 FROM scoped), 0) AS profit_loss,
                  COALESCE((SELECT jsonb_object_agg(status, count) FROM status_counts), '{}'::jsonb) AS by_status
                "#,
                &[&local_date_value],
            )
            .await?;
        let settlement_row = client
            .query_one(
                r#"
                SELECT count(*)::int AS observation_count
                FROM settlement_observations
                WHERE (created_at AT TIME ZONE 'Europe/Copenhagen')::date = $1
                "#,
                &[&local_date_value],
            )
            .await?;

        let snapshot_count: i32 = performance_row.get("snapshot_count");
        let bet_count: i32 = bet_row.get("placed_count");
        let bet_turnover: f64 = bet_row.get("turnover");
        let coupon_count: i32 = coupon_row.get("placed_count");
        let settlement_count: i32 = settlement_row.get("observation_count");
        let bet_statuses: Value = bet_row.get("by_status");
        let coupon_statuses: Value = coupon_row.get("by_status");
        let bet_realized_count = json_status_closed_count(&bet_statuses);
        let coupon_realized_count = json_status_closed_count(&coupon_statuses);
        let bet_open_count = json_status_open_count(&bet_statuses);
        let coupon_open_count = json_status_open_count(&coupon_statuses);
        let realized_count = bet_realized_count + coupon_realized_count;
        let open_count = bet_open_count + coupon_open_count;
        let total_profit_loss =
            bet_row.get::<_, f64>("profit_loss") + coupon_row.get::<_, f64>("profit_loss");
        let first_snapshot_at: Option<DateTime<Utc>> = performance_row.get("first_snapshot_at");
        let last_snapshot_at: Option<DateTime<Utc>> = performance_row.get("last_snapshot_at");
        let title = format!("Daily paper reflection {local_date}");
        let summary = if bet_count + coupon_count == 0 {
            format!("{local_date}: no paper placements were created. {snapshot_count} performance snapshots were recorded; no settlement observations were available.")
        } else if realized_count == 0 {
            format!("{local_date}: recorded {snapshot_count} performance snapshots and {bet_count} paper singles / {coupon_count} paper coupons. Paper single turnover was {bet_turnover:.2}. {open_count} paper positions remain open or awaiting result, so won/lost performance is not evaluable.")
        } else {
            format!("{local_date}: recorded {snapshot_count} performance snapshots, {bet_count} paper singles, {coupon_count} paper coupons, {realized_count} realized paper results, and paper P/L {total_profit_loss:.2}.")
        };
        let evidence = json!({
            "reflection_date": local_date,
            "timezone": "Europe/Copenhagen",
            "paper_only": true,
            "first_snapshot_at": first_snapshot_at,
            "last_snapshot_at": last_snapshot_at,
            "performance_snapshot_count": snapshot_count,
            "latest_performance": latest_performance,
            "single_bets": {
                "placed_count": bet_count,
                "turnover": bet_turnover,
                "profit_loss": bet_row.get::<_, f64>("profit_loss"),
                "by_status": bet_statuses,
                "open_count": bet_open_count,
                "realized_count": bet_realized_count
            },
            "coupons": {
                "placed_count": coupon_count,
                "turnover": coupon_row.get::<_, f64>("turnover"),
                "profit_loss": coupon_row.get::<_, f64>("profit_loss"),
                "by_status": coupon_statuses,
                "open_count": coupon_open_count,
                "realized_count": coupon_realized_count
            },
            "settlement_observation_count": settlement_count,
            "assessment": {
                "has_realized_results": realized_count > 0,
                "needs_result_review": realized_count == 0 && (bet_count + coupon_count) > 0,
                "open_or_awaiting_count": open_count,
                "realized_count": realized_count,
                "profit_loss": total_profit_loss,
                "recommendation": if realized_count == 0 && (bet_count + coupon_count) > 0 {
                    "Keep positions in awaiting-result review; do not promote strategy based on unresolved paper exposure."
                } else if bet_count + coupon_count == 0 {
                    "No paper results to evaluate for this date."
                } else if open_count > 0 {
                    "Evaluate realized rows, but keep strategy promotion blocked until all same-day paper exposure is settled or explicitly voided."
                } else {
                    "Review realized P/L and calibration before considering any one-variable strategy promotion."
                }
            }
        });
        let row = client
            .query_one(
                r#"
                INSERT INTO hermes_reflections (id, title, summary, evidence, status)
                VALUES ($1,$2,$3,$4,'recorded')
                ON CONFLICT (id) DO UPDATE
                SET created_at = now(),
                    title = EXCLUDED.title,
                    summary = EXCLUDED.summary,
                    evidence = EXCLUDED.evidence,
                    status = EXCLUDED.status
                RETURNING id, created_at, title, summary, evidence, status
                "#,
                &[&reflection_id, &title, &summary, &evidence],
            )
            .await?;
        let created_at: DateTime<Utc> = row.get("created_at");
        Ok(json!({
            "id": row.get::<_, String>("id"),
            "created_at": created_at,
            "title": row.get::<_, String>("title"),
            "summary": row.get::<_, String>("summary"),
            "evidence": row.get::<_, Value>("evidence"),
            "status": row.get::<_, String>("status")
        }))
    }

    pub async fn strategy_state(&self) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let baseline = client
            .query_opt(
                r#"
                SELECT id, created_at, strategy_id, version, status, active, config,
                       promoted_from_experiment_id, notes
                FROM strategy_baselines
                WHERE active = true
                ORDER BY created_at DESC
                LIMIT 1
                "#,
                &[],
            )
            .await?;
        let experiments = client
            .query(
                r#"
                SELECT id, created_at, updated_at, title, hypothesis, variable_name,
                       baseline_value, proposed_value, baseline_strategy_id, status,
                       evidence, decision_payload
                FROM strategy_experiments
                ORDER BY
                  CASE status
                    WHEN 'proposed' THEN 0
                    WHEN 'approved_for_replay' THEN 1
                    WHEN 'active_simulation' THEN 2
                    WHEN 'promoted' THEN 3
                    ELSE 4
                  END,
                  created_at DESC
                LIMIT 25
                "#,
                &[],
            )
            .await?;
        Ok(json!({
            "active_baseline": baseline.map(|row| {
                let created_at: DateTime<Utc> = row.get("created_at");
                json!({
                    "id": row.get::<_, String>("id"),
                    "created_at": created_at,
                    "strategy_id": row.get::<_, String>("strategy_id"),
                    "version": row.get::<_, i32>("version"),
                    "status": row.get::<_, String>("status"),
                    "active": row.get::<_, bool>("active"),
                    "config": row.get::<_, Value>("config"),
                    "promoted_from_experiment_id": row.get::<_, Option<String>>("promoted_from_experiment_id"),
                    "notes": row.get::<_, Option<String>>("notes")
                })
            }),
            "experiments": experiments.iter().map(|row| {
                let created_at: DateTime<Utc> = row.get("created_at");
                let updated_at: DateTime<Utc> = row.get("updated_at");
                json!({
                    "id": row.get::<_, String>("id"),
                    "created_at": created_at,
                    "updated_at": updated_at,
                    "title": row.get::<_, String>("title"),
                    "hypothesis": row.get::<_, String>("hypothesis"),
                    "variable_name": row.get::<_, String>("variable_name"),
                    "baseline_value": row.get::<_, Value>("baseline_value"),
                    "proposed_value": row.get::<_, Value>("proposed_value"),
                    "baseline_strategy_id": row.get::<_, String>("baseline_strategy_id"),
                    "status": row.get::<_, String>("status"),
                    "evidence": row.get::<_, Value>("evidence"),
                    "decision_payload": row.get::<_, Value>("decision_payload")
                })
            }).collect::<Vec<_>>()
        }))
    }

    pub async fn ensure_scan_strategy_proposal(
        &self,
        snapshot_id: &str,
        candidates: &[CandidateBet],
    ) -> anyhow::Result<Option<Value>> {
        let client = self.connect().await?;
        let existing = client
            .query_opt(
                r#"
                SELECT id, created_at, updated_at, title, hypothesis, variable_name,
                       baseline_value, proposed_value, baseline_strategy_id, status,
                       evidence, decision_payload
                FROM strategy_experiments
                WHERE status IN ('proposed', 'approved_for_replay', 'active_simulation')
                ORDER BY created_at DESC
                LIMIT 1
                "#,
                &[],
            )
            .await?;
        if let Some(row) = existing {
            return Ok(Some(strategy_experiment_from_row(&row)));
        }

        let long_price_candidates: Vec<&CandidateBet> = candidates
            .iter()
            .filter(|candidate| {
                candidate.decimal_odds.unwrap_or_default() > 8.0
                    || candidate
                        .risk_flags
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|flag| flag.as_str() == Some("long_price"))
            })
            .collect();
        if long_price_candidates.len() >= 3 {
            return self
                .insert_strategy_experiment(
                    snapshot_id,
                    "Cap long-price candidates",
                    "Reducing the maximum decimal odds considered by poc_ranker_v1 may lower noisy long-shot paper candidates until settlement history supports them.",
                    "max_decimal_odds",
                    json!(8.0),
                    json!(6.0),
                    candidates,
                    &long_price_candidates,
                    "long_price_candidate_count",
                )
                .await;
        }

        let specialized_candidates: Vec<&CandidateBet> = candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .risk_flags
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|flag| flag.as_str() == Some("specialized_market"))
            })
            .collect();
        if specialized_candidates.len() >= 3 {
            return self
                .insert_strategy_experiment(
                    snapshot_id,
                    "Exclude specialized markets",
                    "Temporarily excluding specialized markets may improve paper-simulation interpretability until market-specific settlement and feature coverage are stronger.",
                    "excluded_market_kinds",
                    json!([]),
                    json!(["goal", "corners", "half_time", "period_or_quarter", "set_or_game"]),
                    candidates,
                    &specialized_candidates,
                    "specialized_market_candidate_count",
                )
                .await;
        }

        let large_movement_candidates: Vec<&CandidateBet> = candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .risk_flags
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|flag| flag.as_str() == Some("large_odds_movement"))
            })
            .collect();
        if large_movement_candidates.len() >= 3 {
            return self
                .insert_strategy_experiment(
                    snapshot_id,
                    "Exclude large odds moves",
                    "Temporarily excluding candidates with large latest-prior odds movement may reduce paper simulations based on unstable prices until movement-specific calibration has enough settled evidence.",
                    "excluded_risk_flags",
                    json!([]),
                    json!(["large_odds_movement"]),
                    candidates,
                    &large_movement_candidates,
                    "large_odds_movement_candidate_count",
                )
                .await;
        }

        let baseline = client
            .query_opt(
                r#"
                SELECT config
                FROM strategy_baselines
                WHERE strategy_id = 'poc_ranker_v1' AND active = true
                ORDER BY created_at DESC
                LIMIT 1
                "#,
                &[],
            )
            .await?;
        let baseline_config: Option<Value> = baseline.map(|row| row.get("config"));
        let coupon_modes = baseline_config
            .as_ref()
            .and_then(|config| config.get("coupon_modes"))
            .cloned()
            .unwrap_or_else(|| {
                json!({
                    "single": true,
                    "double": false,
                    "triple": false,
                    "accumulator": false,
                    "max_legs": 1,
                    "require_provider_accumulator_support": true,
                    "require_same_sport_or_category_when_provider_requires_it": true
                })
            });
        let doubles_enabled = coupon_modes
            .get("double")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !doubles_enabled {
            let mut by_sport: BTreeMap<&str, Vec<&CandidateBet>> = BTreeMap::new();
            for candidate in candidates
                .iter()
                .filter(|candidate| accumulator_allowed(candidate, 2))
            {
                by_sport
                    .entry(candidate.sport_key.as_str())
                    .or_default()
                    .push(candidate);
            }
            let supported_double_candidates: Vec<&CandidateBet> = by_sport
                .values()
                .find(|items| {
                    let distinct_event_count = items
                        .iter()
                        .filter_map(|candidate| candidate.event_id.as_deref())
                        .collect::<HashSet<_>>()
                        .len();
                    distinct_event_count >= 2
                })
                .cloned()
                .unwrap_or_default();
            if supported_double_candidates.len() >= 2 {
                let mut proposed_coupon_modes = coupon_modes.clone();
                if let Some(object) = proposed_coupon_modes.as_object_mut() {
                    object.insert("double".to_string(), json!(true));
                    object.insert("max_legs".to_string(), json!(2));
                    object.insert("triple".to_string(), json!(false));
                    object.insert("accumulator".to_string(), json!(false));
                    object.insert(
                        "require_provider_accumulator_support".to_string(),
                        json!(true),
                    );
                    object.insert(
                        "require_same_sport_or_category_when_provider_requires_it".to_string(),
                        json!(true),
                    );
                }
                return self
                    .insert_strategy_experiment(
                        snapshot_id,
                        "Enable paper doubles",
                        "Provider accumulator metadata shows at least two same-sport, distinct-event selections that can be combined. Enabling paper doubles lets the simulator evaluate coupon behavior without changing real-money safety gates.",
                        "coupon_modes",
                        coupon_modes,
                        proposed_coupon_modes,
                        candidates,
                        &supported_double_candidates,
                        "provider_supported_double_candidate_count",
                    )
                    .await;
            }
        }

        Ok(None)
    }

    async fn insert_strategy_experiment(
        &self,
        snapshot_id: &str,
        title: &str,
        hypothesis: &str,
        variable_name: &str,
        baseline_value: Value,
        proposed_value: Value,
        candidates: &[CandidateBet],
        evidence_candidates: &[&CandidateBet],
        evidence_count_key: &str,
    ) -> anyhow::Result<Option<Value>> {
        if evidence_candidates.is_empty() {
            return Ok(None);
        }

        let client = self.connect().await?;
        let id = new_id();
        let baseline_strategy_id = "poc_ranker_v1".to_string();
        let status = "proposed".to_string();
        let evidence = json!({
            "source": "scan_candidate_risk_review",
            "snapshot_id": snapshot_id,
            "candidate_count": candidates.len(),
            evidence_count_key: evidence_candidates.len(),
            "examples": evidence_candidates.iter().take(5).map(|candidate| json!({
                "candidate_id": candidate.id,
                "sport_key": candidate.sport_key,
                "event_name": candidate.event_name,
                "market_name": candidate.market_name,
                "outcome_name": candidate.outcome_name,
                "decimal_odds": candidate.decimal_odds,
                "score": candidate.score,
                "risk_flags": candidate.risk_flags,
                "odds_movement": candidate.feature_snapshot.get("odds_movement").cloned().unwrap_or(Value::Null)
            })).collect::<Vec<_>>(),
            "safety": {
                "paper_only": true,
                "one_variable_only": true,
                "requires_operator_review": true,
                "does_not_enable_real_money": true
            }
        });
        client
            .execute(
                r#"
                INSERT INTO strategy_experiments (
                  id, title, hypothesis, variable_name, baseline_value, proposed_value,
                  baseline_strategy_id, status, evidence
                )
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
                "#,
                &[
                    &id,
                    &title,
                    &hypothesis,
                    &variable_name,
                    &baseline_value,
                    &proposed_value,
                    &baseline_strategy_id,
                    &status,
                    &evidence,
                ],
            )
            .await?;
        self.record_audit(
            "strategy_experiment_proposed",
            json!({"experiment_id": id, "snapshot_id": snapshot_id, "variable_name": variable_name}),
        )
        .await
        .ok();
        self.strategy_state()
            .await?
            .get("experiments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|item| item.get("id").and_then(Value::as_str) == Some(id.as_str()))
            .cloned()
            .map(Some)
            .ok_or_else(|| anyhow!("created strategy experiment not found"))
    }

    async fn strategy_experiment_replay(&self, experiment_id: &str) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let experiment = client
            .query_one(
                r#"
                SELECT variable_name, proposed_value, baseline_strategy_id, evidence
                FROM strategy_experiments
                WHERE id = $1
                "#,
                &[&experiment_id],
            )
            .await
            .context("strategy experiment not found")?;
        let variable_name: String = experiment.get("variable_name");
        let proposed_value: Value = experiment.get("proposed_value");
        let baseline_strategy_id: String = experiment.get("baseline_strategy_id");
        let evidence: Value = experiment.get("evidence");
        let baseline = client
            .query_one(
                r#"
                SELECT id, version, config
                FROM strategy_baselines
                WHERE strategy_id = $1 AND active = true
                ORDER BY created_at DESC
                LIMIT 1
                "#,
                &[&baseline_strategy_id],
            )
            .await
            .context("active baseline not found")?;
        let baseline_id: String = baseline.get("id");
        let baseline_version: i32 = baseline.get("version");
        let baseline_config: Value = baseline.get("config");
        let proposed_config =
            strategy_config_with_change(&baseline_config, &variable_name, proposed_value);
        let evidence_snapshot_id = evidence.get("snapshot_id").and_then(Value::as_str);
        let rows = if let Some(snapshot_id) = evidence_snapshot_id {
            client
                .query(
                    r#"
                    SELECT id, snapshot_id, created_at, sport_key, event_id, event_name, competition,
                           market_id, market_name, market_kind, outcome_id, outcome_name,
                           decimal_odds::float8 AS decimal_odds, rationale,
                           implied_probability::float8 AS implied_probability,
                           model_probability::float8 AS model_probability,
                           expected_value::float8 AS expected_value,
                           confidence::float8 AS confidence,
                           score::float8 AS score,
                           risk_flags, feature_snapshot, status
                    FROM candidate_bets
                    WHERE snapshot_id = $1
                    ORDER BY score DESC NULLS LAST, created_at ASC
                    LIMIT 500
                    "#,
                    &[&snapshot_id],
                )
                .await?
        } else {
            client
                .query(
                    r#"
                    SELECT id, snapshot_id, created_at, sport_key, event_id, event_name, competition,
                           market_id, market_name, market_kind, outcome_id, outcome_name,
                           decimal_odds::float8 AS decimal_odds, rationale,
                           implied_probability::float8 AS implied_probability,
                           model_probability::float8 AS model_probability,
                           expected_value::float8 AS expected_value,
                           confidence::float8 AS confidence,
                           score::float8 AS score,
                           risk_flags, feature_snapshot, status
                    FROM candidate_bets
                    ORDER BY created_at DESC, score DESC NULLS LAST
                    LIMIT 500
                    "#,
                    &[],
                )
                .await?
        };
        let candidates: Vec<CandidateBet> = rows.iter().map(candidate_from_row).collect();
        let baseline_eval = strategy_eval_for_config(&candidates, &baseline_config);
        let proposed_eval = strategy_eval_for_config(&candidates, &proposed_config);
        let candidate_count = candidates.len();

        let mut newly_selected = Vec::new();
        let mut newly_rejected = Vec::new();
        for candidate in &candidates {
            let baseline_decision = baseline_eval
                .decisions
                .get(candidate.id.as_str())
                .map(|decision| decision.decision.as_str())
                .unwrap_or("rejected");
            let proposed_decision = proposed_eval
                .decisions
                .get(candidate.id.as_str())
                .map(|decision| decision.decision.as_str())
                .unwrap_or("rejected");
            if baseline_decision != proposed_decision && proposed_decision == "selected" {
                newly_selected.push(strategy_replay_candidate_example(candidate));
            }
            if baseline_decision != proposed_decision && proposed_decision == "rejected" {
                let mut example = strategy_replay_candidate_example(candidate);
                if let Some(decision) = proposed_eval.decisions.get(candidate.id.as_str()) {
                    example["rejection_reasons"] = json!(decision.rejection_reasons);
                }
                newly_rejected.push(example);
            }
        }

        let replay_evidence = json!({
            "source": "strategy_experiment_replay",
            "experiment_id": experiment_id,
            "replayed_at": Utc::now(),
            "paper_only": true,
            "baseline_strategy_id": baseline_strategy_id,
            "baseline_id": baseline_id,
            "baseline_version": baseline_version,
            "variable_name": variable_name,
            "candidate_count": candidate_count,
            "snapshot_id": evidence_snapshot_id,
            "baseline": baseline_eval.summary_json(),
            "proposed": proposed_eval.summary_json(),
            "delta": {
                "selected_count": proposed_eval.selected_count as i64 - baseline_eval.selected_count as i64,
                "rejected_count": proposed_eval.rejected_count as i64 - baseline_eval.rejected_count as i64,
                "newly_selected_count": newly_selected.len(),
                "newly_rejected_count": newly_rejected.len()
            },
            "examples": {
                "newly_selected": newly_selected.into_iter().take(8).collect::<Vec<_>>(),
                "newly_rejected": newly_rejected.into_iter().take(8).collect::<Vec<_>>()
            },
            "safety": {
                "does_not_enable_real_money": true,
                "does_not_place_paper_bets": true,
                "requires_operator_review": true
            }
        });
        client
            .execute(
                r#"
                UPDATE strategy_experiments
                SET updated_at = now(),
                    decision_payload = decision_payload || jsonb_build_object('replay_evidence', $1::jsonb)
                WHERE id = $2
                "#,
                &[&replay_evidence, &experiment_id],
            )
            .await?;
        Ok(replay_evidence)
    }

    pub async fn refresh_hermes_experiment_replays(&self, limit: i64) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT id, title, status, updated_at
                FROM strategy_experiments
                WHERE status IN ('proposed', 'approved_for_replay', 'active_simulation')
                ORDER BY updated_at ASC
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        drop(client);

        let mut refreshed = Vec::new();
        let mut skipped = Vec::new();
        for row in rows {
            let experiment_id: String = row.get("id");
            let title: String = row.get("title");
            let status: String = row.get("status");
            match self.strategy_experiment_replay(&experiment_id).await {
                Ok(replay_evidence) => refreshed.push(json!({
                    "experiment_id": experiment_id,
                    "title": title,
                    "status": status,
                    "replay_evidence": replay_evidence
                })),
                Err(error) => skipped.push(json!({
                    "experiment_id": experiment_id,
                    "title": title,
                    "status": status,
                    "reason": "replay_failed",
                    "error": error.to_string()
                })),
            }
        }

        Ok(json!({
            "paper_only": true,
            "does_not_place_paper_bets": true,
            "does_not_change_experiment_status": true,
            "requested_limit": limit,
            "refreshed_count": refreshed.len(),
            "skipped_count": skipped.len(),
            "refreshed": refreshed,
            "skipped": skipped
        }))
    }

    pub async fn review_strategy_experiment(
        &self,
        experiment_id: &str,
        action: &str,
        notes: &str,
    ) -> anyhow::Result<Value> {
        if experiment_id.is_empty() {
            return Err(anyhow!("experiment_id is required"));
        }
        let replay_evidence = if matches!(action, "replay" | "activate" | "promote") {
            Some(self.strategy_experiment_replay(experiment_id).await?)
        } else {
            None
        };
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let previous = transaction
            .query_one(
                "SELECT status, variable_name, proposed_value, baseline_strategy_id FROM strategy_experiments WHERE id = $1",
                &[&experiment_id],
            )
            .await
            .context("strategy experiment not found")?;
        let previous_status: String = previous.get("status");
        let variable_name: String = previous.get("variable_name");
        let proposed_value: Value = previous.get("proposed_value");
        let baseline_strategy_id: String = previous.get("baseline_strategy_id");
        let status = match action {
            "approve" => "approved_for_replay".to_string(),
            "reject" => "rejected".to_string(),
            "replay" => previous_status.clone(),
            "activate" => "active_simulation".to_string(),
            "promote" => "promoted".to_string(),
            "rollback" => "rolled_back".to_string(),
            _ => return Err(anyhow!("unsupported experiment review action: {action}")),
        };
        let mut decision_payload = json!({
            "action": action,
            "previous_status": previous_status,
            "notes": notes,
            "reviewed_at": Utc::now(),
            "paper_only": true
        });
        if let Some(replay_evidence) = replay_evidence {
            decision_payload["replay_evidence"] = replay_evidence;
        }
        transaction
            .execute(
                r#"
                UPDATE strategy_experiments
                SET status = $1,
                    updated_at = now(),
                    decision_payload = decision_payload || $2
                WHERE id = $3
                "#,
                &[&status, &decision_payload, &experiment_id],
            )
            .await?;
        transaction
            .execute(
                r#"
                INSERT INTO web_review_events (id, subject_type, subject_id, action, notes, payload)
                VALUES ($1,$2,$3,$4,$5,$6)
                "#,
                &[
                    &new_id(),
                    &"strategy_experiment",
                    &experiment_id,
                    &action,
                    &notes,
                    &decision_payload,
                ],
            )
            .await?;
        if action == "promote" {
            let baseline = transaction
                .query_one(
                    "SELECT config, version FROM strategy_baselines WHERE strategy_id = $1 AND active = true LIMIT 1",
                    &[&baseline_strategy_id],
                )
                .await?;
            let mut config: Value = baseline.get("config");
            let version: i32 = baseline.get("version");
            if let Some(object) = config.as_object_mut() {
                object.insert(variable_name.clone(), proposed_value.clone());
            }
            transaction
                .execute(
                    "UPDATE strategy_baselines SET active = false, status = 'superseded' WHERE strategy_id = $1 AND active = true",
                    &[&baseline_strategy_id],
                )
                .await?;
            transaction
                .execute(
                    r#"
                    INSERT INTO strategy_baselines (
                      id, strategy_id, version, status, active, config,
                      promoted_from_experiment_id, notes
                    )
                    VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                    "#,
                    &[
                        &new_id(),
                        &baseline_strategy_id,
                        &(version + 1),
                        &"active",
                        &true,
                        &config,
                        &experiment_id,
                        &notes,
                    ],
                )
                .await?;
        }
        transaction.commit().await?;
        self.strategy_state()
            .await?
            .get("experiments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|item| item.get("id").and_then(Value::as_str) == Some(experiment_id))
            .cloned()
            .ok_or_else(|| anyhow!("reviewed strategy experiment not found"))
    }

    pub async fn record_audit(&self, event_type: &str, details: Value) -> anyhow::Result<()> {
        let client = self.connect().await?;
        client
            .execute(
                "INSERT INTO audit_events (id, event_type, details) VALUES ($1,$2,$3)",
                &[&new_id(), &event_type, &details],
            )
            .await?;
        Ok(())
    }

    pub async fn audit_events(&self, limit: i64) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT id, created_at, event_type, details
                FROM audit_events
                ORDER BY created_at DESC
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        Ok(json!({
            "items": rows.iter().map(|row| {
                let created_at: DateTime<Utc> = row.get("created_at");
                json!({
                    "id": row.get::<_, String>("id"),
                    "created_at": created_at,
                    "event_type": row.get::<_, String>("event_type"),
                    "details": row.get::<_, Value>("details")
                })
            }).collect::<Vec<_>>()
        }))
    }

    pub async fn latest_audit_event(&self, event_type: &str) -> anyhow::Result<Option<Value>> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                r#"
                SELECT id, created_at, event_type, details
                FROM audit_events
                WHERE event_type = $1
                ORDER BY created_at DESC
                LIMIT 1
                "#,
                &[&event_type],
            )
            .await?;
        Ok(row.map(|row| {
            let created_at: DateTime<Utc> = row.get("created_at");
            json!({
                "id": row.get::<_, String>("id"),
                "created_at": created_at,
                "event_type": row.get::<_, String>("event_type"),
                "details": row.get::<_, Value>("details")
            })
        }))
    }

    pub async fn audit_events_by_type(
        &self,
        event_type: &str,
        limit: i64,
    ) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT id, created_at, event_type, details
                FROM audit_events
                WHERE event_type = $1
                ORDER BY created_at DESC
                LIMIT $2
                "#,
                &[&event_type, &limit],
            )
            .await?;
        Ok(json!({
            "items": rows.iter().map(|row| {
                let created_at: DateTime<Utc> = row.get("created_at");
                json!({
                    "id": row.get::<_, String>("id"),
                    "created_at": created_at,
                    "event_type": row.get::<_, String>("event_type"),
                    "details": row.get::<_, Value>("details")
                })
            }).collect::<Vec<_>>()
        }))
    }

    async fn connect(&self) -> anyhow::Result<Client> {
        let database_url = self
            .database_url
            .as_ref()
            .ok_or_else(|| anyhow!("DATABASE_URL unavailable"))?;
        let (client, connection) = tokio_postgres::connect(database_url, NoTls).await?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::warn!(%error, "postgres connection task ended");
            }
        });
        Ok(client)
    }
}

async fn save_source_registry(
    transaction: &Transaction<'_>,
    sport_keys: &[String],
) -> anyhow::Result<()> {
    transaction
        .execute(
            r#"
            INSERT INTO source_registry (
              source_key, source_name, source_type, url_pattern, sport_scope,
              reliability, can_settle, manual_review_required, notes, payload
            )
            VALUES ($1,$2,$3,$4,$5,($6::float8)::numeric,$7,$8,$9,$10)
            ON CONFLICT (source_key) DO UPDATE
            SET sport_scope = EXCLUDED.sport_scope,
                last_seen_at = now(),
                payload = EXCLUDED.payload
            "#,
            &[
                &"danskespil_content_service",
                &"Danske Spil content-service",
                &"market_snapshot",
                &"https://content.sb.danskespil.dk/content-service/api/v1/q/*",
                &sport_keys,
                &0.78_f64,
                &false,
                &true,
                &"Read-only anonymous market metadata source. Useful for odds, markets, and event state; not sufficient alone for final settlement.",
                &json!({"runtime": "rust-dioxus", "paper_only": true}),
            ],
        )
        .await?;
    Ok(())
}

async fn save_market_catalog(
    transaction: &Transaction<'_>,
    snapshot_id: &str,
    payload: &Value,
) -> anyhow::Result<()> {
    for sport in payload
        .get("sports")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(sport_key) = text(sport, "sport_key") else {
            continue;
        };
        let sport_codes: Vec<String> = sport
            .get("sport_codes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        transaction
            .execute(
                r#"
                INSERT INTO sports (sport_key, label, drilldown_id, sport_codes, payload)
                VALUES ($1,$2,$3,$4,$5)
                ON CONFLICT (sport_key) DO UPDATE
                SET label = EXCLUDED.label,
                    drilldown_id = EXCLUDED.drilldown_id,
                    sport_codes = EXCLUDED.sport_codes,
                    last_seen_at = now(),
                    payload = EXCLUDED.payload
                "#,
                &[
                    &sport_key,
                    &text(sport, "label"),
                    &text(sport, "drilldown_id"),
                    &sport_codes,
                    sport,
                ],
            )
            .await?;

        for event in sport_events(sport) {
            let Some(event_id) = text(event, "id") else {
                continue;
            };
            let competition = text(event, "competition");
            if let Some(name) = competition {
                transaction
                    .execute(
                        r#"
                        INSERT INTO competitions (id, sport_key, name, class_name, drilldown_tag_id, payload)
                        VALUES ($1,$2,$3,$4,$5,$6)
                        ON CONFLICT (sport_key, name) DO UPDATE
                        SET class_name = EXCLUDED.class_name,
                            drilldown_tag_id = EXCLUDED.drilldown_tag_id,
                            last_seen_at = now(),
                            payload = EXCLUDED.payload
                        "#,
                        &[
                            &new_id(),
                            &sport_key,
                            &name,
                            &text(event, "class_name"),
                            &text(event, "competition_drilldown_tag_id"),
                            event,
                        ],
                    )
                    .await?;
            }

            let start_time = parse_datetime(event.get("start_time"));
            transaction
                .execute(
                    r#"
                    INSERT INTO sport_events (
                      id, sport_key, competition_name, event_name, start_time, status,
                      live_now, started, resulted, settled, payload
                    )
                    VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
                    ON CONFLICT (id) DO UPDATE
                    SET sport_key = EXCLUDED.sport_key,
                        competition_name = EXCLUDED.competition_name,
                        event_name = EXCLUDED.event_name,
                        start_time = EXCLUDED.start_time,
                        status = EXCLUDED.status,
                        live_now = EXCLUDED.live_now,
                        started = EXCLUDED.started,
                        resulted = EXCLUDED.resulted,
                        settled = EXCLUDED.settled,
                        last_seen_at = now(),
                        payload = EXCLUDED.payload
                    "#,
                    &[
                        &event_id,
                        &sport_key,
                        &competition,
                        &text(event, "name"),
                        &start_time,
                        &text(event, "status"),
                        &bool_value(event, "live_now"),
                        &bool_value(event, "started"),
                        &bool_value(event, "resulted"),
                        &bool_value(event, "settled"),
                        event,
                    ],
                )
                .await?;

            for participant in event
                .get("teams")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let name = participant
                    .get("name")
                    .and_then(Value::as_str)
                    .or_else(|| participant.get("fullName").and_then(Value::as_str));
                let Some(name) = name else {
                    continue;
                };
                let role = participant
                    .get("roleCode")
                    .and_then(Value::as_str)
                    .or_else(|| participant.get("role").and_then(Value::as_str));
                transaction
                    .execute(
                        r#"
                        INSERT INTO event_participants (id, event_id, name, role, payload)
                        VALUES ($1,$2,$3,$4,$5)
                        ON CONFLICT (event_id, name, role) DO UPDATE
                        SET last_seen_at = now(), payload = EXCLUDED.payload
                        "#,
                        &[&new_id(), &event_id, &name, &role, participant],
                    )
                    .await?;
            }

            for market in event
                .get("markets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let market_observation_id = new_id();
                let row = transaction
                    .query_one(
                        r#"
                        INSERT INTO market_observations (
                          id, snapshot_id, event_id, market_id, market_name, market_kind,
                          group_code, active, displayed, bet_in_run, outcome_count, payload
                        )
                        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
                        ON CONFLICT (snapshot_id, event_id, market_id) DO UPDATE
                        SET market_name = EXCLUDED.market_name,
                            market_kind = EXCLUDED.market_kind,
                            group_code = EXCLUDED.group_code,
                            active = EXCLUDED.active,
                            displayed = EXCLUDED.displayed,
                            bet_in_run = EXCLUDED.bet_in_run,
                            outcome_count = EXCLUDED.outcome_count,
                            payload = EXCLUDED.payload
                        RETURNING id
                        "#,
                        &[
                            &market_observation_id,
                            &snapshot_id,
                            &event_id,
                            &text(market, "id"),
                            &text(market, "name"),
                            &text(market, "kind"),
                            &text(market, "group_code"),
                            &optional_bool(market, "active"),
                            &optional_bool(market, "displayed"),
                            &optional_bool(market, "bet_in_run"),
                            &market
                                .get("outcomes")
                                .and_then(Value::as_array)
                                .map(|items| items.len() as i32)
                                .unwrap_or_default(),
                            market,
                        ],
                    )
                    .await?;
                let stored_market_id: String = row.get("id");
                let minimum_accumulator = json_i32(market.get("minimum_accumulator"));
                let maximum_accumulator = json_i32(market.get("maximum_accumulator"));
                if minimum_accumulator.is_some() || maximum_accumulator.is_some() {
                    transaction
                        .execute(
                            r#"
                            INSERT INTO coupon_rule_observations (
                              id, snapshot_id, sport_key, event_id, market_observation_id,
                              market_id, market_name, market_kind, group_code, competition_name,
                              minimum_accumulator, maximum_accumulator, restriction_scope, payload
                            )
                            VALUES (
                              $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14
                            )
                            ON CONFLICT (snapshot_id, event_id, market_id) DO UPDATE
                            SET market_observation_id = EXCLUDED.market_observation_id,
                                market_name = EXCLUDED.market_name,
                                market_kind = EXCLUDED.market_kind,
                                group_code = EXCLUDED.group_code,
                                competition_name = EXCLUDED.competition_name,
                                minimum_accumulator = EXCLUDED.minimum_accumulator,
                                maximum_accumulator = EXCLUDED.maximum_accumulator,
                                restriction_scope = EXCLUDED.restriction_scope,
                                observed_at = now(),
                                payload = EXCLUDED.payload
                            "#,
                            &[
                                &new_id(),
                                &snapshot_id,
                                &sport_key,
                                &event_id,
                                &stored_market_id,
                                &text(market, "id"),
                                &text(market, "name"),
                                &text(market, "kind"),
                                &text(market, "group_code"),
                                &competition,
                                &minimum_accumulator,
                                &maximum_accumulator,
                                &"same_sport_market_metadata",
                                &json!({
                                    "source": "normalized_danskespil_market_metadata",
                                    "paper_only": true,
                                    "sport_key": sport_key,
                                    "event_id": event_id,
                                    "market": market,
                                    "known_limits": {
                                        "minimum_accumulator": minimum_accumulator,
                                        "maximum_accumulator": maximum_accumulator
                                    },
                                    "unknown_restrictions": [
                                        "cross_sport_combination_rules",
                                        "cross_category_combination_rules",
                                        "market_exclusion_pairs"
                                    ]
                                }),
                            ],
                        )
                        .await?;
                }

                for outcome in market
                    .get("outcomes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    transaction
                        .execute(
                            r#"
                            INSERT INTO outcome_observations (
                              id, snapshot_id, market_observation_id, outcome_id, outcome_name,
                              outcome_type, outcome_sub_type, decimal_odds, active, displayed,
                              handicap_low, handicap_high, payload
                            )
                            VALUES (
                              $1,$2,$3,$4,$5,$6,$7,
                              ($8::float8)::numeric,
                              $9,$10,
                              ($11::float8)::numeric,
                              ($12::float8)::numeric,
                              $13
                            )
                            ON CONFLICT (snapshot_id, market_observation_id, outcome_id) DO UPDATE
                            SET outcome_name = EXCLUDED.outcome_name,
                                outcome_type = EXCLUDED.outcome_type,
                                outcome_sub_type = EXCLUDED.outcome_sub_type,
                                decimal_odds = EXCLUDED.decimal_odds,
                                active = EXCLUDED.active,
                                displayed = EXCLUDED.displayed,
                                handicap_low = EXCLUDED.handicap_low,
                                handicap_high = EXCLUDED.handicap_high,
                                payload = EXCLUDED.payload
                            "#,
                            &[
                                &new_id(),
                                &snapshot_id,
                                &stored_market_id,
                                &text(outcome, "id"),
                                &text(outcome, "name"),
                                &text(outcome, "type"),
                                &text(outcome, "sub_type"),
                                &number(outcome, "decimal_odds"),
                                &optional_bool(outcome, "active"),
                                &optional_bool(outcome, "displayed"),
                                &number(outcome, "handicap_low"),
                                &number(outcome, "handicap_high"),
                                outcome,
                            ],
                        )
                        .await?;
                }
            }
            let features = event_feature_snapshot(sport_key, event);
            transaction
                .execute(
                    r#"
                    INSERT INTO feature_snapshots (
                      id, snapshot_id, event_id, sport_key, feature_set, source_key,
                      confidence, missing_signals, features
                    )
                    VALUES ($1,$2,$3,$4,$5,$6,($7::float8)::numeric,$8,$9)
                    ON CONFLICT (snapshot_id, event_id, feature_set) DO UPDATE
                    SET confidence = EXCLUDED.confidence,
                        missing_signals = EXCLUDED.missing_signals,
                        features = EXCLUDED.features
                    "#,
                    &[
                        &new_id(),
                        &snapshot_id,
                        &event_id,
                        &sport_key,
                        &"market_context_v1",
                        &"danskespil_content_service",
                        &features
                            .get("confidence")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0),
                        &features
                            .get("missing_signals")
                            .cloned()
                            .unwrap_or_else(|| json!([])),
                        &features,
                    ],
                )
                .await?;
        }
    }
    Ok(())
}

async fn candidate_odds_movement(
    transaction: &Transaction<'_>,
    current_snapshot_id: &str,
    current_observed_at: DateTime<Utc>,
    candidate: &CandidateBet,
) -> anyhow::Result<Option<Value>> {
    let (Some(event_id), Some(market_id), Some(outcome_id), Some(current_decimal_odds)) = (
        candidate.event_id.as_deref(),
        candidate.market_id.as_deref(),
        candidate.outcome_id.as_deref(),
        candidate.decimal_odds,
    ) else {
        return Ok(None);
    };
    let row = transaction
        .query_opt(
            r#"
            SELECT
              os.id AS previous_snapshot_id,
              os.observed_at AS previous_observed_at,
              oo.decimal_odds::float8 AS previous_decimal_odds,
              oo.active AS previous_active,
              oo.displayed AS previous_displayed
            FROM outcome_observations oo
            JOIN market_observations mo ON mo.id = oo.market_observation_id
            JOIN odds_snapshots os ON os.id = oo.snapshot_id
            WHERE os.id <> $1
              AND os.observed_at < $2
              AND mo.event_id = $3
              AND mo.market_id = $4
              AND oo.outcome_id = $5
              AND oo.decimal_odds IS NOT NULL
            ORDER BY os.observed_at DESC
            LIMIT 1
            "#,
            &[
                &current_snapshot_id,
                &current_observed_at,
                &event_id,
                &market_id,
                &outcome_id,
            ],
        )
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let previous_decimal_odds: f64 = row.get("previous_decimal_odds");
    let delta = current_decimal_odds - previous_decimal_odds;
    let delta_pct = if previous_decimal_odds == 0.0 {
        None
    } else {
        Some(delta / previous_decimal_odds)
    };
    let direction = if delta > 0.0 {
        "up"
    } else if delta < 0.0 {
        "down"
    } else {
        "flat"
    };
    let previous_observed_at: DateTime<Utc> = row.get("previous_observed_at");
    Ok(Some(json!({
        "source": "outcome_observations_latest_previous_at_candidate_insert",
        "previous_snapshot_id": row.get::<_, String>("previous_snapshot_id"),
        "previous_observed_at": previous_observed_at,
        "current_snapshot_id": current_snapshot_id,
        "current_observed_at": current_observed_at,
        "previous_decimal_odds": previous_decimal_odds,
        "current_decimal_odds": current_decimal_odds,
        "decimal_odds_delta": delta,
        "decimal_odds_delta_pct": delta_pct,
        "direction": direction,
        "previous_active": row.get::<_, Option<bool>>("previous_active"),
        "previous_displayed": row.get::<_, Option<bool>>("previous_displayed"),
        "paper_only": true,
        "not_settlement_grade": true
    })))
}

fn attach_candidate_odds_movement(candidate: &mut CandidateBet, mut movement: Value) {
    let classification = classify_candidate_odds_movement(&movement);
    if let Some(object) = movement.as_object_mut() {
        object.insert("classification".to_string(), classification.clone());
    }
    let risk_flags = candidate_movement_risk_flags(&candidate.risk_flags, &classification);
    candidate.risk_flags = json!(risk_flags.clone());

    if let Some(features) = candidate.feature_snapshot.as_object_mut() {
        features.insert("odds_movement".to_string(), movement.clone());
        features.insert("risk_flags".to_string(), json!(risk_flags.clone()));
    } else {
        candidate.feature_snapshot = json!({
            "odds_movement": movement.clone(),
            "risk_flags": risk_flags
        });
    }

    if let Some(evidence) = candidate
        .rationale
        .get_mut("evidence")
        .and_then(Value::as_object_mut)
    {
        evidence.insert("odds_movement".to_string(), movement);
    } else if let Some(rationale) = candidate.rationale.as_object_mut() {
        rationale.insert("odds_movement".to_string(), movement);
    }

    if let Some(score_summary) = candidate
        .rationale
        .get_mut("score_summary")
        .and_then(Value::as_object_mut)
    {
        score_summary.insert("risk_flags".to_string(), candidate.risk_flags.clone());
        score_summary.insert("movement_classification".to_string(), classification);
    }
}

fn classify_candidate_odds_movement(movement: &Value) -> Value {
    let delta = movement
        .get("decimal_odds_delta")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let delta_pct = movement
        .get("decimal_odds_delta_pct")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let abs_delta = delta.abs();
    let abs_delta_pct = delta_pct.abs();
    let direction = movement
        .get("direction")
        .and_then(Value::as_str)
        .unwrap_or("flat");
    let movement_band = if abs_delta < 0.01 || abs_delta_pct < 0.005 {
        "stable"
    } else if abs_delta >= 0.5 || abs_delta_pct >= 0.10 {
        "large"
    } else {
        "normal"
    };
    let mut flags = Vec::new();
    match direction {
        "up" => flags.push("odds_moved_up"),
        "down" => flags.push("odds_moved_down"),
        _ => flags.push("odds_stable"),
    }
    if movement_band == "large" {
        flags.push("large_odds_movement");
    }
    if movement
        .get("previous_active")
        .and_then(Value::as_bool)
        .is_some_and(|active| !active)
    {
        flags.push("previous_outcome_inactive");
    }
    if movement
        .get("previous_displayed")
        .and_then(Value::as_bool)
        .is_some_and(|displayed| !displayed)
    {
        flags.push("previous_outcome_hidden");
    }
    flags.sort();
    flags.dedup();
    json!({
        "movement_band": movement_band,
        "direction": direction,
        "absolute_delta": abs_delta,
        "absolute_delta_pct": abs_delta_pct,
        "risk_flags": flags,
        "score_adjusted": false,
        "reason": "Movement is persisted as decision-time evidence. Numeric score changes require reviewed strategy experiments."
    })
}

fn candidate_movement_risk_flags(existing: &Value, classification: &Value) -> Vec<String> {
    let mut flags: Vec<String> = existing
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    flags.extend(
        classification
            .get("risk_flags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string),
    );
    flags.sort();
    flags.dedup();
    flags
}

fn event_feature_snapshot(sport_key: &str, event: &Value) -> Value {
    let teams = event
        .get("teams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let markets = event
        .get("markets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let scoreboard_facts = event
        .get("scoreboard_facts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let external_ids = event
        .get("external_ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let market_kinds = unique_strings(markets.iter().filter_map(|market| text(market, "kind")));
    let external_providers = unique_strings(
        external_ids
            .iter()
            .filter_map(|external_id| text(external_id, "provider")),
    );
    let motorsports_context = if sport_key == "motorsports" {
        Some(motorsports_series_context(event))
    } else {
        None
    };

    let mut missing = Vec::new();
    if text(event, "competition").is_none() {
        missing.push("competition");
    }
    if event.get("start_time").unwrap_or(&Value::Null).is_null() {
        missing.push("start_time");
    }
    if teams.is_empty() && !matches!(sport_key, "motorsports" | "golf" | "cycling") {
        missing.push("participants");
    }
    if external_ids.is_empty() {
        missing.push("external_ids");
    }
    if markets.is_empty() {
        missing.push("markets");
    }
    if scoreboard_facts.is_empty() && bool_value(event, "live_now") {
        missing.push("live_scoreboard");
    }
    if motorsports_context
        .as_ref()
        .and_then(|context| text(context, "series_family"))
        == Some("unknown")
    {
        missing.push("motorsports_series");
    }
    // Placeholders for the next ingestion layers. Keeping them explicit makes
    // candidate reasoning honest until those sources are wired in.
    missing.extend(["form", "weather", "news", "rankings", "injury_availability"]);
    missing.sort();
    missing.dedup();

    let outcome_count: usize = markets
        .iter()
        .map(|market| {
            market
                .get("outcomes")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default()
        })
        .sum();
    let confidence = (0.25_f64
        + if text(event, "competition").is_some() {
            0.12
        } else {
            0.0
        }
        + if !event.get("start_time").unwrap_or(&Value::Null).is_null() {
            0.12
        } else {
            0.0
        }
        + if !markets.is_empty() { 0.16 } else { 0.0 }
        + if outcome_count > 0 { 0.12 } else { 0.0 }
        + if !external_ids.is_empty() { 0.08 } else { 0.0 }
        + if !teams.is_empty() { 0.08 } else { 0.0 }
        + if !scoreboard_facts.is_empty() {
            0.04
        } else {
            0.0
        }
        + if motorsports_context
            .as_ref()
            .and_then(|context| text(context, "series_family"))
            .is_some_and(|series| series != "unknown")
        {
            0.04
        } else {
            0.0
        })
    .clamp(0.1, 0.82);

    json!({
        "feature_set": "market_context_v1",
        "source_key": "danskespil_content_service",
        "sport_key": sport_key,
        "event_id": event.get("id").cloned().unwrap_or(Value::Null),
        "event_name": event.get("name").cloned().unwrap_or(Value::Null),
        "competition": event.get("competition").cloned().unwrap_or(Value::Null),
        "class_name": event.get("class_name").cloned().unwrap_or(Value::Null),
        "start_time": event.get("start_time").cloned().unwrap_or(Value::Null),
        "live_now": bool_value(event, "live_now"),
        "started": bool_value(event, "started"),
        "resulted": bool_value(event, "resulted"),
        "settled": bool_value(event, "settled"),
        "participant_count": teams.len(),
        "market_count": markets.len(),
        "outcome_count": outcome_count,
        "market_kinds": market_kinds,
        "scoreboard_fact_count": scoreboard_facts.len(),
        "external_provider_count": external_providers.len(),
        "external_providers": external_providers,
        "sport_context": motorsports_context.unwrap_or(Value::Null),
        "missing_signals": missing,
        "confidence": confidence,
        "limits": {
            "paper_only": true,
            "not_settlement_grade": true,
            "uses_only_market_feed": true
        }
    })
}

fn motorsports_series_context(event: &Value) -> Value {
    let fields = [
        text(event, "competition"),
        text(event, "class_name"),
        text(event, "name"),
    ];
    let haystack = fields
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let series_family = if haystack.contains("indycar") || haystack.contains("indy car") {
        "indycar"
    } else if haystack.contains("nascar") {
        "nascar"
    } else if haystack.contains("le mans")
        || haystack.contains("wec")
        || haystack.contains("endurance")
        || haystack.contains("imsa")
    {
        "endurance"
    } else if haystack.contains("motogp")
        || haystack.contains("moto gp")
        || haystack.contains("superbike")
        || haystack.contains("motorbike")
        || haystack.contains("motorcycle")
    {
        "motorbike"
    } else if haystack.contains("formula 1")
        || haystack.contains("formel 1")
        || haystack == "f1"
        || haystack.contains(" f1 ")
        || haystack.starts_with("f1 ")
        || haystack.ends_with(" f1")
    {
        "formula_1"
    } else if haystack.contains("rally") || haystack.contains("wrc") {
        "rally"
    } else {
        "unknown"
    };
    let vehicle_type = match series_family {
        "motorbike" => "motorbike",
        "unknown" => "unknown",
        _ => "car",
    };
    json!({
        "category": "motorsports",
        "series_family": series_family,
        "vehicle_type": vehicle_type,
        "series_known": series_family != "unknown",
        "source_fields": ["competition", "class_name", "event_name"]
    })
}

fn unique_strings<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut values: Vec<String> = values.map(str::to_string).collect();
    values.sort();
    values.dedup();
    values
}

fn sport_events(sport: &Value) -> impl Iterator<Item = &Value> {
    sport
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(
            sport
                .get("outrights")
                .and_then(Value::as_array)
                .into_iter()
                .flatten(),
        )
}

fn parse_datetime(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|datetime| datetime.with_timezone(&Utc))
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn bool_value(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn optional_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn accumulator_allowed(candidate: &CandidateBet, leg_count: usize) -> bool {
    let minimum = json_usize(candidate.feature_snapshot.get("minimum_accumulator"));
    let maximum = json_usize(candidate.feature_snapshot.get("maximum_accumulator"));
    match (minimum, maximum) {
        (Some(minimum), Some(maximum)) => minimum <= leg_count && leg_count <= maximum,
        _ => false,
    }
}

fn json_usize(value: Option<&Value>) -> Option<usize> {
    value.and_then(|value| {
        value
            .as_u64()
            .map(|value| value as usize)
            .or_else(|| value.as_str().and_then(|text| text.parse::<usize>().ok()))
    })
}

fn json_i32(value: Option<&Value>) -> Option<i32> {
    value.and_then(|value| {
        value
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .or_else(|| value.as_u64().and_then(|value| i32::try_from(value).ok()))
            .or_else(|| {
                value
                    .as_str()
                    .and_then(|text| text.trim().parse::<i32>().ok())
            })
    })
}

fn account_history_raw_status(payload: &Value) -> Option<String> {
    [
        "settlement_result",
        "result",
        "result_status",
        "status",
        "bookmaker_status",
    ]
    .into_iter()
    .find_map(|key| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn account_history_event_names(payload: &Value, fallback_event_name: &str) -> Vec<String> {
    let mut names = dedup_strings(
        json_string_array(payload.get("event_names"))
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect(),
    );
    if names.is_empty() && !fallback_event_name.trim().is_empty() {
        names.push(fallback_event_name.trim().to_string());
    }
    names
}

fn account_history_settlement_result(payload: &Value) -> Option<&'static str> {
    let raw = account_history_raw_status(payload)?;
    let normalized = raw
        .trim()
        .to_ascii_lowercase()
        .replace(['-', '_', '.', '/'], " ");
    let words: HashSet<&str> = normalized.split_whitespace().collect();
    if words.contains("won")
        || words.contains("win")
        || words.contains("vundet")
        || words.contains("gevinst")
        || normalized.contains("settled won")
        || normalized.contains("paid out")
    {
        return Some("won");
    }
    if words.contains("lost")
        || words.contains("loss")
        || words.contains("tabt")
        || normalized.contains("settled lost")
    {
        return Some("lost");
    }
    if words.contains("void") || words.contains("voided") || words.contains("annulled") {
        return Some("void");
    }
    if words.contains("push")
        || words.contains("pushed")
        || normalized.contains("stake returned")
        || normalized.contains("stake return")
    {
        return Some("pushed");
    }
    if words.contains("refund")
        || words.contains("refunded")
        || words.contains("refunderet")
        || normalized.contains("money back")
    {
        return Some("refunded");
    }
    if words.contains("cancelled")
        || words.contains("canceled")
        || words.contains("cancel")
        || words.contains("aflyst")
    {
        return Some("cancelled");
    }
    if words.contains("abandoned") || words.contains("abandon") {
        return Some("abandoned");
    }
    if words.contains("postponed") || words.contains("postpone") || words.contains("udsat") {
        return Some("postponed");
    }
    if words.contains("unresolved") || words.contains("pending") || words.contains("unknown") {
        return Some("unresolved");
    }
    None
}

fn distinct_events(candidates: &[&CandidateBet]) -> bool {
    let mut seen = HashSet::new();
    for candidate in candidates {
        let Some(event_id) = candidate.event_id.as_deref() else {
            return false;
        };
        if !seen.insert(event_id) {
            return false;
        }
    }
    true
}

fn coupon_leg_signature(candidates: &[&CandidateBet]) -> String {
    let mut ids: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect();
    ids.sort_unstable();
    ids.join("+")
}

fn combinations<'a>(items: &[&'a CandidateBet], leg_count: usize) -> Vec<Vec<&'a CandidateBet>> {
    fn walk<'a>(
        items: &[&'a CandidateBet],
        leg_count: usize,
        start: usize,
        current: &mut Vec<&'a CandidateBet>,
        output: &mut Vec<Vec<&'a CandidateBet>>,
    ) {
        if current.len() == leg_count {
            output.push(current.clone());
            return;
        }
        let remaining_needed = leg_count - current.len();
        if items.len().saturating_sub(start) < remaining_needed {
            return;
        }
        for index in start..items.len() {
            current.push(items[index]);
            walk(items, leg_count, index + 1, current, output);
            current.pop();
        }
    }

    let mut output = Vec::new();
    let mut current = Vec::new();
    walk(items, leg_count, 0, &mut current, &mut output);
    output
}

#[derive(Debug, Clone)]
struct ExternalResultLink {
    source_key: String,
    url: String,
    sport_key: Option<String>,
    gender_scope: Option<String>,
    home_aliases: Vec<String>,
    away_aliases: Vec<String>,
    requires_browser_automation: bool,
    known_home_score: Option<i32>,
    known_away_score: Option<i32>,
    known_result_status: Option<String>,
    known_result_notes: Option<String>,
}

#[derive(Debug, Clone)]
struct ExternalMatchResult {
    source_key: String,
    url: String,
    title: String,
    home_name: String,
    away_name: String,
    home_score: i32,
    away_score: i32,
    confidence: f64,
}

struct EntityAliasInput<'a> {
    entity_kind: &'a str,
    sport_key: Option<&'a str>,
    gender_scope: Option<&'a str>,
    canonical_name: &'a str,
    alias_name: &'a str,
    source_key: Option<&'a str>,
    external_id: Option<&'a str>,
    confidence: f64,
    payload: Value,
}

async fn upsert_entity_alias(
    client: &Client,
    input: EntityAliasInput<'_>,
) -> anyhow::Result<Value> {
    validate_entity_kind(input.entity_kind)?;
    let canonical_key = normalize_match_name(input.canonical_name);
    let alias_key = normalize_match_name(input.alias_name);
    if canonical_key.is_empty() || alias_key.is_empty() {
        return Err(anyhow!("alias names must include alphanumeric characters"));
    }
    let row = client
        .query_one(
            r#"
            INSERT INTO entity_aliases (
              id, entity_kind, sport_key, gender_scope, canonical_name, canonical_key, alias_name,
              alias_key, source_key, external_id, confidence, payload
            )
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,($11::float8)::numeric,$12)
            ON CONFLICT (
              entity_kind,
              COALESCE(sport_key, ''),
              COALESCE(gender_scope, ''),
              canonical_key,
              alias_key,
              COALESCE(source_key, ''),
              COALESCE(external_id, '')
            )
            DO UPDATE
            SET canonical_name = EXCLUDED.canonical_name,
                alias_name = EXCLUDED.alias_name,
                confidence = GREATEST(entity_aliases.confidence, EXCLUDED.confidence),
                payload = entity_aliases.payload || EXCLUDED.payload,
                last_seen_at = now()
            RETURNING
              id,
              entity_kind,
              sport_key,
              gender_scope,
              canonical_name,
              canonical_key,
              alias_name,
              alias_key,
              source_key,
              external_id,
              confidence::float8 AS confidence,
              payload,
              first_seen_at,
              last_seen_at
            "#,
            &[
                &new_id(),
                &input.entity_kind,
                &input.sport_key,
                &input.gender_scope,
                &input.canonical_name,
                &canonical_key,
                &input.alias_name,
                &alias_key,
                &input.source_key,
                &input.external_id,
                &input.confidence,
                &input.payload,
            ],
        )
        .await?;
    let first_seen_at: DateTime<Utc> = row.get("first_seen_at");
    let last_seen_at: DateTime<Utc> = row.get("last_seen_at");
    Ok(json!({
        "id": row.get::<_, String>("id"),
        "entity_kind": row.get::<_, String>("entity_kind"),
        "sport_key": row.get::<_, Option<String>>("sport_key"),
        "gender_scope": row.get::<_, Option<String>>("gender_scope"),
        "canonical_name": row.get::<_, String>("canonical_name"),
        "canonical_key": row.get::<_, String>("canonical_key"),
        "alias_name": row.get::<_, String>("alias_name"),
        "alias_key": row.get::<_, String>("alias_key"),
        "source_key": row.get::<_, Option<String>>("source_key"),
        "external_id": row.get::<_, Option<String>>("external_id"),
        "confidence": row.get::<_, f64>("confidence"),
        "payload": row.get::<_, Value>("payload"),
        "first_seen_at": first_seen_at,
        "last_seen_at": last_seen_at,
        "paper_only": true
    }))
}

async fn record_entity_aliases(
    client: &Client,
    entity_kind: &str,
    sport_key: Option<&str>,
    gender_scope: Option<&str>,
    canonical_name: &str,
    aliases: Vec<String>,
    source_key: Option<&str>,
    external_id: Option<&str>,
    confidence: f64,
    notes: Value,
) -> anyhow::Result<Vec<Value>> {
    let mut rows = Vec::new();
    for alias_name in dedup_strings(
        aliases
            .into_iter()
            .chain(std::iter::once(canonical_name.to_string()))
            .collect(),
    ) {
        rows.push(
            upsert_entity_alias(
                client,
                EntityAliasInput {
                    entity_kind,
                    sport_key,
                    gender_scope,
                    canonical_name,
                    alias_name: &alias_name,
                    source_key,
                    external_id,
                    confidence,
                    payload: json!({
                        "notes": notes,
                        "paper_only": true
                    }),
                },
            )
            .await?,
        );
    }
    Ok(rows)
}

async fn expand_aliases_from_registry(
    client: &Client,
    entity_kind: &str,
    sport_key: Option<&str>,
    gender_scope: Option<&str>,
    aliases: Vec<String>,
) -> anyhow::Result<Vec<String>> {
    let mut expanded = dedup_strings(aliases);
    if expanded.is_empty() {
        return Ok(expanded);
    }
    let keys = expanded
        .iter()
        .map(|value| normalize_match_name(value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if keys.is_empty() {
        return Ok(expanded);
    }
    let rows = client
        .query(
            r#"
            SELECT canonical_name, alias_name
            FROM entity_aliases
            WHERE entity_kind = $1
              AND (canonical_key = ANY($2) OR alias_key = ANY($2))
              AND ($3::text IS NULL OR sport_key IS NULL OR sport_key = $3)
              AND ($4::text IS NULL OR gender_scope IS NULL OR gender_scope = $4)
            "#,
            &[&entity_kind, &keys, &sport_key, &gender_scope],
        )
        .await?;
    for row in rows {
        expanded.push(row.get::<_, String>("canonical_name"));
        expanded.push(row.get::<_, String>("alias_name"));
    }
    Ok(dedup_strings(expanded))
}

fn external_result_link_for_event(
    source_policy: &Value,
    event_name: &str,
) -> Option<ExternalResultLink> {
    external_result_links_for_event(source_policy, event_name)
        .into_iter()
        .next()
}

fn external_result_links_for_event(
    source_policy: &Value,
    event_name: &str,
) -> Vec<ExternalResultLink> {
    let event_keys = normalized_event_name_variants(event_name);
    let mut links: Vec<ExternalResultLink> = source_policy
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|source| {
            matches!(
                source.get("source_key").and_then(Value::as_str),
                Some(
                    "flashscore_results"
                        | "sofascore_results"
                        | "xscores_results"
                        | "livescore_results"
                )
            )
        })
        .flat_map(|source| {
            let Some(source_key) = source.get("source_key").and_then(Value::as_str) else {
                return Vec::new();
            };
            let requires_browser_automation = source
                .get("payload")
                .and_then(|payload| payload.get("requires_browser_automation"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            source
                .get("payload")
                .and_then(|payload| payload.get("known_matches"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| {
                    let known_event = item.get("event_name").and_then(Value::as_str)?;
                    if !event_keys.contains(&normalize_match_name(known_event)) {
                        return None;
                    }
                    Some(ExternalResultLink {
                        source_key: source_key.to_string(),
                        url: item.get("url").and_then(Value::as_str)?.to_string(),
                        sport_key: item
                            .get("sport_key")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string),
                        gender_scope: payload_gender_scope(item).ok().flatten(),
                        home_aliases: json_string_array(item.get("home_aliases")),
                        away_aliases: json_string_array(item.get("away_aliases")),
                        requires_browser_automation: requires_browser_automation
                            || item
                                .get("requires_browser_automation")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        known_home_score: json_i32(item.get("home_score")),
                        known_away_score: json_i32(item.get("away_score")),
                        known_result_status: item
                            .get("result_status")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        known_result_notes: item
                            .get("notes")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();
    links.sort_by_key(|link| {
        (
            link.requires_browser_automation,
            match link.source_key.as_str() {
                "flashscore_results" => 0,
                "sofascore_results" => 1,
                "xscores_results" => 2,
                "livescore_results" => 3,
                _ => 9,
            },
        )
    });
    links
}

fn external_result_link_json(link: &ExternalResultLink) -> Value {
    json!({
        "source_key": link.source_key.clone(),
        "source_url": link.url.clone(),
        "sport_key": link.sport_key.clone(),
        "gender_scope": link.gender_scope.clone(),
        "home_aliases": link.home_aliases.clone(),
        "away_aliases": link.away_aliases.clone(),
        "requires_browser_automation": link.requires_browser_automation,
        "known_result": match (link.known_home_score, link.known_away_score) {
            (Some(home), Some(away)) => json!({
                "home_score": home,
                "away_score": away,
                "status": link.known_result_status,
                "notes": link.known_result_notes
            }),
            _ => Value::Null
        }
    })
}

fn merge_operator_result_links(mut payload: Value, links: &[Value]) -> Value {
    if links.is_empty() {
        return payload;
    }
    let Some(object) = payload.as_object_mut() else {
        return json!({"known_matches": links});
    };
    let known_matches = object.entry("known_matches").or_insert_with(|| json!([]));
    if let Some(items) = known_matches.as_array_mut() {
        let mut existing = items
            .iter()
            .filter_map(|item| {
                Some((
                    normalize_match_name(item.get("event_name")?.as_str()?),
                    item.get("url")?.as_str()?.to_string(),
                ))
            })
            .collect::<HashSet<_>>();
        for link in links {
            let key = (
                normalize_match_name(link.get("event_name").and_then(Value::as_str).unwrap_or("")),
                link.get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            );
            if existing.insert(key) {
                items.push(link.clone());
            }
        }
    }
    payload
}

fn is_external_result_source(source_key: &str) -> bool {
    matches!(
        source_key,
        "flashscore_results" | "sofascore_results" | "xscores_results" | "livescore_results"
    )
}

fn use_external_result_alias_registry(sport_key: Option<&str>, event_name: &str) -> bool {
    !is_tennis_doubles_event_name(sport_key, event_name)
}

fn is_tennis_doubles_event_name(sport_key: Option<&str>, event_name: &str) -> bool {
    sport_key == Some("tennis")
        && event_name
            .split_once(" - ")
            .map(|(home, away)| home.contains('/') && away.contains('/'))
            .unwrap_or(false)
}

fn validate_entity_kind(entity_kind: &str) -> anyhow::Result<()> {
    match entity_kind {
        "participant" | "team" | "player" | "league" | "competition" | "driver" | "golfer"
        | "rider" => Ok(()),
        _ => Err(anyhow!("unsupported alias entity_kind: {entity_kind}")),
    }
}

fn payload_gender_scope(payload: &Value) -> anyhow::Result<Option<String>> {
    let Some(raw) = payload
        .get("gender_scope")
        .or_else(|| payload.get("gender"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    normalize_gender_scope(raw)
        .map(|value| Some(value.to_string()))
        .ok_or_else(|| anyhow!("unsupported gender_scope: {raw}"))
}

fn normalize_gender_scope(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "men" | "mens" | "men's" | "male" | "m" | "herre" | "herrer" | "herresingle" => Some("men"),
        "women" | "womens" | "women's" | "female" | "f" | "dame" | "damer" | "damesingle"
        | "kvinde" | "kvinder" => Some("women"),
        "mixed" | "mix" | "mixed doubles" => Some("mixed"),
        "unknown" | "any" | "all" => None,
        _ => None,
    }
}

fn validate_external_result_url(source_key: &str, source_url: &str) -> anyhow::Result<()> {
    let parsed = Url::parse(source_url).with_context(|| format!("invalid URL: {source_url}"))?;
    let scheme = parsed.scheme();
    if scheme != "https" && scheme != "http" {
        return Err(anyhow!("external result URL must be http or https"));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("external result URL must include a host"))?
        .to_ascii_lowercase();
    let allowed = match source_key {
        "flashscore_results" => {
            host == "flashscore.com"
                || host.ends_with(".flashscore.com")
                || host == "flashscore.dk"
                || host.ends_with(".flashscore.dk")
        }
        "sofascore_results" => host == "sofascore.com" || host.ends_with(".sofascore.com"),
        "xscores_results" => host == "xscores.com" || host.ends_with(".xscores.com"),
        "livescore_results" => host == "livescore.com" || host.ends_with(".livescore.com"),
        _ => false,
    };
    if !allowed {
        return Err(anyhow!(
            "URL host {host} is not allowed for settlement source {source_key}"
        ));
    }
    Ok(())
}

fn event_aliases_from_name(event_name: &str) -> (Vec<String>, Vec<String>) {
    event_name
        .split_once(" - ")
        .map(|(home, away)| (vec![home.trim().to_string()], vec![away.trim().to_string()]))
        .unwrap_or_else(|| (Vec::new(), Vec::new()))
}

fn reversed_event_name(event_name: &str) -> Option<String> {
    event_name
        .split_once(" - ")
        .map(|(home, away)| format!("{} - {}", away.trim(), home.trim()))
}

fn event_name_variants(event_name: &str) -> Vec<String> {
    dedup_strings(
        std::iter::once(event_name.to_string())
            .chain(reversed_event_name(event_name))
            .collect(),
    )
}

fn normalized_event_name_variants(event_name: &str) -> HashSet<String> {
    event_name_variants(event_name)
        .into_iter()
        .map(|value| normalize_match_name(&value))
        .filter(|value| !value.is_empty())
        .collect()
}

fn dedup_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.to_ascii_lowercase()))
        .collect()
}

fn external_result_http_client() -> anyhow::Result<HttpClient> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        ),
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("da-DK,da;q=0.9,en-US;q=0.8,en;q=0.7"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        HeaderName::from_static("upgrade-insecure-requests"),
        HeaderValue::from_static("1"),
    );
    Ok(HttpClient::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
        .default_headers(headers)
        .timeout(StdDuration::from_secs(12))
        .build()?)
}

fn external_result_link_from_task_source(value: &Value) -> Option<ExternalResultLink> {
    let source_key = value.get("source_key").and_then(Value::as_str)?.to_string();
    let url = value
        .get("source_url")
        .or_else(|| value.get("url"))
        .and_then(Value::as_str)?
        .to_string();
    let known_result = value.get("known_result").unwrap_or(&Value::Null);
    Some(ExternalResultLink {
        source_key,
        url,
        sport_key: value
            .get("sport_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        gender_scope: payload_gender_scope(value).ok().flatten(),
        home_aliases: json_string_array(value.get("home_aliases")),
        away_aliases: json_string_array(value.get("away_aliases")),
        requires_browser_automation: value
            .get("requires_browser_automation")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        known_home_score: json_i32(known_result.get("home_score")),
        known_away_score: json_i32(known_result.get("away_score")),
        known_result_status: known_result
            .get("status")
            .and_then(Value::as_str)
            .map(str::to_string),
        known_result_notes: known_result
            .get("notes")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

async fn fetch_external_match_result(
    http: &HttpClient,
    link: &ExternalResultLink,
) -> anyhow::Result<ExternalMatchResult> {
    if let Some(result) = known_external_match_result(link) {
        return Ok(result);
    }
    if link.source_key == "flashscore_results" {
        if let Some(result) = fetch_flashscore_match_result(http, link).await? {
            return Ok(result);
        }
    }

    let html = http
        .get(&link.url)
        .send()
        .await
        .with_context(|| format!("fetch external result page {}", link.url))?
        .error_for_status()
        .with_context(|| format!("external result page returned error {}", link.url))?
        .text()
        .await?;
    if link.source_key == "xscores_results" {
        if let Some(result) = parse_xscores_match_result(&html, link) {
            return Ok(result);
        }
    }
    let title = extract_meta_content(&html, "og:title")
        .or_else(|| extract_title(&html))
        .ok_or_else(|| anyhow!("external result page did not expose a title"))?;
    let (home_name, away_name, home_score, away_score) =
        parse_score_title(&title).ok_or_else(|| {
            anyhow!("external result title did not include a parseable final score: {title}")
        })?;
    Ok(ExternalMatchResult {
        source_key: link.source_key.clone(),
        url: link.url.clone(),
        title,
        home_name,
        away_name,
        home_score,
        away_score,
        confidence: if link.source_key == "flashscore_results" {
            0.86
        } else {
            0.82
        },
    })
}

fn known_external_match_result(link: &ExternalResultLink) -> Option<ExternalMatchResult> {
    let (Some(home_score), Some(away_score)) = (link.known_home_score, link.known_away_score)
    else {
        return None;
    };
    let (home_name, away_name) = external_link_participants(link);
    Some(ExternalMatchResult {
        source_key: link.source_key.clone(),
        url: link.url.clone(),
        title: format!("{home_name} - {away_name} {home_score}:{away_score}"),
        home_name,
        away_name,
        home_score,
        away_score,
        confidence: match link.source_key.as_str() {
            "xscores_results" => 0.78,
            "flashscore_results" => 0.86,
            _ => 0.76,
        },
    })
}

async fn fetch_flashscore_match_result(
    http: &HttpClient,
    link: &ExternalResultLink,
) -> anyhow::Result<Option<ExternalMatchResult>> {
    let Some(event_id) = flashscore_match_id(&link.url) else {
        return Ok(None);
    };
    let feed_url = format!("{FLASHSCORE_BASE_URL}/x/feed/dc_1_{event_id}");
    let feed = http
        .get(feed_url)
        .header("x-fsign", FLASHSCORE_DEFAULT_XFSIGN)
        .send()
        .await
        .with_context(|| format!("fetch FlashScore match feed {}", link.url))?
        .error_for_status()
        .with_context(|| format!("FlashScore match feed returned error {}", link.url))?
        .text()
        .await?;
    let fields = parse_flashscore_kv_feed(&feed);
    let home_score = flashscore_score_field(&fields, &["DE", "DG", "DA"])
        .ok_or_else(|| anyhow!("FlashScore match feed did not expose a home score"))?;
    let away_score = flashscore_score_field(&fields, &["DF", "DH", "DS"])
        .ok_or_else(|| anyhow!("FlashScore match feed did not expose an away score"))?;
    let (home_name, away_name) = fetch_flashscore_page_participants(http, &link.url)
        .await?
        .or_else(|| source_url_participant_names(link))
        .unwrap_or_else(|| external_link_participants(link));
    let title = format!("{home_name} - {away_name} {home_score}:{away_score}");
    Ok(Some(ExternalMatchResult {
        source_key: link.source_key.clone(),
        url: link.url.clone(),
        title,
        home_name,
        away_name,
        home_score,
        away_score,
        confidence: 0.86,
    }))
}

async fn fetch_flashscore_page_participants(
    http: &HttpClient,
    source_url: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let html = http
        .get(source_url)
        .send()
        .await
        .with_context(|| format!("fetch FlashScore match page {source_url}"))?
        .error_for_status()
        .with_context(|| format!("FlashScore match page returned error {source_url}"))?
        .text()
        .await?;
    Ok(extract_meta_content(&html, "og:title")
        .or_else(|| extract_title(&html))
        .and_then(|title| parse_flashscore_title_participants(&title)))
}

fn parse_xscores_match_result(
    html: &str,
    link: &ExternalResultLink,
) -> Option<ExternalMatchResult> {
    let text = html_text(html);
    if !text
        .to_ascii_lowercase()
        .split_whitespace()
        .any(|part| part == "finished")
    {
        return None;
    }
    let (home_name, away_name) = external_link_participants(link);
    let (home_score, away_score) =
        parse_score_near_participant_names(&text, &home_name, &away_name)
            .or_else(|| parse_first_dash_score(&text))?;
    Some(ExternalMatchResult {
        source_key: link.source_key.clone(),
        url: link.url.clone(),
        title: format!("{home_name} - {away_name} {home_score}:{away_score}"),
        home_name,
        away_name,
        home_score,
        away_score,
        confidence: 0.78,
    })
}

fn parse_score_near_participant_names(
    text: &str,
    home_name: &str,
    away_name: &str,
) -> Option<(i32, i32)> {
    let normalized = collapse_whitespace(text);
    let lower = normalized.to_ascii_lowercase();
    let home_idx = lower.find(&home_name.to_ascii_lowercase())?;
    let after_home = &normalized[home_idx + home_name.len()..];
    let window = after_home
        .get(..after_home.len().min(240))
        .unwrap_or(after_home);
    if !window
        .to_ascii_lowercase()
        .contains(&away_name.to_ascii_lowercase())
    {
        return None;
    }
    parse_first_dash_score(window)
}

fn parse_first_dash_score(text: &str) -> Option<(i32, i32)> {
    let tokens = text.split_whitespace().collect::<Vec<_>>();
    tokens.windows(3).find_map(|window| {
        if window.get(1).copied() != Some("-") {
            return None;
        }
        let home = window.first()?.parse::<i32>().ok()?;
        let away = window.get(2)?.parse::<i32>().ok()?;
        Some((home, away))
    })
}

fn html_text(html: &str) -> String {
    let mut output = String::new();
    let mut in_tag = false;
    for ch in html_unescape(html).chars() {
        match ch {
            '<' => {
                in_tag = true;
                output.push(' ');
            }
            '>' => {
                in_tag = false;
                output.push(' ');
            }
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    collapse_whitespace(&output)
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn flashscore_match_id(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    parsed.query_pairs().find_map(|(key, value)| {
        if key == "mid" && !value.trim().is_empty() {
            Some(value.to_string())
        } else {
            None
        }
    })
}

fn parse_flashscore_kv_feed(feed: &str) -> HashMap<String, String> {
    feed.split(|ch| ch == '¬' || ch == '~')
        .filter_map(|cell| {
            let (key, value) = cell.split_once('÷')?;
            if key.is_empty() {
                None
            } else {
                Some((key.to_string(), html_unescape(value)))
            }
        })
        .collect()
}

fn flashscore_score_field(fields: &HashMap<String, String>, keys: &[&str]) -> Option<i32> {
    keys.iter().find_map(|key| {
        fields
            .get(*key)
            .and_then(|value| value.trim().parse::<i32>().ok())
    })
}

fn external_link_participants(link: &ExternalResultLink) -> (String, String) {
    let home = link
        .home_aliases
        .first()
        .cloned()
        .or_else(|| source_url_participant_names(link).map(|(home, _)| home))
        .unwrap_or_else(|| "Home".to_string());
    let away = link
        .away_aliases
        .first()
        .cloned()
        .or_else(|| source_url_participant_names(link).map(|(_, away)| away))
        .unwrap_or_else(|| "Away".to_string());
    (home, away)
}

fn source_url_participant_names(link: &ExternalResultLink) -> Option<(String, String)> {
    match link.source_key.as_str() {
        "flashscore_results" => flashscore_url_participant_names(&link.url),
        "xscores_results" => xscores_url_participant_names(&link.url),
        _ => None,
    }
}

fn flashscore_url_participant_names(url: &str) -> Option<(String, String)> {
    let parsed = Url::parse(url).ok()?;
    let segments = parsed.path_segments()?.collect::<Vec<_>>();
    if segments.len() < 4 || !matches!(segments.first().copied(), Some("match" | "kamp")) {
        return None;
    }
    Some((
        flashscore_slug_to_name(segments[2]),
        flashscore_slug_to_name(segments[3]),
    ))
}

fn xscores_url_participant_names(url: &str) -> Option<(String, String)> {
    let parsed = Url::parse(url).ok()?;
    let segments = parsed.path_segments()?.collect::<Vec<_>>();
    let match_index = segments.iter().position(|segment| *segment == "match")?;
    let slug = segments.get(match_index + 1)?;
    let (home, away) = slug.split_once("-vs-")?;
    Some((plain_slug_to_name(home), plain_slug_to_name(away)))
}

fn flashscore_slug_to_name(slug: &str) -> String {
    let mut parts = slug.split('-').collect::<Vec<_>>();
    if parts.len() > 1 {
        parts.pop();
    }
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn plain_slug_to_name(slug: &str) -> String {
    slug.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_auto_settle_winner_market(market_kind: Option<&str>, market_name: Option<&str>) -> bool {
    let kind = market_kind.unwrap_or_default().to_ascii_lowercase();
    let name = market_name.unwrap_or_default().to_ascii_lowercase();
    kind == "winner"
        || name.contains("kampvinder")
        || name.contains("match winner")
        || name.contains("winner")
}

fn is_auto_settle_over_under_market(market_kind: Option<&str>, market_name: Option<&str>) -> bool {
    let kind = market_kind.unwrap_or_default().to_ascii_lowercase();
    let name = market_name.unwrap_or_default().to_ascii_lowercase();
    kind == "over_under"
        || kind == "total"
        || name.contains("o/u")
        || name.contains("over/under")
        || name.contains("over under")
}

fn is_auto_settle_external_market(market_kind: Option<&str>, market_name: Option<&str>) -> bool {
    is_auto_settle_winner_market(market_kind, market_name)
        || is_auto_settle_over_under_market(market_kind, market_name)
}

fn grade_external_outcome(
    outcome_name: &str,
    market_kind: Option<&str>,
    market_name: Option<&str>,
    link: &ExternalResultLink,
    evidence: &ExternalMatchResult,
) -> Option<&'static str> {
    if is_auto_settle_over_under_market(market_kind, market_name) {
        return grade_over_under_outcome(outcome_name, market_name, evidence);
    }
    grade_winner_outcome(outcome_name, link, evidence)
}

fn grade_winner_outcome(
    outcome_name: &str,
    link: &ExternalResultLink,
    evidence: &ExternalMatchResult,
) -> Option<&'static str> {
    let evidence_home_is_link_home = alias_matches(&evidence.home_name, &link.home_aliases);
    let evidence_away_is_link_away = alias_matches(&evidence.away_name, &link.away_aliases);
    let evidence_home_is_link_away = alias_matches(&evidence.home_name, &link.away_aliases);
    let evidence_away_is_link_home = alias_matches(&evidence.away_name, &link.home_aliases);
    let reversed_source_order = evidence_home_is_link_away && evidence_away_is_link_home;
    let normal_source_order = evidence_home_is_link_home && evidence_away_is_link_away;

    let (home_score, away_score, home_aliases, away_aliases) = if reversed_source_order {
        let mut home_aliases = link.home_aliases.clone();
        home_aliases.push(evidence.away_name.clone());
        let mut away_aliases = link.away_aliases.clone();
        away_aliases.push(evidence.home_name.clone());
        (
            evidence.away_score,
            evidence.home_score,
            home_aliases,
            away_aliases,
        )
    } else if normal_source_order {
        let mut home_aliases = link.home_aliases.clone();
        home_aliases.push(evidence.home_name.clone());
        let mut away_aliases = link.away_aliases.clone();
        away_aliases.push(evidence.away_name.clone());
        (
            evidence.home_score,
            evidence.away_score,
            home_aliases,
            away_aliases,
        )
    } else {
        let mut home_aliases = link.home_aliases.clone();
        home_aliases.push(evidence.home_name.clone());
        let mut away_aliases = link.away_aliases.clone();
        away_aliases.push(evidence.away_name.clone());
        (
            evidence.home_score,
            evidence.away_score,
            home_aliases,
            away_aliases,
        )
    };

    if home_score == away_score {
        return Some(if is_draw_outcome(outcome_name) {
            "won"
        } else {
            "lost"
        });
    }
    let home_won = home_score > away_score;
    if home_won && alias_matches(outcome_name, &home_aliases) {
        return Some("won");
    }
    if !home_won && alias_matches(outcome_name, &away_aliases) {
        return Some("won");
    }
    if alias_matches(outcome_name, &home_aliases)
        || alias_matches(outcome_name, &away_aliases)
        || is_draw_outcome(outcome_name)
    {
        return Some("lost");
    }
    None
}

fn grade_over_under_outcome(
    outcome_name: &str,
    market_name: Option<&str>,
    evidence: &ExternalMatchResult,
) -> Option<&'static str> {
    let outcome = normalize_match_name(outcome_name);
    let wants_over = outcome.contains("over");
    let wants_under = outcome.contains("under");
    if !wants_over && !wants_under {
        return None;
    }
    let line = parse_over_under_line(market_name.unwrap_or_default())
        .or_else(|| parse_over_under_line(outcome_name))?;
    let total = f64::from(evidence.home_score + evidence.away_score);
    if (total - line).abs() < f64::EPSILON {
        return Some("pushed");
    }
    if wants_over {
        Some(if total > line { "won" } else { "lost" })
    } else {
        Some(if total < line { "won" } else { "lost" })
    }
}

fn parse_over_under_line(value: &str) -> Option<f64> {
    value
        .replace(',', ".")
        .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .find_map(|part| {
            let part = part.trim_matches('.');
            if part.is_empty() {
                None
            } else {
                part.parse::<f64>().ok()
            }
        })
}

fn parse_score_title(title: &str) -> Option<(String, String, i32, i32)> {
    let clean = html_unescape(title);
    let (teams, score) = clean.rsplit_once(' ')?;
    let (home_score, away_score) = score.split_once(':')?;
    let (home_name, away_name) = teams.split_once(" - ")?;
    Some((
        home_name.trim().to_string(),
        away_name.trim().to_string(),
        home_score.trim().parse().ok()?,
        away_score.trim().parse().ok()?,
    ))
}

fn parse_flashscore_title_participants(title: &str) -> Option<(String, String)> {
    let clean = html_unescape(title);
    let head = clean
        .split('|')
        .next()
        .unwrap_or(clean.as_str())
        .trim()
        .to_string();
    let without_date = head
        .split_whitespace()
        .take_while(|part| !looks_like_date(part))
        .collect::<Vec<_>>()
        .join(" ");
    [" vs ", " - ", " v "].iter().find_map(|separator| {
        without_date.split_once(separator).and_then(|(home, away)| {
            let home = home.trim();
            let away = away.trim();
            if home.is_empty() || away.is_empty() {
                None
            } else {
                Some((home.to_string(), away.to_string()))
            }
        })
    })
}

fn looks_like_date(value: &str) -> bool {
    let trimmed = value.trim_matches(|character: char| !character.is_ascii_digit());
    let parts = trimmed.split('/').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        })
}

fn extract_meta_content(html: &str, property: &str) -> Option<String> {
    let property_index = html.find(property)?;
    let start = property_index.saturating_sub(180);
    let end = (property_index + 420).min(html.len());
    let window = &html[start..end];
    let content_index = window.find("content=\"")? + "content=\"".len();
    let content = &window[content_index..];
    let end_index = content.find('"')?;
    Some(html_unescape(&content[..end_index]))
}

fn extract_title(html: &str) -> Option<String> {
    let start = html.find("<title>")? + "<title>".len();
    let end = html[start..].find("</title>")?;
    Some(html_unescape(&html[start..start + end]))
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn json_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn alias_matches(value: &str, aliases: &[String]) -> bool {
    let normalized = normalize_match_name(value);
    let normalized_tokens = alias_match_tokens(value);
    aliases.iter().any(|raw_alias| {
        let alias = normalize_match_name(raw_alias);
        normalized == alias
            || (normalized.len() >= 5 && alias.contains(&normalized))
            || (alias.len() >= 5 && normalized.contains(&alias))
            || (!normalized_tokens.is_empty() && alias_match_tokens(raw_alias) == normalized_tokens)
    })
}

fn alias_match_tokens(value: &str) -> Vec<String> {
    let stop = ["fc", "sc", "ac", "bk", "if", "kk"];
    let mut tokens = normalize_token_text(value)
        .split_whitespace()
        .filter(|token| token.len() > 1 && !stop.contains(token))
        .map(str::to_string)
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens
}

fn is_draw_outcome(value: &str) -> bool {
    let normalized = normalize_match_name(value);
    matches!(normalized.as_str(), "uafgjort" | "draw" | "x" | "tie")
}

fn normalize_match_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalize_token_text(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars().flat_map(|ch| match ch {
        'Æ' | 'æ' => "ae".chars().collect::<Vec<_>>(),
        'Ø' | 'ø' => "o".chars().collect::<Vec<_>>(),
        'Å' | 'å' | 'Ä' | 'ä' | 'Á' | 'á' | 'À' | 'à' | 'Â' | 'â' => vec!['a'],
        'Ö' | 'ö' | 'Ó' | 'ó' | 'Ò' | 'ò' | 'Ô' | 'ô' => vec!['o'],
        'Ü' | 'ü' | 'Ú' | 'ú' | 'Ù' | 'ù' | 'Û' | 'û' => vec!['u'],
        'É' | 'é' | 'È' | 'è' | 'Ê' | 'ê' => vec!['e'],
        'Í' | 'í' | 'Ì' | 'ì' | 'Î' | 'î' => vec!['i'],
        'Ç' | 'ç' => vec!['c'],
        'Ñ' | 'ñ' => vec!['n'],
        other => vec![other],
    }) {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
        } else {
            output.push(' ');
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn settlement_review_recommendation(
    event_status: Option<&str>,
    event_resulted: Option<bool>,
    event_settled: Option<bool>,
    expected_result_check_after: Option<DateTime<Utc>>,
) -> &'static str {
    let normalized_status = event_status.unwrap_or_default().to_ascii_lowercase();
    if normalized_status.contains("cancel")
        || normalized_status.contains("postpon")
        || normalized_status.contains("abandon")
        || normalized_status.contains("suspend")
        || normalized_status.contains("void")
    {
        return "manual_void_or_refund_review";
    }
    if event_settled == Some(true) || event_resulted == Some(true) {
        return "manual_grade_ready";
    }
    if expected_result_check_after.is_some_and(|value| value <= Utc::now() - Duration::hours(2)) {
        return "external_result_required";
    }
    if expected_result_check_after.is_some_and(|value| value <= Utc::now()) {
        return "expected_finish_passed_recheck";
    }
    "await_more_evidence"
}

fn coupon_settlement_review_recommendation(
    legs: &Value,
    expected_result_check_after: Option<DateTime<Utc>>,
) -> &'static str {
    let leg_items = legs.as_array().map(Vec::as_slice).unwrap_or_default();
    if leg_items.iter().any(|leg| {
        let status = leg
            .get("event_status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        status.contains("cancel")
            || status.contains("postpon")
            || status.contains("abandon")
            || status.contains("suspend")
            || status.contains("void")
    }) {
        return "manual_void_or_refund_review";
    }
    if !leg_items.is_empty()
        && leg_items.iter().all(|leg| {
            leg.get("event_settled").and_then(Value::as_bool) == Some(true)
                || leg.get("event_resulted").and_then(Value::as_bool) == Some(true)
        })
    {
        return "manual_grade_ready";
    }
    if expected_result_check_after.is_some_and(|value| value <= Utc::now() - Duration::hours(2)) {
        return "external_result_required";
    }
    if expected_result_check_after.is_some_and(|value| value <= Utc::now()) {
        return "expected_finish_passed_recheck";
    }
    "await_more_evidence"
}

fn settlement_overdue_minutes(expected_result_check_after: Option<DateTime<Utc>>) -> Option<i64> {
    expected_result_check_after.map(|value| (Utc::now() - value).num_minutes().max(0))
}

fn settlement_recommended_source_key(recommendation: &str) -> &'static str {
    match recommendation {
        "external_result_required" => "official_competition_results",
        "manual_void_or_refund_review" => "danskespil_account_history",
        "manual_grade_ready" => "danskespil_account_history",
        _ => "danskespil_content_service",
    }
}

fn settlement_recommended_source_keys(recommendation: &str) -> Value {
    match recommendation {
        "external_result_required" => json!([
            "official_competition_results",
            "flashscore_results",
            "sofascore_results",
            "xscores_results",
            "livescore_results",
            "documented_third_party_results"
        ]),
        "manual_void_or_refund_review" | "manual_grade_ready" => {
            json!(["danskespil_account_history"])
        }
        _ => json!(["danskespil_content_service"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flashscore_og_title_score() {
        let parsed = parse_score_title("Notts Co - Salford 3:0").expect("score title");
        assert_eq!(
            parsed,
            ("Notts Co".to_string(), "Salford".to_string(), 3, 0)
        );
    }

    #[test]
    fn account_history_result_normalizes_bookmaker_statuses() {
        assert_eq!(
            account_history_settlement_result(&json!({"settlement_result": "settled_won"})),
            Some("won")
        );
        assert_eq!(
            account_history_settlement_result(&json!({"result_status": "settled-lost"})),
            Some("lost")
        );
        assert_eq!(
            account_history_settlement_result(&json!({"status": "stake returned"})),
            Some("pushed")
        );
        assert_eq!(
            account_history_settlement_result(&json!({"bookmaker_status": "refunderet"})),
            Some("refunded")
        );
        assert_eq!(
            account_history_settlement_result(&json!({"status": "manual review required"})),
            None
        );
    }

    #[test]
    fn account_history_event_names_preserve_coupon_legs() {
        let names = account_history_event_names(
            &json!({
                "event_names": [
                    "Team Fog Næstved - Bakken Bears",
                    "Team Fog Næstved - Bakken Bears",
                    "Fajing Sun - Jay Dylan Hara Friend"
                ]
            }),
            "Coupon: Team Fog Næstved - Bakken Bears / Fajing Sun - Jay Dylan Hara Friend",
        );

        assert_eq!(
            names,
            vec![
                "Team Fog Næstved - Bakken Bears".to_string(),
                "Fajing Sun - Jay Dylan Hara Friend".to_string()
            ]
        );
    }

    #[test]
    fn account_history_event_names_fall_back_to_event_name() {
        let names = account_history_event_names(&json!({}), "Notts County - Salford City FC");

        assert_eq!(names, vec!["Notts County - Salford City FC".to_string()]);
    }

    #[test]
    fn parses_flashscore_match_id_and_feed_score() {
        let link = ExternalResultLink {
            source_key: "flashscore_results".to_string(),
            url: "https://www.flashscore.com/match/football/notts-county-EwJVdqzn/salford-W4AadhN3/?mid=E3uvsQPP".to_string(),
            sport_key: Some("football".to_string()),
            gender_scope: None,
            home_aliases: vec!["Notts Co".to_string(), "Notts County".to_string()],
            away_aliases: vec!["Salford".to_string(), "Salford City FC".to_string()],
            requires_browser_automation: false,
            known_home_score: None,
            known_away_score: None,
            known_result_status: None,
            known_result_notes: None,
        };
        let fields = parse_flashscore_kv_feed("DA÷3¬DS÷0¬DE÷3¬DF÷0¬A1÷¬~");

        assert_eq!(flashscore_match_id(&link.url).as_deref(), Some("E3uvsQPP"));
        assert_eq!(
            flashscore_score_field(&fields, &["DE", "DG", "DA"]),
            Some(3)
        );
        assert_eq!(
            flashscore_score_field(&fields, &["DF", "DH", "DS"]),
            Some(0)
        );
        assert_eq!(
            external_link_participants(&link),
            ("Notts Co".to_string(), "Salford".to_string())
        );
    }

    #[test]
    fn parses_flashscore_danish_path_match_id_and_title_participants() {
        let url = "https://www.flashscore.dk/kamp/fodbold/andorra-dnO5z404/irak-K8aAGt6r/";

        assert_eq!(flashscore_match_id(url), None);
        assert_eq!(
            flashscore_url_participant_names(
                "https://www.flashscore.dk/kamp/basketball/dallas-wings-WlAAvRyL/las-vegas-aces-nZjYLTCd/?mid=88ogphkR"
            ),
            Some(("Dallas Wings".to_string(), "Las Vegas Aces".to_string()))
        );
        assert_eq!(
            parse_flashscore_title_participants(
                "Irak vs Andorra 29/05/2026 | Fodbold - Flashscore"
            ),
            Some(("Irak".to_string(), "Andorra".to_string()))
        );
    }

    #[test]
    fn parses_xscores_html_and_known_result_score() {
        let link = ExternalResultLink {
            source_key: "xscores_results".to_string(),
            url: "https://www.xscores.com/tennis/match/brendan-loh-vs-marcus-schoeman/26-05-2026/2783346".to_string(),
            sport_key: Some("tennis".to_string()),
            gender_scope: Some("men".to_string()),
            home_aliases: vec!["Brendan Loh".to_string()],
            away_aliases: vec!["Marcus Schoeman".to_string()],
            requires_browser_automation: false,
            known_home_score: Some(0),
            known_away_score: Some(2),
            known_result_status: Some("finished".to_string()),
            known_result_notes: Some("test fixture".to_string()),
        };
        let html = "<html><body>26-05-2026 / 05:30 Brendan Loh 0 - 2 Finished Marcus Schoeman</body></html>";
        let parsed = parse_xscores_match_result(html, &link).expect("xscores score");
        let known = known_external_match_result(&link).expect("known result");

        assert_eq!((parsed.home_score, parsed.away_score), (0, 2));
        assert_eq!(known.title, "Brendan Loh - Marcus Schoeman 0:2");
        assert_eq!(
            xscores_url_participant_names(&link.url),
            Some(("Brendan Loh".to_string(), "Marcus Schoeman".to_string()))
        );
    }

    #[test]
    fn external_check_grace_is_relative_to_expected_finish() {
        let start = parse_rfc3339_utc("2026-05-25T18:00:00Z");

        assert_eq!(
            expected_event_finish_after_for_sport("football", start),
            parse_rfc3339_utc("2026-05-25T20:10:00Z")
        );
        assert_eq!(
            external_result_check_after_for_sport("football", start, 120),
            parse_rfc3339_utc("2026-05-25T22:10:00Z")
        );
    }

    #[test]
    fn external_result_links_include_all_known_sources() {
        let policy = json!({
            "items": [
                {
                    "source_key": "flashscore_results",
                    "payload": {
                        "known_matches": [
                            {
                                "event_name": "Notts County - Salford City FC",
                                "url": "https://www.flashscore.com/match/example",
                                "home_aliases": ["Notts Co"],
                                "away_aliases": ["Salford"]
                            }
                        ]
                    }
                },
                {
                    "source_key": "sofascore_results",
                    "payload": {
                        "requires_browser_automation": true,
                        "known_matches": [
                            {
                                "event_name": "Notts County - Salford City FC",
                                "url": "https://www.sofascore.com/example",
                                "home_aliases": ["Notts County"],
                                "away_aliases": ["Salford City FC"]
                            }
                        ]
                    }
                }
            ]
        });

        let links = external_result_links_for_event(&policy, "Notts County - Salford City FC");
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].source_key, "flashscore_results");
        assert!(!links[0].requires_browser_automation);
        assert_eq!(links[1].source_key, "sofascore_results");
        assert!(links[1].requires_browser_automation);
    }

    #[test]
    fn tennis_doubles_result_links_do_not_use_global_alias_registry() {
        assert!(!use_external_result_alias_registry(
            Some("tennis"),
            "Shimizu Y / Watanabe S - Basel V / Oliveira B"
        ));
        assert!(use_external_result_alias_registry(
            Some("tennis"),
            "Casper Ruud - Tommy Paul"
        ));
        assert!(use_external_result_alias_registry(
            Some("basketball"),
            "Team A / Sponsor - Team B / Sponsor"
        ));
    }

    #[test]
    fn parses_task_source_link_for_direct_evidence_check() {
        let link = external_result_link_from_task_source(&json!({
            "source_key": "flashscore_results",
            "source_url": "https://www.flashscore.com/match/football/psg-CjhkPw0k/arsenal-hA1Zm19f/?mid=EJZRaQ15",
            "sport_key": "football",
            "home_aliases": ["Paris SG", "PSG", "Paris Saint-Germain"],
            "away_aliases": ["Arsenal"],
            "requires_browser_automation": false,
            "known_result": {
                "home_score": 2,
                "away_score": 1,
                "status": "finished",
                "notes": "fixture"
            }
        }))
        .expect("source link");

        assert_eq!(link.source_key, "flashscore_results");
        assert_eq!(link.known_home_score, Some(2));
        assert_eq!(link.known_away_score, Some(1));
        assert_eq!(link.known_result_status.as_deref(), Some("finished"));
        assert!(link.home_aliases.contains(&"PSG".to_string()));
    }

    #[test]
    fn validates_external_result_url_host() {
        assert!(validate_external_result_url(
            "flashscore_results",
            "https://www.flashscore.com/match/example"
        )
        .is_ok());
        assert!(validate_external_result_url(
            "flashscore_results",
            "https://www.flashscore.dk/kamp/fodbold/andorra-dnO5z404/irak-K8aAGt6r/"
        )
        .is_ok());
        assert!(validate_external_result_url(
            "sofascore_results",
            "https://www.sofascore.com/da/football/match/vasco-da-gama-america-mineiro/WzocsKgAc"
        )
        .is_ok());
        assert!(validate_external_result_url(
            "sofascore_results",
            "https://www.flashscore.com/match/example"
        )
        .is_err());
        assert!(
            validate_external_result_url("flashscore_results", "file:///tmp/result.html").is_err()
        );
    }

    #[test]
    fn external_result_links_match_reversed_neutral_event_names() {
        let policy = json!({
            "items": [
                {
                    "source_key": "flashscore_results",
                    "payload": {
                        "known_matches": [
                            {
                                "event_name": "Irak - Andorra",
                                "url": "https://www.flashscore.dk/kamp/fodbold/andorra-dnO5z404/irak-K8aAGt6r/",
                                "home_aliases": ["Irak", "Iraq"],
                                "away_aliases": ["Andorra"],
                                "home_score": 1,
                                "away_score": 0,
                                "result_status": "finished"
                            }
                        ]
                    }
                }
            ]
        });

        let links = external_result_links_for_event(&policy, "Andorra - Irak");

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].home_aliases, vec!["Irak", "Iraq"]);
        assert_eq!(links[0].away_aliases, vec!["Andorra"]);
        assert_eq!(links[0].known_home_score, Some(1));
        assert_eq!(links[0].known_away_score, Some(0));
    }

    #[test]
    fn known_neutral_friendly_result_can_be_oriented_to_event_order() {
        let link = ExternalResultLink {
            source_key: "flashscore_results".to_string(),
            url: "https://www.flashscore.dk/kamp/fodbold/andorra-dnO5z404/irak-K8aAGt6r/"
                .to_string(),
            sport_key: Some("football".to_string()),
            gender_scope: None,
            home_aliases: vec!["Andorra".to_string()],
            away_aliases: vec!["Irak".to_string(), "Iraq".to_string()],
            requires_browser_automation: false,
            known_home_score: Some(0),
            known_away_score: Some(1),
            known_result_status: Some("finished".to_string()),
            known_result_notes: Some("test fixture".to_string()),
        };
        let evidence = known_external_match_result(&link).expect("known result");

        assert_eq!(evidence.title, "Andorra - Irak 0:1");
        assert_eq!(grade_winner_outcome("Irak", &link, &evidence), Some("won"));
        assert_eq!(
            grade_winner_outcome("Andorra", &link, &evidence),
            Some("lost")
        );
        assert_eq!(
            grade_winner_outcome("Uafgjort", &link, &evidence),
            Some("lost")
        );
    }

    #[test]
    fn known_basketball_results_can_be_oriented_to_event_order() {
        let palencia_link = ExternalResultLink {
            source_key: "flashscore_results".to_string(),
            url: "https://www.flashscore.dk/kamp/basketball/fuenlabrada-E1z0hlIr/palencia-hMgAw6Je/?mid=4UaIOcR6".to_string(),
            sport_key: Some("basketball".to_string()),
            gender_scope: None,
            home_aliases: vec!["CD Maristas Palencia".to_string(), "Palencia".to_string()],
            away_aliases: vec!["Cb Fuenlabrada".to_string(), "Fuenlabrada".to_string()],
            requires_browser_automation: false,
            known_home_score: Some(51),
            known_away_score: Some(76),
            known_result_status: Some("finished".to_string()),
            known_result_notes: Some("test fixture".to_string()),
        };
        let palencia_evidence = known_external_match_result(&palencia_link).expect("known result");

        assert_eq!(
            palencia_evidence.title,
            "CD Maristas Palencia - Cb Fuenlabrada 51:76"
        );
        assert_eq!(
            grade_winner_outcome("Cb Fuenlabrada", &palencia_link, &palencia_evidence),
            Some("won")
        );
        assert_eq!(
            grade_winner_outcome("CD Maristas Palencia", &palencia_link, &palencia_evidence),
            Some("lost")
        );

        let nsa_link = ExternalResultLink {
            source_key: "flashscore_results".to_string(),
            url: "https://www.flashscore.com/match/basketball/antonine-xMbwy4Uk/nsa-xjRIpje7/"
                .to_string(),
            sport_key: Some("basketball".to_string()),
            gender_scope: None,
            home_aliases: vec!["Nsa".to_string(), "NSA".to_string()],
            away_aliases: vec![
                "Club Antonin Sportif".to_string(),
                "Antonine".to_string(),
                "Antonin".to_string(),
            ],
            requires_browser_automation: false,
            known_home_score: Some(77),
            known_away_score: Some(84),
            known_result_status: Some("finished".to_string()),
            known_result_notes: Some("test fixture".to_string()),
        };
        let nsa_evidence = known_external_match_result(&nsa_link).expect("known result");

        assert_eq!(nsa_evidence.title, "Nsa - Club Antonin Sportif 77:84");
        assert_eq!(
            flashscore_match_id(&nsa_link.url),
            None,
            "no-mid Flashscore URLs must fall back to page-title or known-result evidence"
        );
        assert_eq!(
            grade_winner_outcome("Club Antonin Sportif", &nsa_link, &nsa_evidence),
            Some("won")
        );
        assert_eq!(
            grade_winner_outcome("Nsa", &nsa_link, &nsa_evidence),
            Some("lost")
        );
    }

    #[test]
    fn grades_winner_market_against_aliases() {
        let link = ExternalResultLink {
            source_key: "flashscore_results".to_string(),
            url: "https://example.test/match".to_string(),
            sport_key: Some("football".to_string()),
            gender_scope: None,
            home_aliases: vec!["Lyngby".to_string(), "Lyngby AC".to_string()],
            away_aliases: vec!["Horsens".to_string(), "AC Horsens".to_string()],
            requires_browser_automation: false,
            known_home_score: None,
            known_away_score: None,
            known_result_status: None,
            known_result_notes: None,
        };
        let evidence = ExternalMatchResult {
            source_key: link.source_key.clone(),
            url: link.url.clone(),
            title: "Lyngby - Horsens 0:2".to_string(),
            home_name: "Lyngby".to_string(),
            away_name: "Horsens".to_string(),
            home_score: 0,
            away_score: 2,
            confidence: 0.86,
        };

        assert_eq!(
            grade_winner_outcome("Horsens", &link, &evidence),
            Some("won")
        );
        assert_eq!(
            grade_winner_outcome("Lyngby", &link, &evidence),
            Some("lost")
        );
        assert_eq!(
            grade_winner_outcome("Uafgjort", &link, &evidence),
            Some("lost")
        );
    }

    #[test]
    fn grades_neutral_friendly_when_source_order_is_reversed() {
        let link = ExternalResultLink {
            source_key: "flashscore_results".to_string(),
            url: "https://www.flashscore.dk/kamp/fodbold/andorra-dnO5z404/irak-K8aAGt6r/"
                .to_string(),
            sport_key: Some("football".to_string()),
            gender_scope: None,
            home_aliases: vec!["Andorra".to_string()],
            away_aliases: vec!["Irak".to_string(), "Iraq".to_string()],
            requires_browser_automation: false,
            known_home_score: None,
            known_away_score: None,
            known_result_status: None,
            known_result_notes: None,
        };
        let evidence = ExternalMatchResult {
            source_key: link.source_key.clone(),
            url: link.url.clone(),
            title: "Irak - Andorra 1:0".to_string(),
            home_name: "Irak".to_string(),
            away_name: "Andorra".to_string(),
            home_score: 1,
            away_score: 0,
            confidence: 0.86,
        };

        assert_eq!(grade_winner_outcome("Irak", &link, &evidence), Some("won"));
        assert_eq!(
            grade_winner_outcome("Andorra", &link, &evidence),
            Some("lost")
        );
    }

    #[test]
    fn grades_tennis_when_source_uses_surname_first_order() {
        let link = ExternalResultLink {
            source_key: "flashscore_results".to_string(),
            url: "https://www.flashscore.dk/kamp/tennis/paul-tommy-pd3ye1BS/ruud-casper-zN9UpRqp/?mid=UHlr5dhM".to_string(),
            sport_key: Some("tennis".to_string()),
            gender_scope: Some("men".to_string()),
            home_aliases: vec!["Casper Ruud".to_string()],
            away_aliases: vec!["Tommy Paul".to_string()],
            requires_browser_automation: false,
            known_home_score: None,
            known_away_score: None,
            known_result_status: None,
            known_result_notes: None,
        };
        let evidence = ExternalMatchResult {
            source_key: link.source_key.clone(),
            url: link.url.clone(),
            title: "Paul Tommy - Ruud Casper 2:0".to_string(),
            home_name: "Paul Tommy".to_string(),
            away_name: "Ruud Casper".to_string(),
            home_score: 2,
            away_score: 0,
            confidence: 0.86,
        };

        assert_eq!(
            grade_winner_outcome("Tommy Paul", &link, &evidence),
            Some("won")
        );
        assert_eq!(
            grade_winner_outcome("Casper Ruud", &link, &evidence),
            Some("lost")
        );
    }

    #[test]
    fn grades_draw_outcomes() {
        let link = ExternalResultLink {
            source_key: "flashscore_results".to_string(),
            url: "https://example.test/match".to_string(),
            sport_key: Some("football".to_string()),
            gender_scope: None,
            home_aliases: vec!["Notts County".to_string()],
            away_aliases: vec!["Salford City".to_string()],
            requires_browser_automation: false,
            known_home_score: None,
            known_away_score: None,
            known_result_status: None,
            known_result_notes: None,
        };
        let evidence = ExternalMatchResult {
            source_key: link.source_key.clone(),
            url: link.url.clone(),
            title: "Notts County - Salford City 1:1".to_string(),
            home_name: "Notts County".to_string(),
            away_name: "Salford City".to_string(),
            home_score: 1,
            away_score: 1,
            confidence: 0.86,
        };

        assert_eq!(
            grade_winner_outcome("Uafgjort", &link, &evidence),
            Some("won")
        );
        assert_eq!(
            grade_winner_outcome("Notts County", &link, &evidence),
            Some("lost")
        );
    }

    #[test]
    fn grades_over_under_totals() {
        let link = ExternalResultLink {
            source_key: "flashscore_results".to_string(),
            url: "https://example.test/match".to_string(),
            sport_key: Some("basketball".to_string()),
            gender_scope: None,
            home_aliases: vec!["Team FOG Naestved".to_string()],
            away_aliases: vec!["Bakken Bears".to_string()],
            requires_browser_automation: false,
            known_home_score: None,
            known_away_score: None,
            known_result_status: None,
            known_result_notes: None,
        };
        let evidence = ExternalMatchResult {
            source_key: link.source_key.clone(),
            url: link.url.clone(),
            title: "Team FOG Naestved - Bakken Bears 81:87".to_string(),
            home_name: "Team FOG Naestved".to_string(),
            away_name: "Bakken Bears".to_string(),
            home_score: 81,
            away_score: 87,
            confidence: 0.82,
        };

        assert_eq!(
            grade_external_outcome(
                "Over",
                Some("over_under"),
                Some("Antal point O/U 180,5"),
                &link,
                &evidence
            ),
            Some("lost")
        );
        assert_eq!(
            grade_external_outcome(
                "Under",
                Some("over_under"),
                Some("Antal point O/U 180,5"),
                &link,
                &evidence
            ),
            Some("won")
        );
    }

    #[test]
    fn motorsports_context_detects_series_family() {
        let context = motorsports_series_context(&json!({
            "competition": "NASCAR Cup Series",
            "class_name": "Motorsport",
            "name": "Coca-Cola 600"
        }));

        assert_eq!(
            context.get("series_family").and_then(Value::as_str),
            Some("nascar")
        );
        assert_eq!(
            context.get("vehicle_type").and_then(Value::as_str),
            Some("car")
        );
        assert_eq!(
            context.get("series_known").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn motorsports_feature_snapshot_marks_unknown_series_missing() {
        let features = event_feature_snapshot(
            "motorsports",
            &json!({
                "id": "event-1",
                "name": "Racing Special",
                "competition": "Motorsport",
                "start_time": "2026-06-01T12:00:00Z",
                "markets": [],
                "external_ids": []
            }),
        );

        let missing = features
            .get("missing_signals")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(missing
            .iter()
            .any(|item| item.as_str() == Some("motorsports_series")));
        assert_eq!(
            features
                .get("sport_context")
                .and_then(|context| context.get("series_family"))
                .and_then(Value::as_str),
            Some("unknown")
        );
    }
}

#[derive(Debug, Clone)]
struct StrategyEvalDecision {
    decision: String,
    rejection_reasons: Vec<String>,
}

#[derive(Debug, Clone)]
struct StrategyEval {
    selected_count: usize,
    rejected_count: usize,
    reason_counts: BTreeMap<String, usize>,
    decisions: HashMap<String, StrategyEvalDecision>,
}

impl StrategyEval {
    fn summary_json(&self) -> Value {
        json!({
            "selected_count": self.selected_count,
            "rejected_count": self.rejected_count,
            "rejection_reason_counts": self.reason_counts
        })
    }
}

fn strategy_eval_for_config(candidates: &[CandidateBet], config: &Value) -> StrategyEval {
    let mut eval = StrategyEval {
        selected_count: 0,
        rejected_count: 0,
        reason_counts: BTreeMap::new(),
        decisions: HashMap::new(),
    };
    for candidate in candidates {
        let rejection_reasons = strategy_rejection_reasons(candidate, config);
        let decision = if rejection_reasons.is_empty() {
            eval.selected_count += 1;
            "selected"
        } else {
            eval.rejected_count += 1;
            for reason in &rejection_reasons {
                *eval.reason_counts.entry(reason.clone()).or_default() += 1;
            }
            "rejected"
        };
        eval.decisions.insert(
            candidate.id.clone(),
            StrategyEvalDecision {
                decision: decision.to_string(),
                rejection_reasons,
            },
        );
    }
    eval
}

fn strategy_rejection_reasons(candidate: &CandidateBet, config: &Value) -> Vec<String> {
    let max_decimal_odds = config
        .get("max_decimal_odds")
        .and_then(Value::as_f64)
        .unwrap_or(8.0);
    let min_confidence = config
        .get("min_confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.1);
    let allow_live_markets = config
        .get("allow_live_markets")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let excluded_market_kinds: HashSet<String> = config
        .get("excluded_market_kinds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    let excluded_risk_flags: HashSet<String> = config
        .get("excluded_risk_flags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();

    let mut reasons = Vec::new();
    match candidate.decimal_odds {
        Some(odds) if odds > max_decimal_odds => reasons.push("above_max_decimal_odds".to_string()),
        Some(_) => {}
        None => reasons.push("missing_decimal_odds".to_string()),
    }
    if candidate.confidence.unwrap_or_default() < min_confidence {
        reasons.push("below_min_confidence".to_string());
    }
    if candidate
        .market_kind
        .as_ref()
        .is_some_and(|kind| excluded_market_kinds.contains(kind))
    {
        reasons.push("excluded_market_kind".to_string());
    }
    if candidate
        .risk_flags
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|flag| excluded_risk_flags.contains(flag))
    {
        reasons.push("excluded_risk_flag".to_string());
    }
    if !allow_live_markets
        && candidate
            .feature_snapshot
            .get("live_now")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        reasons.push("live_market_disabled".to_string());
    }
    reasons
}

fn strategy_config_with_change(
    baseline_config: &Value,
    variable_name: &str,
    proposed_value: Value,
) -> Value {
    let mut config = baseline_config.clone();
    if let Some(object) = config.as_object_mut() {
        object.insert(variable_name.to_string(), proposed_value);
    }
    config
}

fn strategy_replay_candidate_example(candidate: &CandidateBet) -> Value {
    json!({
        "candidate_id": candidate.id,
        "sport_key": candidate.sport_key,
        "event_name": candidate.event_name,
        "competition": candidate.competition,
        "market_kind": candidate.market_kind,
        "market_name": candidate.market_name,
        "outcome_name": candidate.outcome_name,
        "decimal_odds": candidate.decimal_odds,
        "score": candidate.score,
        "confidence": candidate.confidence,
        "risk_flags": candidate.risk_flags
    })
}

fn candidate_from_row(row: &Row) -> CandidateBet {
    CandidateBet {
        id: row.get("id"),
        snapshot_id: row.get("snapshot_id"),
        created_at: row.get("created_at"),
        sport_key: row.get("sport_key"),
        event_id: row.get("event_id"),
        event_name: row.get("event_name"),
        competition: row.get("competition"),
        market_id: row.get("market_id"),
        market_name: row.get("market_name"),
        market_kind: row.get("market_kind"),
        outcome_id: row.get("outcome_id"),
        outcome_name: row.get("outcome_name"),
        decimal_odds: row.get("decimal_odds"),
        rationale: row.get("rationale"),
        implied_probability: row.get("implied_probability"),
        model_probability: row.get("model_probability"),
        expected_value: row.get("expected_value"),
        confidence: row.get("confidence"),
        score: row.get("score"),
        risk_flags: row.get("risk_flags"),
        feature_snapshot: row.get("feature_snapshot"),
        status: row.get("status"),
    }
}

fn simulated_bet_from_row(row: &Row) -> SimulatedBet {
    SimulatedBet {
        id: row.get("id"),
        candidate_id: row.get("candidate_id"),
        created_at: row.get("created_at"),
        sport_key: row.get("sport_key"),
        event_name: row.get("event_name"),
        competition: row.get("competition"),
        market_name: row.get("market_name"),
        market_kind: row.get("market_kind"),
        outcome_name: row.get("outcome_name"),
        hypothetical_stake: row.get("hypothetical_stake"),
        observed_decimal_odds: row.get("observed_decimal_odds"),
        status: row.get("status"),
        strategy_id: row.get("strategy_id"),
        event_start_time: row.get("event_start_time"),
        expected_result_check_after: row.get("expected_result_check_after"),
        settled_at: row.get("settled_at"),
        simulated_return: row.get("simulated_return"),
        profit_loss: row.get("profit_loss"),
        settlement_payload: row.get("settlement_payload"),
        payload: row.get("payload"),
    }
}

fn performance_snapshot_from_row(row: &Row) -> Value {
    let created_at: DateTime<Utc> = row.get("created_at");
    json!({
        "id": row.get::<_, String>("id"),
        "created_at": created_at,
        "source": row.get::<_, String>("source"),
        "odds_snapshot_id": row.get::<_, Option<String>>("odds_snapshot_id"),
        "ledger": row.get::<_, Value>("ledger"),
        "played": row.get::<_, Value>("played"),
        "performance": row.get::<_, Value>("performance"),
        "payload": row.get::<_, Value>("payload")
    })
}

fn is_open_settlement_status(status: &str) -> bool {
    matches!(
        status,
        "open" | "awaiting_result" | "unresolved" | "postponed"
    )
}

fn is_closed_settlement_status(status: &str) -> bool {
    status.starts_with("settled_")
        || matches!(
            status,
            "void" | "pushed" | "cancelled" | "abandoned" | "refunded"
        )
}

fn json_status_open_count(statuses: &Value) -> i64 {
    json_status_count_by(statuses, is_open_settlement_status)
}

fn json_status_closed_count(statuses: &Value) -> i64 {
    json_status_count_by(statuses, is_closed_settlement_status)
}

fn json_status_count_by(statuses: &Value, predicate: fn(&str) -> bool) -> i64 {
    statuses
        .as_object()
        .into_iter()
        .flat_map(|items| items.iter())
        .filter(|(status, _)| predicate(status.as_str()))
        .map(|(_, count)| {
            count
                .as_i64()
                .or_else(|| count.as_u64().map(|value| value as i64))
        })
        .sum::<Option<i64>>()
        .unwrap_or(0)
}

fn candidate_event_start_time(candidate: &CandidateBet) -> Option<DateTime<Utc>> {
    candidate
        .feature_snapshot
        .get("start_time")
        .and_then(Value::as_str)
        .and_then(parse_rfc3339_utc)
}

fn leg_event_start_time(leg: &Value) -> Option<DateTime<Utc>> {
    leg.get("payload")
        .and_then(|payload| payload.get("candidate"))
        .and_then(|candidate| candidate.get("feature_snapshot"))
        .and_then(|features| features.get("start_time"))
        .and_then(Value::as_str)
        .and_then(parse_rfc3339_utc)
}

fn leg_expected_result_check_after(leg: &Value) -> Option<DateTime<Utc>> {
    let sport_key = leg
        .get("payload")
        .and_then(|payload| payload.get("candidate"))
        .and_then(|candidate| candidate.get("sport_key"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    expected_result_check_after_for_sport(sport_key, leg_event_start_time(leg))
}

fn coupon_latest_event_start_time(legs: &[Value]) -> Option<DateTime<Utc>> {
    legs.iter().filter_map(leg_event_start_time).max()
}

fn coupon_expected_result_check_after(legs: &[Value]) -> Option<DateTime<Utc>> {
    legs.iter()
        .filter_map(leg_expected_result_check_after)
        .max()
}

fn expected_result_check_after_for_sport(
    sport_key: &str,
    event_start_time: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    expected_event_finish_after_for_sport(sport_key, event_start_time)
}

fn expected_event_finish_after_for_sport(
    sport_key: &str,
    event_start_time: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    let start = event_start_time?;
    let duration = match sport_key {
        "football" => Duration::minutes(130),
        "basketball" => Duration::minutes(150),
        "tennis" => Duration::minutes(240),
        "motorsports" | "golf" | "cycling" => Duration::days(1),
        _ => Duration::hours(4),
    };
    Some(start + duration)
}

#[cfg(test)]
fn external_result_check_after_for_sport(
    sport_key: &str,
    event_start_time: Option<DateTime<Utc>>,
    grace_minutes: i64,
) -> Option<DateTime<Utc>> {
    expected_event_finish_after_for_sport(sport_key, event_start_time)
        .map(|finish| finish + Duration::minutes(grace_minutes.max(0)))
}

fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn merge_json_object(target: &mut Value, patch: Value) {
    if !target.is_object() {
        *target = json!({});
    }
    if let (Some(target), Some(patch)) = (target.as_object_mut(), patch.as_object()) {
        for (key, value) in patch {
            target.insert(key.clone(), value.clone());
        }
    }
}

fn strategy_experiment_from_row(row: &Row) -> Value {
    let created_at: DateTime<Utc> = row.get("created_at");
    let updated_at: DateTime<Utc> = row.get("updated_at");
    json!({
        "id": row.get::<_, String>("id"),
        "created_at": created_at,
        "updated_at": updated_at,
        "title": row.get::<_, String>("title"),
        "hypothesis": row.get::<_, String>("hypothesis"),
        "variable_name": row.get::<_, String>("variable_name"),
        "baseline_value": row.get::<_, Value>("baseline_value"),
        "proposed_value": row.get::<_, Value>("proposed_value"),
        "baseline_strategy_id": row.get::<_, String>("baseline_strategy_id"),
        "status": row.get::<_, String>("status"),
        "evidence": row.get::<_, Value>("evidence"),
        "decision_payload": row.get::<_, Value>("decision_payload")
    })
}

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}
