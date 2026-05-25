use crate::models::{CandidateBet, HermesReflection, LedgerSummary, SimulatedBet};
use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::BTreeMap;
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

CREATE TABLE IF NOT EXISTS settlement_observations (
  id text PRIMARY KEY,
  simulated_bet_id text REFERENCES simulated_bets(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  source text NOT NULL,
  observed_result text NOT NULL,
  confidence numeric NOT NULL,
  payload jsonb NOT NULL
);

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

CREATE INDEX IF NOT EXISTS idx_sport_events_sport_key ON sport_events(sport_key);
CREATE INDEX IF NOT EXISTS idx_sport_events_start_time ON sport_events(start_time);
CREATE INDEX IF NOT EXISTS idx_market_observations_snapshot ON market_observations(snapshot_id);
CREATE INDEX IF NOT EXISTS idx_market_observations_kind ON market_observations(market_kind);
CREATE INDEX IF NOT EXISTS idx_outcome_observations_snapshot ON outcome_observations(snapshot_id);
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
        save_market_catalog(&transaction, &snapshot_id, payload).await?;
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

    pub async fn simulate_bet(
        &self,
        candidate_id: &str,
        stake: f64,
    ) -> anyhow::Result<SimulatedBet> {
        let candidate = self
            .candidates(200)
            .await?
            .into_iter()
            .find(|candidate| candidate.id == candidate_id)
            .ok_or_else(|| anyhow!("candidate not found: {candidate_id}"))?;
        let id = new_id();
        let payload =
            json!({"candidate": candidate, "paper_only": true, "strategy_id": "poc_ranker_v1"});
        let client = self.connect().await?;
        client
            .execute(
                r#"
                INSERT INTO simulated_bets (
                  id, candidate_id, hypothetical_stake, observed_decimal_odds, status,
                  strategy_id, settlement_payload, payload
                )
                VALUES ($1,$2,($3::float8)::numeric,($4::float8)::numeric,$5,$6,$7,$8)
                "#,
                &[
                    &id,
                    &candidate_id,
                    &stake,
                    &candidate.decimal_odds,
                    &"open",
                    &"poc_ranker_v1",
                    &json!({}),
                    &payload,
                ],
            )
            .await?;
        self.simulated_bets(1)
            .await?
            .into_iter()
            .find(|bet| bet.id == id)
            .ok_or_else(|| anyhow!("inserted simulated bet not found"))
    }

    pub async fn simulated_bets(&self, limit: i64) -> anyhow::Result<Vec<SimulatedBet>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                r#"
                SELECT id, candidate_id, created_at, hypothetical_stake::float8 AS hypothetical_stake,
                       observed_decimal_odds::float8 AS observed_decimal_odds, status,
                       strategy_id, settled_at,
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
            summary.turnover += bet.hypothetical_stake;
            summary.simulated_return += bet.simulated_return.unwrap_or_default();
            summary.profit_loss += bet.profit_loss.unwrap_or_default();
            if let Some(odds) = bet.observed_decimal_odds {
                odds_total += odds;
                odds_count += 1;
            }
            if matches!(
                bet.status.as_str(),
                "open" | "awaiting_result" | "unresolved"
            ) {
                summary.open_count += 1;
                summary.open_exposure += bet.hypothetical_stake;
            }
            if bet.status.starts_with("settled_")
                || matches!(bet.status.as_str(), "void" | "pushed")
            {
                summary.settled_count += 1;
            }
            if matches!(bet.status.as_str(), "settled_won" | "settled_lost") {
                decided += 1;
                if bet.status == "settled_won" {
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
            "open" | "awaiting_result" | "unresolved"
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
            "void" | "pushed" => (Some(bet.hypothetical_stake), Some(0.0)),
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
        }
    }
    Ok(())
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
        settled_at: row.get("settled_at"),
        simulated_return: row.get("simulated_return"),
        profit_loss: row.get("profit_loss"),
        settlement_payload: row.get("settlement_payload"),
        payload: row.get("payload"),
    }
}

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}
