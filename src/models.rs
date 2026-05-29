use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateBet {
    pub id: String,
    pub snapshot_id: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub sport_key: String,
    pub event_id: Option<String>,
    pub event_name: Option<String>,
    pub competition: Option<String>,
    pub market_id: Option<String>,
    pub market_name: Option<String>,
    pub market_kind: Option<String>,
    pub outcome_id: Option<String>,
    pub outcome_name: Option<String>,
    pub decimal_odds: Option<f64>,
    pub rationale: Value,
    pub implied_probability: Option<f64>,
    pub model_probability: Option<f64>,
    pub expected_value: Option<f64>,
    pub confidence: Option<f64>,
    pub score: Option<f64>,
    pub risk_flags: Value,
    pub feature_snapshot: Value,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedBet {
    pub id: String,
    pub candidate_id: String,
    pub created_at: Option<DateTime<Utc>>,
    pub sport_key: Option<String>,
    pub event_name: Option<String>,
    pub competition: Option<String>,
    pub market_name: Option<String>,
    pub market_kind: Option<String>,
    pub outcome_name: Option<String>,
    pub hypothetical_stake: f64,
    pub observed_decimal_odds: Option<f64>,
    pub status: String,
    pub strategy_id: String,
    pub event_start_time: Option<DateTime<Utc>>,
    pub expected_result_check_after: Option<DateTime<Utc>>,
    pub settled_at: Option<DateTime<Utc>>,
    pub simulated_return: Option<f64>,
    pub profit_loss: Option<f64>,
    pub settlement_payload: Value,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerSummary {
    pub count: usize,
    pub open_count: usize,
    pub settled_count: usize,
    pub turnover: f64,
    pub open_exposure: f64,
    pub simulated_return: f64,
    pub profit_loss: f64,
    pub hit_rate: Option<f64>,
    pub average_odds: Option<f64>,
    pub by_status: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HermesReflection {
    pub id: String,
    pub created_at: Option<DateTime<Utc>>,
    pub title: String,
    pub summary: String,
    pub evidence: Value,
    pub status: String,
}
