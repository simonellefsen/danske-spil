use crate::config::Settings;
use crate::danske_spil::scan_sports;
use crate::models::CandidateBet;
use crate::store::{new_id, Store};
use serde_json::{json, Value};

const PRIMARY_MARKET_KINDS: &[&str] = &[
    "winner",
    "over_under",
    "handicap",
    "both_teams_score",
    "double_chance",
    "set_or_game",
    "period_or_quarter",
    "half_time",
    "corners",
    "goal",
    "outright",
];

#[derive(Clone)]
pub struct GamblerService {
    settings: Settings,
    store: Store,
}

impl GamblerService {
    pub fn new(settings: Settings, store: Store) -> Self {
        Self { settings, store }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub async fn status(&self) -> Value {
        let latest = self.store.latest_snapshot().await.ok().flatten();
        let candidates = self.store.candidates(5).await.unwrap_or_default();
        let ledger = self.store.simulated_bets(5).await.unwrap_or_default();
        let ledger_summary = self.store.ledger_summary().await.ok();
        json!({
            "component": self.settings.component,
            "mode": self.settings.mode,
            "observe_only": self.settings.observe_only,
            "allow_real_money_placement": self.settings.allow_real_money_placement,
            "database": self.store.status().await,
            "latest_snapshot_id": latest.as_ref().and_then(|item| item.get("id")).cloned(),
            "recent_candidate_count": candidates.len(),
            "recent_simulated_bet_count": ledger.len(),
            "ledger_summary": ledger_summary,
            "strategy_id": "poc_ranker_v1",
            "auto_paper": {
                "enabled": self.settings.auto_paper_enabled,
                "per_scan_limit": self.settings.auto_paper_per_scan_limit,
                "max_open_exposure": self.settings.auto_paper_max_open_exposure,
                "default_stake": self.settings.default_stake,
                "coupon_placement": "enabled_when_active_strategy_generates_candidate_coupons"
            },
            "settlement_queue": {
                "enabled": self.settings.settlement_queue_enabled,
                "awaiting_grace_minutes": self.settings.settlement_awaiting_grace_minutes,
                "limit": self.settings.settlement_queue_limit
            },
            "runtime": "rust-dioxus",
            "sports_scope": ["football", "tennis", "basketball", "formula1", "golf", "cycling"]
        })
    }

    pub async fn scan(&self, include_live: bool) -> anyhow::Result<Value> {
        let snapshot = scan_sports(
            self.settings.scan_limit,
            self.settings.scan_max_markets,
            include_live,
        )
        .await?;
        let mut candidates = build_candidates(&snapshot, 40);
        let snapshot_id = self.store.save_snapshot(&snapshot, &mut candidates).await?;
        let strategy_decision_summary = match self
            .store
            .apply_active_strategy(&snapshot_id, &candidates)
            .await
        {
            Ok(summary) => summary,
            Err(error) => {
                tracing::warn!(%error, snapshot_id = %snapshot_id, "strategy decision application failed");
                json!({
                    "snapshot_id": snapshot_id,
                    "selected_count": 0,
                    "rejected_count": 0,
                    "error": error.to_string()
                })
            }
        };
        let coupon_candidate_summary = match self
            .store
            .generate_candidate_coupons(&snapshot_id, 10)
            .await
        {
            Ok(summary) => summary,
            Err(error) => {
                tracing::warn!(%error, snapshot_id = %snapshot_id, "coupon candidate generation failed");
                json!({
                    "enabled": false,
                    "snapshot_id": snapshot_id,
                    "generated_count": 0,
                    "error": error.to_string()
                })
            }
        };
        let auto_paper_summary = if self.settings.auto_paper_enabled {
            match self
                .store
                .paper_place_selected(
                    Some(&snapshot_id),
                    self.settings.default_stake,
                    self.settings.auto_paper_per_scan_limit,
                    self.settings.auto_paper_max_open_exposure,
                )
                .await
            {
                Ok(summary) => summary,
                Err(error) => {
                    tracing::warn!(%error, snapshot_id = %snapshot_id, "auto paper placement failed");
                    json!({
                        "enabled": true,
                        "placed_count": 0,
                        "error": error.to_string()
                    })
                }
            }
        } else {
            json!({"enabled": false, "placed_count": 0})
        };
        let auto_coupon_paper_summary = if self.settings.auto_paper_enabled {
            match self
                .store
                .paper_place_candidate_coupons(
                    Some(&snapshot_id),
                    self.settings.default_stake,
                    self.settings.auto_paper_per_scan_limit,
                    self.settings.auto_paper_max_open_exposure,
                )
                .await
            {
                Ok(summary) => summary,
                Err(error) => {
                    tracing::warn!(%error, snapshot_id = %snapshot_id, "auto paper coupon placement failed");
                    json!({
                        "enabled": true,
                        "placed_count": 0,
                        "error": error.to_string()
                    })
                }
            }
        } else {
            json!({"enabled": false, "placed_count": 0})
        };
        let settlement_queue_summary = self.advance_settlement_queue().await;
        let settlement_review_summary = self.refresh_settlement_review_queue().await;
        let (strategy_proposal, strategy_proposal_error) = match self
            .store
            .ensure_scan_strategy_proposal(&snapshot_id, &candidates)
            .await
        {
            Ok(proposal) => (proposal, None),
            Err(error) => {
                tracing::warn!(%error, snapshot_id = %snapshot_id, "strategy proposal creation failed");
                (None, Some(error.to_string()))
            }
        };
        let performance_snapshot = match self
            .store
            .performance_report(
                self.settings.default_stake,
                self.settings.auto_paper_per_scan_limit,
                self.settings.auto_paper_max_open_exposure,
            )
            .await
        {
            Ok(report) => match self
                .store
                .record_performance_snapshot("scan_completed", Some(&snapshot_id), &report)
                .await
            {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    tracing::warn!(%error, snapshot_id = %snapshot_id, "performance snapshot recording failed");
                    json!({
                        "recorded": false,
                        "error": error.to_string()
                    })
                }
            },
            Err(error) => {
                tracing::warn!(%error, snapshot_id = %snapshot_id, "performance snapshot report failed");
                json!({
                    "recorded": false,
                    "error": error.to_string()
                })
            }
        };
        self.store
            .record_audit(
                "scan_completed",
                json!({
                    "snapshot_id": snapshot_id,
                    "candidate_count": candidates.len(),
                    "strategy_decision_summary": strategy_decision_summary,
                    "coupon_candidate_summary": coupon_candidate_summary,
                    "auto_paper_summary": auto_paper_summary,
                    "auto_coupon_paper_summary": auto_coupon_paper_summary,
                    "settlement_queue_summary": settlement_queue_summary,
                    "settlement_review_summary": settlement_review_summary,
                    "performance_snapshot_id": performance_snapshot.get("id").cloned().unwrap_or(Value::Null),
                    "include_live": include_live,
                    "paper_only": true,
                    "runtime": "rust-dioxus"
                }),
            )
            .await
            .ok();
        Ok(json!({
            "snapshot_id": snapshot_id,
            "candidate_count": candidates.len(),
            "strategy_decision_summary": strategy_decision_summary,
            "coupon_candidate_summary": coupon_candidate_summary,
            "auto_paper_summary": auto_paper_summary,
            "auto_coupon_paper_summary": auto_coupon_paper_summary,
            "settlement_queue_summary": settlement_queue_summary,
            "settlement_review_summary": settlement_review_summary,
            "strategy_proposal": strategy_proposal,
            "strategy_proposal_error": strategy_proposal_error,
            "performance_snapshot": performance_snapshot,
            "snapshot": snapshot
        }))
    }

