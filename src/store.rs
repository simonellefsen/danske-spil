use crate::models::{CandidateBet, HermesReflection, LedgerSummary, SimulatedBet};
use anyhow::{anyhow, Context};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use tokio_postgres::{Client, NoTls, Row, Transaction};
use uuid::Uuid;

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

CREATE TABLE IF NOT EXISTS audit_events (
  id text PRIMARY KEY,
  created_at timestamptz NOT NULL DEFAULT now(),
  event_type text NOT NULL,
  details jsonb NOT NULL
);

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

CREATE INDEX IF NOT EXISTS idx_sport_events_sport_key ON sport_events(sport_key);
CREATE INDEX IF NOT EXISTS idx_sport_events_start_time ON sport_events(start_time);
CREATE INDEX IF NOT EXISTS idx_market_observations_snapshot ON market_observations(snapshot_id);
CREATE INDEX IF NOT EXISTS idx_market_observations_kind ON market_observations(market_kind);
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
                SELECT id, candidate_id, created_at, hypothetical_stake::float8 AS hypothetical_stake,
                       observed_decimal_odds::float8 AS observed_decimal_odds, status,
                       strategy_id, event_start_time, expected_result_check_after, settled_at,
                       simulated_return::float8 AS simulated_return,
                       profit_loss::float8 AS profit_loss,
                       settlement_payload, payload
                FROM simulated_bets
                WHERE candidate_id = $1
                ORDER BY created_at ASC
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
                        WHEN cb.sport_key IN ('formula1', 'golf', 'cycling') THEN COALESCE(sb.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '1 day'
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
                          WHEN cb.sport_key IN ('formula1', 'golf', 'cycling') THEN COALESCE(scl.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '1 day'
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
                            WHEN cb.sport_key IN ('formula1', 'golf', 'cycling') THEN COALESCE(scl.event_start_time, se.start_time, (cb.feature_snapshot->>'start_time')::timestamptz) + interval '1 day'
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

    pub async fn refresh_settlement_review_queue(&self, limit: usize) -> anyhow::Result<Value> {
        if limit == 0 {
            return Ok(json!({
                "enabled": true,
                "review_count": 0,
                "skipped": true,
                "reason": "settlement review limit is zero"
            }));
        }

        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                WITH review AS (
                  SELECT
                    sb.id AS bet_id,
                    sb.status AS bet_status,
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
                    COALESCE(
                      sb.expected_result_check_after,
                      CASE
                        WHEN COALESCE(sb.event_start_time, se.start_time) IS NULL THEN NULL
                        WHEN cb.sport_key = 'football' THEN COALESCE(sb.event_start_time, se.start_time) + interval '130 minutes'
                        WHEN cb.sport_key = 'basketball' THEN COALESCE(sb.event_start_time, se.start_time) + interval '150 minutes'
                        WHEN cb.sport_key = 'tennis' THEN COALESCE(sb.event_start_time, se.start_time) + interval '240 minutes'
                        WHEN cb.sport_key IN ('formula1', 'golf', 'cycling') THEN COALESCE(sb.event_start_time, se.start_time) + interval '1 day'
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
                    'latest_outcome_payload', review.latest_outcome_payload
                  )
                )
                FROM review
                WHERE sb.id = review.bet_id
                RETURNING
                  sb.id,
                  review.bet_status,
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
                  review.latest_outcome_payload
                "#,
                &[&(limit as i64)],
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
                    COALESCE(
                      scl.expected_result_check_after,
                      CASE
                        WHEN COALESCE(scl.event_start_time, se.start_time) IS NULL THEN NULL
                        WHEN cb.sport_key = 'football' THEN COALESCE(scl.event_start_time, se.start_time) + interval '130 minutes'
                        WHEN cb.sport_key = 'basketball' THEN COALESCE(scl.event_start_time, se.start_time) + interval '150 minutes'
                        WHEN cb.sport_key = 'tennis' THEN COALESCE(scl.event_start_time, se.start_time) + interval '240 minutes'
                        WHEN cb.sport_key IN ('formula1', 'golf', 'cycling') THEN COALESCE(scl.event_start_time, se.start_time) + interval '1 day'
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
                  WHERE sc.status IN ('awaiting_result', 'unresolved', 'postponed')
                ),
                review AS (
                  SELECT
                    simulated_coupon_id,
                    coupon_status,
                    coupon_id,
                    combined_decimal_odds,
                    strategy_id,
                    coupon_type,
                    leg_count,
                    max(expected_result_check_after) AS expected_result_check_after,
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
                        'latest_outcome_payload', latest_outcome_payload
                      )
                      ORDER BY leg_index
                    ) AS legs
                  FROM leg_review
                  GROUP BY simulated_coupon_id, coupon_status, coupon_id,
                           combined_decimal_odds, strategy_id, coupon_type, leg_count
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
                    'coupon_status', review.coupon_status,
                    'coupon_id', review.coupon_id,
                    'coupon_type', review.coupon_type,
                    'leg_count', review.leg_count,
                    'combined_decimal_odds', review.combined_decimal_odds,
                    'expected_result_check_after', review.expected_result_check_after,
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
                  review.combined_decimal_odds,
                  review.strategy_id,
                  review.expected_result_check_after,
                  review.legs
                "#,
                &[&(limit as i64)],
            )
            .await?;

        let mut items: Vec<Value> = rows
            .iter()
            .map(|row| {
                let start_time: Option<DateTime<Utc>> = row.get("start_time");
                let expected_result_check_after: Option<DateTime<Utc>> =
                    row.get("expected_result_check_after");
                let event_status: Option<String> = row.get("event_status");
                let event_resulted: Option<bool> = row.get("event_resulted");
                let event_settled: Option<bool> = row.get("event_settled");
                let recommendation = settlement_review_recommendation(
                    event_status.as_deref(),
                    event_resulted,
                    event_settled,
                    expected_result_check_after,
                );
                json!({
                    "item_type": "single",
                    "bet_id": row.get::<_, String>("id"),
                    "bet_status": row.get::<_, String>("bet_status"),
                    "candidate_id": row.get::<_, String>("candidate_id"),
                    "sport_key": row.get::<_, String>("sport_key"),
                    "event_id": row.get::<_, Option<String>>("event_id"),
                    "event_name": row.get::<_, Option<String>>("event_name"),
                    "competition": row.get::<_, Option<String>>("competition"),
                    "market_id": row.get::<_, Option<String>>("market_id"),
                    "market_name": row.get::<_, Option<String>>("market_name"),
                    "market_kind": row.get::<_, Option<String>>("market_kind"),
                    "outcome_id": row.get::<_, Option<String>>("outcome_id"),
                    "outcome_name": row.get::<_, Option<String>>("outcome_name"),
                    "observed_decimal_odds": row.get::<_, Option<f64>>("observed_decimal_odds"),
                    "start_time": start_time,
                    "expected_result_check_after": expected_result_check_after,
                    "event_status": event_status,
                    "event_resulted": event_resulted,
                    "event_settled": event_settled,
                    "latest_market_active": row.get::<_, Option<bool>>("latest_market_active"),
                    "latest_market_displayed": row.get::<_, Option<bool>>("latest_market_displayed"),
                    "latest_outcome_active": row.get::<_, Option<bool>>("latest_outcome_active"),
                    "latest_outcome_displayed": row.get::<_, Option<bool>>("latest_outcome_displayed"),
                    "latest_decimal_odds": row.get::<_, Option<f64>>("latest_decimal_odds"),
                    "latest_outcome_payload": row.get::<_, Option<Value>>("latest_outcome_payload"),
                    "recommendation": recommendation
                })
            })
            .collect();
        items.extend(coupon_rows.iter().map(|row| {
            let expected_result_check_after: Option<DateTime<Utc>> =
                row.get("expected_result_check_after");
            let legs: Value = row.get("legs");
            let recommendation =
                coupon_settlement_review_recommendation(&legs, expected_result_check_after);
            json!({
                "item_type": "coupon",
                "coupon_simulation_id": row.get::<_, String>("id"),
                "coupon_status": row.get::<_, String>("coupon_status"),
                "coupon_id": row.get::<_, Option<String>>("coupon_id"),
                "coupon_type": row.get::<_, String>("coupon_type"),
                "leg_count": row.get::<_, i32>("leg_count"),
                "combined_decimal_odds": row.get::<_, Option<f64>>("combined_decimal_odds"),
                "strategy_id": row.get::<_, String>("strategy_id"),
                "expected_result_check_after": expected_result_check_after,
                "legs": legs,
                "recommendation": recommendation
            })
        }));

        Ok(json!({
            "enabled": true,
            "review_count": rows.len() + coupon_rows.len(),
            "single_review_count": rows.len(),
            "coupon_review_count": coupon_rows.len(),
            "limit": limit,
            "items": items,
            "paper_only": true,
            "not_auto_graded": true
        }))
    }

    pub async fn simulated_bets(&self, limit: i64) -> anyhow::Result<Vec<SimulatedBet>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT id, candidate_id, created_at, hypothetical_stake::float8 AS hypothetical_stake,
                       observed_decimal_odds::float8 AS observed_decimal_odds, status,
                       strategy_id, event_start_time, expected_result_check_after, settled_at,
                       simulated_return::float8 AS simulated_return,
                       profit_loss::float8 AS profit_loss,
                       settlement_payload, payload
                FROM simulated_bets
                ORDER BY created_at DESC
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

    pub async fn strategy_played_summary(&self) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let by_strategy = client
            .query(
                r#"
                SELECT
                  sb.strategy_id,
                  count(*) FILTER (WHERE sb.status <> 'duplicate_void')::int AS played_count,
                  count(*) FILTER (WHERE sb.status IN ('open', 'awaiting_result', 'unresolved', 'postponed'))::int AS open_count,
                  count(*) FILTER (WHERE sb.status = 'awaiting_result')::int AS awaiting_result_count,
                  count(*) FILTER (WHERE sb.status = 'duplicate_void')::int AS duplicate_void_count,
                  COALESCE(sum(sb.hypothetical_stake) FILTER (WHERE sb.status IN ('open', 'awaiting_result', 'unresolved', 'postponed')), 0)::float8 AS open_exposure,
                  COALESCE(sum(sb.profit_loss) FILTER (WHERE sb.status <> 'duplicate_void'), 0)::float8 AS profit_loss
                FROM simulated_bets sb
                GROUP BY sb.strategy_id
                ORDER BY played_count DESC, sb.strategy_id
                "#,
                &[],
            )
            .await?;
        let by_sport = client
            .query(
                r#"
                SELECT
                  cb.sport_key,
                  sb.status,
                  count(*)::int AS count,
                  COALESCE(sum(sb.hypothetical_stake), 0)::float8 AS stake
                FROM simulated_bets sb
                LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                WHERE sb.status <> 'duplicate_void'
                GROUP BY cb.sport_key, sb.status
                ORDER BY cb.sport_key, sb.status
                "#,
                &[],
            )
            .await?;
        let recent = client
            .query(
                r#"
                SELECT
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
                ORDER BY sb.created_at DESC
                LIMIT 25
                "#,
                &[],
            )
            .await?;
        Ok(json!({
            "by_strategy": by_strategy.iter().map(|row| json!({
                "strategy_id": row.get::<_, String>("strategy_id"),
                "played_count": row.get::<_, i32>("played_count"),
                "open_count": row.get::<_, i32>("open_count"),
                "awaiting_result_count": row.get::<_, i32>("awaiting_result_count"),
                "duplicate_void_count": row.get::<_, i32>("duplicate_void_count"),
                "open_exposure": row.get::<_, f64>("open_exposure"),
                "profit_loss": row.get::<_, f64>("profit_loss")
            })).collect::<Vec<_>>(),
            "by_sport_status": by_sport.iter().map(|row| json!({
                "sport_key": row.get::<_, Option<String>>("sport_key"),
                "status": row.get::<_, String>("status"),
                "count": row.get::<_, i32>("count"),
                "stake": row.get::<_, f64>("stake")
            })).collect::<Vec<_>>(),
            "recent": recent.iter().map(|row| {
                let created_at: DateTime<Utc> = row.get("created_at");
                json!({
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
                  min(expected_result_check_after) FILTER (
                    WHERE status IN ('awaiting_result', 'unresolved', 'postponed')
                      AND expected_result_check_after <= now()
                  ) AS oldest_due_coupon
                FROM simulated_coupons
                "#,
                &[],
            )
            .await?;

        let by_sport = client
            .query(
                r#"
                SELECT
                  COALESCE(cb.sport_key, 'unknown') AS sport_key,
                  count(*) FILTER (WHERE sb.status <> 'duplicate_void')::int AS played_count,
                  count(*) FILTER (WHERE sb.status IN ('open', 'awaiting_result', 'unresolved', 'postponed'))::int AS open_count,
                  count(*) FILTER (WHERE sb.status = 'awaiting_result')::int AS awaiting_result_count,
                  count(*) FILTER (WHERE sb.status IN ('settled_won', 'settled_lost'))::int AS decided_count,
                  count(*) FILTER (WHERE sb.status = 'settled_won')::int AS won_count,
                  COALESCE(sum(sb.hypothetical_stake) FILTER (WHERE sb.status <> 'duplicate_void'), 0)::float8 AS turnover,
                  COALESCE(sum(sb.hypothetical_stake) FILTER (WHERE sb.status IN ('open', 'awaiting_result', 'unresolved', 'postponed')), 0)::float8 AS open_exposure,
                  COALESCE(sum(sb.profit_loss) FILTER (WHERE sb.status <> 'duplicate_void'), 0)::float8 AS profit_loss,
                  avg(sb.observed_decimal_odds) FILTER (WHERE sb.status <> 'duplicate_void')::float8 AS average_odds
                FROM simulated_bets sb
                LEFT JOIN candidate_bets cb ON cb.id = sb.candidate_id
                GROUP BY COALESCE(cb.sport_key, 'unknown')
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
                "due_coupon_count": due_coupon.get::<_, i32>("due_coupon_count"),
                "due_total": due_single.get::<_, i32>("due_single_count") + due_coupon.get::<_, i32>("due_coupon_count"),
                "oldest_due": oldest_due,
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
                    "decided_count": decided_count,
                    "won_count": won_count,
                    "hit_rate": if decided_count > 0 { Some(won_count as f64 / decided_count as f64) } else { None },
                    "turnover": row.get::<_, f64>("turnover"),
                    "open_exposure": row.get::<_, f64>("open_exposure"),
                    "profit_loss": row.get::<_, f64>("profit_loss"),
                    "average_odds": row.get::<_, Option<f64>>("average_odds")
                })
            }).collect::<Vec<_>>()
        }))
    }

    pub async fn market_catalog_coverage(&self) -> anyhow::Result<Value> {
        let client = self.connect().await?;
        let sports = client
            .query(
                r#"
                SELECT
                  s.sport_key,
                  s.label,
                  s.last_seen_at,
                  count(DISTINCT e.id)::int AS event_count,
                  count(DISTINCT c.id)::int AS competition_count,
                  count(DISTINCT mo.id)::int AS market_count,
                  count(DISTINCT oo.id)::int AS outcome_count,
                  count(DISTINCT cb.id)::int AS candidate_count
                FROM sports s
                LEFT JOIN sport_events e ON e.sport_key = s.sport_key
                LEFT JOIN competitions c ON c.sport_key = s.sport_key
                LEFT JOIN market_observations mo ON mo.event_id = e.id
                LEFT JOIN outcome_observations oo ON oo.market_observation_id = mo.id
                LEFT JOIN candidate_bets cb ON cb.sport_key = s.sport_key
                GROUP BY s.sport_key, s.label, s.last_seen_at
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
        let settlement_payload = json!({
            "source": source,
            "observed_result": result,
            "confidence": confidence,
            "notes": notes,
            "paper_only": true
        });
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
        let settlement_payload = json!({
            "source": source,
            "observed_result": result,
            "confidence": confidence,
            "notes": notes,
            "paper_only": true,
            "coupon_level": true
        });
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
                "risk_flags": candidate.risk_flags
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

    let mut missing = Vec::new();
    if text(event, "competition").is_none() {
        missing.push("competition");
    }
    if event.get("start_time").unwrap_or(&Value::Null).is_null() {
        missing.push("start_time");
    }
    if teams.is_empty() && !matches!(sport_key, "formula1" | "golf" | "cycling") {
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
        "missing_signals": missing,
        "confidence": confidence,
        "limits": {
            "paper_only": true,
            "not_settlement_grade": true,
            "uses_only_market_feed": true
        }
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
    if expected_result_check_after.is_some_and(|value| value <= Utc::now()) {
        return "expected_finish_passed_recheck";
    }
    "await_more_evidence"
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
    let start = event_start_time?;
    let duration = match sport_key {
        "football" => Duration::minutes(130),
        "basketball" => Duration::minutes(150),
        "tennis" => Duration::minutes(240),
        "formula1" | "golf" | "cycling" => Duration::days(1),
        _ => Duration::hours(4),
    };
    Some(start + duration)
}

fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
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