    pub async fn advance_settlement_queue(&self) -> Value {
        if !self.settings.settlement_queue_enabled {
            return json!({"enabled": false, "transitioned_count": 0});
        }
        match self
            .store
            .advance_settlement_queue(
                self.settings.settlement_awaiting_grace_minutes,
                self.settings.settlement_queue_limit,
            )
            .await
        {
            Ok(summary) => {
                self.store
                    .record_audit("settlement_queue_advanced", summary.clone())
                    .await
                    .ok();
                summary
            }
            Err(error) => {
                tracing::warn!(%error, "settlement queue advance failed");
                json!({
                    "enabled": true,
                    "transitioned_count": 0,
                    "error": error.to_string()
                })
            }
        }
    }

    pub async fn refresh_settlement_review_queue(&self) -> Value {
        if !self.settings.settlement_queue_enabled {
            return json!({"enabled": false, "review_count": 0});
        }
        match self
            .store
            .refresh_settlement_review_queue(self.settings.settlement_queue_limit)
            .await
        {
            Ok(summary) => {
                self.store
                    .record_audit("settlement_review_refreshed", summary.clone())
                    .await
                    .ok();
                summary
            }
            Err(error) => {
                tracing::warn!(%error, "settlement review refresh failed");
                json!({
                    "enabled": true,
                    "review_count": 0,
                    "error": error.to_string()
                })
            }
        }
    }

    pub async fn performance_report(&self) -> Value {
        match self
            .store
            .performance_report(
                self.settings.default_stake,
                self.settings.auto_paper_per_scan_limit,
                self.settings.auto_paper_max_open_exposure,
            )
            .await
        {
            Ok(report) => report,
            Err(error) => {
                tracing::warn!(%error, "performance report failed");
                json!({
                    "paper_only": true,
                    "error": error.to_string()
                })
            }
        }
    }

    pub async fn performance_history(&self, limit: i64) -> Value {
        match self.store.performance_history(limit).await {
            Ok(history) => history,
            Err(error) => {
                tracing::warn!(%error, "performance history failed");
                json!({
                    "items": [],
                    "error": error.to_string()
                })
            }
        }
    }
}

pub fn build_candidates(snapshot: &Value, max_candidates: usize) -> Vec<CandidateBet> {
    let mut candidates = Vec::new();
    for sport in snapshot
        .get("sports")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let events = sport
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
            );
        for event in events {
            for market in event
                .get("markets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let kind = market
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !PRIMARY_MARKET_KINDS.contains(&kind) || !boolish(market.get("displayed")) {
                    continue;
                }
                for outcome in market
                    .get("outcomes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if !boolish(outcome.get("displayed")) {
                        continue;
                    }
                    let Some(decimal_odds) = outcome.get("decimal_odds").and_then(Value::as_f64)
                    else {
                        continue;
                    };
                    let features = candidate_features(sport, event, market, outcome);
                    let scoring = score_candidate(decimal_odds, &features);
                    let risk_flags = scoring
                        .get("risk_flags")
                        .cloned()
                        .unwrap_or_else(|| json!([]));
                    let candidate = CandidateBet {
                        id: new_id(),
                        snapshot_id: None,
                        created_at: None,
                        sport_key: text(sport, "sport_key").unwrap_or("unknown").to_string(),
                        event_id: text(event, "id").map(str::to_string),
                        event_name: text(event, "name").map(str::to_string),
                        competition: text(event, "competition").map(str::to_string),
                        market_id: text(market, "id").map(str::to_string),
                        market_name: text(market, "name").map(str::to_string),
                        market_kind: Some(kind.to_string()),
                        outcome_id: text(outcome, "id").map(str::to_string),
                        outcome_name: text(outcome, "name").map(str::to_string),
                        decimal_odds: Some(decimal_odds),
                        implied_probability: scoring
                            .get("implied_probability")
                            .and_then(Value::as_f64),
                        model_probability: scoring.get("model_probability").and_then(Value::as_f64),
                        expected_value: scoring.get("expected_value").and_then(Value::as_f64),
                        confidence: scoring.get("confidence").and_then(Value::as_f64),
                        score: scoring.get("score").and_then(Value::as_f64),
                        risk_flags: risk_flags.clone(),
                        feature_snapshot: features.clone(),
                        status: "candidate".to_string(),
                        rationale: json!({
                            "paper_only": true,
                            "strategy_id": "poc_ranker_v1",
                            "selection_basis": "Conservative watchlist score from odds shape and available market metadata; not a recommendation.",
                            "safety": "Real-money placement is disabled; candidate can only be paper-ledgered.",
                            "score_summary": scoring,
                            "evidence": {
                                "sport": sport.get("label").cloned().unwrap_or(Value::Null),
                                "competition": event.get("competition").cloned().unwrap_or(Value::Null),
                                "market_kind": kind,
                                "market_group_code": market.get("group_code").cloned().unwrap_or(Value::Null),
                                "start_time": event.get("start_time").cloned().unwrap_or(Value::Null),
                                "scoreboard_facts": event.get("scoreboard_facts").cloned().unwrap_or_else(|| json!([])),
                                "handicap_low": outcome.get("handicap_low").cloned().unwrap_or(Value::Null),
                                "handicap_high": outcome.get("handicap_high").cloned().unwrap_or(Value::Null)
                            }
                        }),
                    };
                    candidates.push(candidate);
                    if candidates.len() >= max_candidates {
                        candidates.sort_by(|a, b| {
                            b.score
                                .partial_cmp(&a.score)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                        return candidates;
                    }
                }
            }
        }
    }
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates
}

fn candidate_features(sport: &Value, event: &Value, market: &Value, outcome: &Value) -> Value {
    let teams = event
        .get("teams")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let scoreboard_facts = event
        .get("scoreboard_facts")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let start_time = event.get("start_time").cloned().unwrap_or(Value::Null);
    let market_kind = text(market, "kind").unwrap_or_default();
    let mut risk_flags = Vec::new();
    if start_time.is_null() {
        risk_flags.push("missing_start_time");
    }
    if teams == 0 && market_kind != "outright" {
        risk_flags.push("missing_participants");
    }
    if boolish(event.get("live_now")) && scoreboard_facts == 0 {
        risk_flags.push("missing_live_scoreboard");
    }
    if matches!(
        market_kind,
        "corners" | "goal" | "period_or_quarter" | "set_or_game" | "half_time"
    ) {
        risk_flags.push("specialized_market");
    }
    if market_kind == "outright" {
        risk_flags.push("long_horizon_market");
    }
    if !outcome
        .get("handicap_low")
        .unwrap_or(&Value::Null)
        .is_null()
        || !outcome
            .get("handicap_high")
            .unwrap_or(&Value::Null)
            .is_null()
    {
        risk_flags.push("line_market");
    }
    json!({
        "source": "danskespil_content_service",
        "sport_key": sport.get("sport_key").cloned().unwrap_or(Value::Null),
        "sport_label": sport.get("label").cloned().unwrap_or(Value::Null),
        "competition": event.get("competition").cloned().unwrap_or(Value::Null),
        "class_name": event.get("class_name").cloned().unwrap_or(Value::Null),
        "start_time": start_time,
        "live_now": boolish(event.get("live_now")),
        "started": boolish(event.get("started")),
        "team_count": teams,
        "scoreboard_fact_count": scoreboard_facts,
        "market_kind": market_kind,
        "market_group_code": market.get("group_code").cloned().unwrap_or(Value::Null),
        "minimum_accumulator": market.get("minimum_accumulator").cloned().unwrap_or(Value::Null),
        "maximum_accumulator": market.get("maximum_accumulator").cloned().unwrap_or(Value::Null),
        "handicap_low": outcome.get("handicap_low").cloned().unwrap_or(Value::Null),
        "handicap_high": outcome.get("handicap_high").cloned().unwrap_or(Value::Null),
        "risk_flags": risk_flags
    })
}

fn score_candidate(decimal_odds: f64, features: &Value) -> Value {
    let implied_probability = 1.0 / decimal_odds;
    let mut risk_flags: Vec<String> = features
        .get("risk_flags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    if decimal_odds < 1.25 {
        risk_flags.push("very_short_price".to_string());
    }
    if decimal_odds > 8.0 {
        risk_flags.push("long_price".to_string());
    }
    risk_flags.sort();
    risk_flags.dedup();

    let completeness =
        0.35 + if !features.get("start_time").unwrap_or(&Value::Null).is_null() {
            0.18
        } else {
            0.0
        } + if !features
            .get("competition")
            .unwrap_or(&Value::Null)
            .is_null()
        {
            0.18
        } else {
            0.0
        } + if features
            .get("team_count")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            > 0
        {
            0.14
        } else {
            0.0
        } + if !features
            .get("market_group_code")
            .unwrap_or(&Value::Null)
            .is_null()
        {
            0.10
        } else {
            0.0
        } + if features
            .get("scoreboard_fact_count")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            > 0
        {
            0.05
        } else {
            0.0
        };
    let confidence = (completeness - (0.04 * risk_flags.len() as f64)).clamp(0.1, 0.82);
    let kind_adjustment = match features
        .get("market_kind")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "winner" => 0.03,
        "over_under" => 0.015,
        "handicap" => 0.01,
        "both_teams_score" => 0.0,
        "double_chance" => -0.005,
        "set_or_game" => -0.02,
        "period_or_quarter" => -0.025,
        "half_time" => -0.02,
        "corners" => -0.03,
        "goal" => -0.015,
        "outright" => -0.04,
        _ => -0.03,
    };
    let odds_penalty = if decimal_odds > 5.0 { 0.04 } else { 0.0 };
    let model_probability =
        (implied_probability + kind_adjustment - odds_penalty - (0.01 * risk_flags.len() as f64))
            .clamp(0.01, 0.95);
    let expected_value = (model_probability * decimal_odds) - 1.0;
    let score = (expected_value * confidence) - (0.015 * risk_flags.len() as f64);
    json!({
        "implied_probability": implied_probability,
        "model_probability": model_probability,
        "expected_value": expected_value,
        "confidence": confidence,
        "score": score,
        "risk_flags": risk_flags
    })
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn boolish(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(false)
}
