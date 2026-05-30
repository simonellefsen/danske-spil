use crate::config::Settings;
use crate::danske_spil::scan_sports;
use crate::models::{CandidateBet, LedgerSummary};
use crate::store::{new_id, Store};
use chrono::{DateTime, Duration, Utc};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE};
use reqwest::{Client as HttpClient, Url};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration as StdDuration;

const FLASHSCORE_SEARCH_URL: &str = "https://s.flashscore.com/search/";
const FLASHSCORE_BASE_URL: &str = "https://www.flashscore.com";
const FLASHSCORE_SOURCE_KEY: &str = "flashscore_results";
const FLASHSCORE_DEFAULT_XFSIGN: &str = "SW9D1eZo";
const FLASHSCORE_FINISHED_STAGES: &[&str] = &["3", "10", "11"];

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

const HERMES_MIN_SETTLED_FOR_PROMOTION: usize = 100;

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
        let latest_observed_at = latest
            .as_ref()
            .and_then(|item| item.get("observed_at"))
            .and_then(Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        let latest_snapshot_age_seconds =
            latest_observed_at.map(|observed_at| (Utc::now() - observed_at).num_seconds().max(0));
        let next_scan_due_at = latest_observed_at.map(|observed_at| {
            observed_at + Duration::seconds(self.settings.scan_interval_seconds as i64)
        });
        let candidates = self.store.candidates(5).await.unwrap_or_default();
        let ledger = self.store.simulated_bets(5).await.unwrap_or_default();
        let ledger_summary = self.store.ledger_summary().await.ok();
        let latest_hermes_reflection = self
            .store
            .hermes_reflections(1)
            .await
            .ok()
            .and_then(|items| items.into_iter().next());
        json!({
            "component": self.settings.component,
            "mode": self.settings.mode,
            "observe_only": self.settings.observe_only,
            "allow_real_money_placement": self.settings.allow_real_money_placement,
            "database": self.store.status().await,
            "latest_snapshot_id": latest.as_ref().and_then(|item| item.get("id")).cloned(),
            "latest_snapshot_observed_at": latest_observed_at,
            "latest_snapshot_age_seconds": latest_snapshot_age_seconds,
            "recent_candidate_count": candidates.len(),
            "recent_simulated_bet_count": ledger.len(),
            "ledger_summary": ledger_summary,
            "strategy_id": "poc_ranker_v1",
            "scanner": {
                "interval_seconds": self.settings.scan_interval_seconds,
                "scan_limit": self.settings.scan_limit,
                "scan_max_markets": self.settings.scan_max_markets,
                "latest_snapshot_observed_at": latest_observed_at,
                "latest_snapshot_age_seconds": latest_snapshot_age_seconds,
                "next_scan_due_at": next_scan_due_at,
                "due": next_scan_due_at
                    .map(|due_at| due_at <= Utc::now())
                    .unwrap_or(true)
            },
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
                "limit": self.settings.settlement_queue_limit,
                "lookup_cooldown_minutes": self.settings.settlement_lookup_cooldown_minutes,
                "result_agent_enabled": self.settings.result_agent_enabled,
                "result_agent_per_cycle_limit": self.settings.result_agent_per_cycle_limit,
                "result_agent_interval_seconds": self.settings.result_agent_interval_seconds,
                "result_agent_remote_url_present": self.settings.result_agent_url.is_some()
            },
            "hermes": {
                "enabled": self.settings.hermes_agent_enabled,
                "mode": "sanitized_reflection_and_one_variable_proposals",
                "reflection_interval_seconds": self.settings.hermes_reflection_interval_seconds,
                "latest_reflection": latest_hermes_reflection,
                "browser_control": false,
                "credential_access": false,
                "real_money_placement": false
            },
            "runtime": "rust-dioxus",
            "sports_scope": ["football", "tennis", "basketball", "motorsports", "golf", "cycling"]
        })
    }

    pub async fn run_hermes_cycle_once(&self, trigger: &str) -> Value {
        if !self.settings.hermes_agent_enabled {
            return json!({
                "enabled": false,
                "trigger": trigger,
                "paper_only": true
            });
        }

        let reflection = match self.store.record_daily_reflection(None).await {
            Ok(reflection) => reflection,
            Err(error) => {
                tracing::warn!(%error, trigger, "hermes reflection cycle failed");
                let summary = json!({
                    "enabled": true,
                    "trigger": trigger,
                    "recorded": false,
                    "error": error.to_string(),
                    "paper_only": true
                });
                self.store
                    .record_audit("hermes_cycle_failed", summary.clone())
                    .await
                    .ok();
                return summary;
            }
        };

        let replay_refresh = match self.store.refresh_hermes_experiment_replays(5).await {
            Ok(summary) => summary,
            Err(error) => {
                tracing::warn!(%error, trigger, "hermes experiment replay refresh failed");
                json!({
                    "paper_only": true,
                    "refreshed_count": 0,
                    "skipped_count": 0,
                    "error": error.to_string()
                })
            }
        };
        let strategy = self.store.strategy_state().await.unwrap_or_else(|error| {
            tracing::warn!(%error, trigger, "hermes strategy state lookup failed");
            json!({"error": error.to_string()})
        });
        let ledger_summary = self.store.ledger_summary().await.ok();
        let promotion_gates = hermes_promotion_gates(&strategy, ledger_summary.as_ref());
        let proposed_experiment_count = strategy
            .get("experiments")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.get("status").and_then(Value::as_str) == Some("proposed"))
                    .count()
            })
            .unwrap_or_default();
        let summary = json!({
            "enabled": true,
            "trigger": trigger,
            "paper_only": true,
            "mode": "sanitized_reflection_and_one_variable_proposals",
            "reflection": reflection,
            "replay_refresh": replay_refresh,
            "strategy": {
                "active_baseline": strategy.get("active_baseline").cloned().unwrap_or(Value::Null),
                "experiment_count": strategy
                    .get("experiments")
                    .and_then(Value::as_array)
                    .map(|items| items.len())
                    .unwrap_or_default(),
                "proposed_experiment_count": proposed_experiment_count,
                "promotion_gates": promotion_gates
            },
            "ledger_summary": ledger_summary,
            "safety": {
                "browser_control": false,
                "credential_access": false,
                "real_money_placement": false,
                "requires_operator_review_for_strategy_changes": true
            }
        });
        self.store
            .record_audit("hermes_cycle_completed", summary.clone())
            .await
            .ok();
        summary
    }

    pub async fn hermes_state(&self) -> anyhow::Result<Value> {
        let reflections = self.store.hermes_reflections(25).await?;
        let strategy = self.store.strategy_state().await?;
        let ledger_summary = self.store.ledger_summary().await.ok();
        let promotion_gates = hermes_promotion_gates(&strategy, ledger_summary.as_ref());
        let latest_cycle = self
            .store
            .latest_audit_event("hermes_cycle_completed")
            .await?
            .map(compact_hermes_cycle_event);
        Ok(json!({
            "mode": "sanitized_reflection_and_one_variable_proposals",
            "summary": "Hermes is integrated as a safe loop participant: it refreshes paper-only reflections and replay evidence, reads one-variable experiment proposals, and cannot control browsers, credentials, or real-money placement.",
            "loop": {
                "enabled": self.settings.hermes_agent_enabled,
                "reflection_interval_seconds": self.settings.hermes_reflection_interval_seconds,
                "manual_trigger": "/api/hermes/run",
                "kubernetes_component": "hermes-agent"
            },
            "safety": {
                "browser_control": false,
                "credential_access": false,
                "real_money_placement": false,
                "strategy_changes_require_operator_review": true
            },
            "promotion_policy": {
                "min_settled_paper_positions": HERMES_MIN_SETTLED_FOR_PROMOTION,
                "requires_replay_evidence": true,
                "requires_no_open_or_awaiting_exposure": true,
                "requires_operator_reviewed_state": true,
                "paper_only": true
            },
            "reflections": reflections,
            "strategy": strategy,
            "ledger_summary": ledger_summary,
            "promotion_gates": promotion_gates,
            "latest_cycle": latest_cycle
        }))
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
                self.settings.settlement_lookup_cooldown_minutes,
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
        let daily_reflection = match self.store.record_daily_reflection(None).await {
            Ok(reflection) => {
                self.store
                    .record_audit("hermes_daily_reflection_auto_recorded", reflection.clone())
                    .await
                    .ok();
                reflection
            }
            Err(error) => {
                tracing::warn!(%error, snapshot_id = %snapshot_id, "daily reflection recording failed");
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
                    "daily_reflection_id": daily_reflection.get("id").cloned().unwrap_or(Value::Null),
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
            "daily_reflection": daily_reflection,
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
            .refresh_settlement_review_queue(
                self.settings.settlement_queue_limit,
                self.settings.settlement_lookup_cooldown_minutes,
            )
            .await
        {
            Ok(mut summary) => {
                let auto_external = match self
                    .store
                    .auto_settle_external_overdue(120, self.settings.settlement_queue_limit)
                    .await
                {
                    Ok(auto_summary) => auto_summary,
                    Err(error) => {
                        tracing::warn!(%error, "external settlement auto-check failed");
                        json!({
                            "enabled": true,
                            "settled_count": 0,
                            "error": error.to_string()
                        })
                    }
                };
                if let Some(summary_object) = summary.as_object_mut() {
                    summary_object.insert(
                        "auto_external_settlement".to_string(),
                        auto_external.clone(),
                    );
                }
                self.store
                    .record_audit("settlement_review_refreshed", summary.clone())
                    .await
                    .ok();
                if auto_external
                    .get("settled_count")
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
                    > 0
                {
                    self.store
                        .record_audit("external_settlement_auto_checked", auto_external)
                        .await
                        .ok();
                }
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

    pub async fn result_agent_queue(&self) -> Value {
        if !self.settings.settlement_queue_enabled {
            return json!({"enabled": false, "task_count": 0, "items": []});
        }

        let review = self.refresh_settlement_review_queue().await;
        let items = review
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let account_agent = danskespil_account_agent_status();
        let mut tasks: Vec<Value> = items
            .iter()
            .filter_map(|item| result_agent_task(item, &account_agent))
            .collect();
        tasks.sort_by(|left, right| {
            number(right, "priority_score")
                .partial_cmp(&number(left, "priority_score"))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let task_exposure = tasks
            .iter()
            .map(|task| number(task, "hypothetical_stake"))
            .sum::<f64>();
        let latest_cycle_event = self
            .store
            .latest_audit_event("result_agent_cycle_completed")
            .await
            .ok()
            .flatten();
        let latest_cycle = latest_cycle_event
            .clone()
            .map(compact_result_agent_cycle_event);
        let cycle_health = result_agent_cycle_health(
            latest_cycle_event.as_ref(),
            self.settings.result_agent_enabled,
            self.settings.result_agent_interval_seconds,
        );
        let recent_cycles = self
            .store
            .audit_events_by_type("result_agent_cycle_completed", 8)
            .await
            .ok()
            .and_then(|events| events.get("items").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .into_iter()
            .map(compact_result_agent_cycle_summary_event)
            .collect::<Vec<_>>();

        json!({
            "enabled": true,
            "paper_only": true,
            "agent": {
                "name": "result_agent",
                "mode": "read_only_result_discovery",
                "settles_real_bets": false,
                "stores_credentials": false,
                "credential_values_exposed": false
            },
            "danskespil_account_agent": account_agent,
            "latest_cycle": latest_cycle,
            "cycle_health": cycle_health,
            "recent_cycles": recent_cycles,
            "review_count": items.len(),
            "task_count": tasks.len(),
            "task_exposure": task_exposure,
            "items": tasks,
            "source_precedence": [
                "danskespil_account_history",
                "official_competition_results",
                "flashscore_results",
                "sofascore_results",
                "xscores_results",
                "livescore_results"
            ],
            "instructions": [
                "Use account/coupon history first when a local authenticated browser session is available.",
                "Post only sanitized result evidence to /api/settlement/external-evidence.",
                "Do not store cookies, credentials, browser storage, or full account pages in Postgres.",
                "Keep settlement paper-only; never submit a real bet."
            ]
        })
    }

    pub async fn account_history_requests(&self) -> Value {
        if !self.settings.settlement_queue_enabled {
            return json!({"enabled": false, "request_count": 0, "items": []});
        }

        let review = self.refresh_settlement_review_queue().await;
        let items = review
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let account_agent = danskespil_account_agent_status();
        let mut requests = items
            .iter()
            .filter_map(|item| account_history_request(item))
            .collect::<Vec<_>>();
        requests.sort_by(|left, right| {
            number(right, "priority_score")
                .partial_cmp(&number(left, "priority_score"))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let request_exposure = requests
            .iter()
            .map(|request| number(request, "hypothetical_stake"))
            .sum::<f64>();

        json!({
            "enabled": true,
            "paper_only": true,
            "agent": {
                "name": "danskespil_account_history_agent",
                "mode": "local_operator_browser_read_only",
                "runs_in_cluster": false,
                "settles_real_bets": false,
                "stores_credentials": false,
                "stores_cookies": false,
                "stores_browser_storage": false,
                "credential_values_exposed": false
            },
            "danskespil_account_agent": account_agent,
            "local_agent_runbook": {
                "script": "scripts/account_history_agent.py",
                "runs_in_cluster": false,
                "requires_local_agent_browser": true,
                "requires_port_forward": true,
                "port_forward_command": "rtk kubectl --context docker-desktop -n danske-spil port-forward svc/gambler-api 18083:8080",
                "dry_run_command": "rtk python3 scripts/account_history_agent.py --api http://127.0.0.1:18083 --dry-run",
                "settle_command": "rtk python3 scripts/account_history_agent.py --api http://127.0.0.1:18083 --settle",
                "make_dry_run_target": "rtk make account-history-agent-dry-run",
                "history_url_env": "DANSKESPIL_ACCOUNT_HISTORY_URL",
                "dry_run_first": true,
                "settle_requires_deterministic_bookmaker_truth": true
            },
            "review_count": items.len(),
            "request_count": requests.len(),
            "request_exposure": request_exposure,
            "items": requests,
            "evidence_endpoint": "/api/settlement/external-evidence",
            "allowed_evidence_fields": [
                "source_key",
                "bet_id",
                "coupon_simulation_id",
                "event_name",
                "event_names",
                "market_name",
                "outcome_name",
                "settlement_result",
                "result_status",
                "home_name",
                "away_name",
                "home_score",
                "away_score",
                "confidence",
                "raw_text_excerpt",
                "settle"
            ],
            "forbidden_payloads": [
                "credentials",
                "cookies",
                "browser_storage",
                "full_account_pages",
                "payment_data",
                "spil_id_identifiers",
                "mitid_payloads"
            ],
            "instructions": [
                "Use an operator-controlled local browser session only.",
                "Read account or coupon history without placing, editing, depositing, withdrawing, or submitting anything.",
                "Post only compact sanitized settlement facts to /api/settlement/external-evidence.",
                "Use source_key=danskespil_account_history for bookmaker-settlement evidence."
            ]
        })
    }

    pub async fn run_result_agent_once(&self) -> Value {
        if !self.settings.result_agent_enabled {
            return json!({
                "enabled": false,
                "reason": "GAMBLER_RESULT_AGENT_ENABLED=false",
                "attempted_count": 0,
                "settled_count": 0,
                "results": [],
                "skipped": []
            });
        }

        let queue = self.result_agent_queue().await;
        let tasks = queue
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let limit = self.settings.result_agent_per_cycle_limit.min(tasks.len());
        let queued_exposure = tasks
            .iter()
            .map(|task| number(task, "hypothetical_stake"))
            .sum::<f64>();
        let selected_tasks = tasks.into_iter().take(limit).collect::<Vec<_>>();
        let selected_exposure = selected_tasks
            .iter()
            .map(|task| number(task, "hypothetical_stake"))
            .sum::<f64>();
        let max_selected_priority = selected_tasks
            .iter()
            .map(|task| number(task, "priority_score"))
            .fold(0.0, f64::max);
        let http = match flashscore_http_client() {
            Ok(client) => client,
            Err(error) => {
                return json!({
                    "enabled": true,
                    "attempted_count": 0,
                    "task_attempted_count": 0,
                    "queued_task_count": queue.get("task_count").cloned().unwrap_or(Value::Null),
                    "queued_task_exposure": queued_exposure,
                    "selected_task_count": selected_tasks.len(),
                    "selected_task_exposure": selected_exposure,
                    "max_selected_priority": max_selected_priority,
                    "settled_count": 0,
                    "error": error.to_string(),
                    "results": [],
                    "skipped": []
                });
            }
        };
        let mut results = Vec::new();
        let mut skipped = Vec::new();
        let mut settled_count = 0usize;
        let mut task_attempted_count = 0usize;
        let mut task_attempted_exposure = 0.0;
        let mut task_skipped_exposure = 0.0;

        match self
            .store
            .auto_settle_external_overdue(120, self.settings.result_agent_per_cycle_limit)
            .await
        {
            Ok(result) => {
                settled_count += result
                    .get("settled_count")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as usize;
                results.push(json!({
                    "source": "configured_external_result_links",
                    "action": "auto_settle_overdue_from_configured_links",
                    "result": result
                }));
            }
            Err(error) => skipped.push(json!({
                "task_kind": "public_result_evidence_check",
                "reason": "configured_external_result_check_failed",
                "error": error.to_string()
            })),
        }

        for task in selected_tasks {
            let task_kind = text(&task, "task_kind").unwrap_or_default();
            let task_stake = number(&task, "hypothetical_stake");
            let task_priority = number(&task, "priority_score");
            if task_kind != "public_result_source_discovery" {
                task_skipped_exposure += task_stake;
                skipped.push(json!({
                    "task_kind": task_kind,
                    "reason": "handled_by_account_agent_or_configured_link_worker",
                    "hypothetical_stake": task_stake,
                    "priority_score": task_priority,
                    "selection": task.get("selection").cloned().unwrap_or(Value::Null)
                }));
                continue;
            }
            task_attempted_count += 1;
            task_attempted_exposure += task_stake;

            match flashscore_discover(&http, &task).await {
                Ok(FlashscoreDiscovery {
                    evidence: Some(evidence),
                    status_evidence: _,
                    diagnostics,
                }) => {
                    let source_link_payload = evidence.source_link_payload();
                    let source_link_result = match self
                        .store
                        .add_external_result_link(&source_link_payload)
                        .await
                    {
                        Ok(result) => result,
                        Err(error) => {
                            skipped.push(json!({
                                "task_kind": task_kind,
                                "reason": "source_link_persist_failed",
                                "event_name": evidence.event_name,
                                "source_url": evidence.source_url,
                                "hypothetical_stake": task_stake,
                                "priority_score": task_priority,
                                "error": error.to_string()
                            }));
                            continue;
                        }
                    };

                    let evidence_result = if evidence.finished {
                        match self
                            .store
                            .ingest_external_result_evidence(&evidence.evidence_payload(true))
                            .await
                        {
                            Ok(result) => {
                                settled_count += result
                                    .get("settled_count")
                                    .and_then(Value::as_u64)
                                    .unwrap_or_default()
                                    as usize;
                                result
                            }
                            Err(error) => json!({
                                "posted": false,
                                "error": error.to_string()
                            }),
                        }
                    } else {
                        json!({
                            "posted": false,
                            "reason": "flashscore_event_not_finished",
                            "stage": evidence.stage
                        })
                    };

                    let result = json!({
                        "source_key": FLASHSCORE_SOURCE_KEY,
                        "source_url": evidence.source_url,
                        "event_name": evidence.event_name,
                        "sport_key": evidence.sport_key,
                        "gender_scope": evidence.gender_scope,
                        "score": {"home": evidence.home_score, "away": evidence.away_score},
                        "finished": evidence.finished,
                        "match_score": evidence.match_score,
                        "diagnostics": diagnostics,
                        "source_link": source_link_result,
                        "evidence_result": evidence_result,
                        "hypothetical_stake": task_stake,
                        "priority_score": task_priority,
                        "paper_only": true
                    });
                    self.store
                        .record_audit("result_agent_flashscore_discovery", result.clone())
                        .await
                        .ok();
                    results.push(result);
                }
                Ok(FlashscoreDiscovery {
                    evidence: None,
                    status_evidence: Some(status_evidence),
                    diagnostics,
                }) => {
                    let source_link_payload = status_evidence.source_link_payload();
                    let source_link_result = match self
                        .store
                        .add_external_result_link(&source_link_payload)
                        .await
                    {
                        Ok(result) => result,
                        Err(error) => {
                            skipped.push(json!({
                                "task_kind": task_kind,
                                "reason": "source_link_persist_failed",
                                "event_name": status_evidence.event_name,
                                "source_url": status_evidence.source_url,
                                "hypothetical_stake": task_stake,
                                "priority_score": task_priority,
                                "error": error.to_string()
                            }));
                            continue;
                        }
                    };
                    let mut status_settled = Vec::new();
                    let mut status_skipped = Vec::new();
                    match self.store.simulated_bets(1000).await {
                        Ok(bets) => {
                            for bet in bets.into_iter().filter(|bet| {
                                bet.event_name.as_deref()
                                    == Some(status_evidence.event_name.as_str())
                                    && matches!(
                                        bet.status.as_str(),
                                        "awaiting_result" | "unresolved" | "postponed"
                                    )
                            }) {
                                match self
                                    .store
                                    .settle_simulated_bet(
                                        &bet.id,
                                        status_evidence.settlement_result,
                                        FLASHSCORE_SOURCE_KEY,
                                        status_evidence.confidence,
                                        &status_evidence.settlement_notes(),
                                    )
                                    .await
                                {
                                    Ok(item) => {
                                        settled_count += 1;
                                        status_settled.push(json!({
                                            "bet_id": item.id,
                                            "event_name": item.event_name,
                                            "outcome_name": item.outcome_name,
                                            "status": item.status,
                                            "observed_result": status_evidence.settlement_result
                                        }));
                                    }
                                    Err(error) => status_skipped.push(json!({
                                        "bet_id": bet.id,
                                        "event_name": bet.event_name,
                                        "outcome_name": bet.outcome_name,
                                        "reason": "status_settlement_failed",
                                        "error": error.to_string()
                                    })),
                                }
                            }
                        }
                        Err(error) => status_skipped.push(json!({
                            "reason": "simulated_bets_lookup_failed",
                            "error": error.to_string()
                        })),
                    }
                    let status_settled_count = status_settled.len();
                    let result = json!({
                        "source_key": FLASHSCORE_SOURCE_KEY,
                        "source_url": status_evidence.source_url,
                        "event_name": status_evidence.event_name,
                        "sport_key": status_evidence.sport_key,
                        "gender_scope": status_evidence.gender_scope,
                        "stage": status_evidence.stage,
                        "status_result": status_evidence.settlement_result,
                        "diagnostics": diagnostics,
                        "source_link": source_link_result,
                        "settled": status_settled,
                        "settled_count": status_settled_count,
                        "skipped": status_skipped,
                        "hypothetical_stake": task_stake,
                        "priority_score": task_priority,
                        "paper_only": true
                    });
                    self.store
                        .record_audit("result_agent_flashscore_status_discovery", result.clone())
                        .await
                        .ok();
                    results.push(result);
                }
                Ok(FlashscoreDiscovery {
                    evidence: None,
                    status_evidence: None,
                    diagnostics,
                }) => {
                    let skipped_item = json!({
                        "task_kind": task_kind,
                        "reason": "flashscore_discovery_no_match",
                        "hypothetical_stake": task_stake,
                        "priority_score": task_priority,
                        "selection": task.get("selection").cloned().unwrap_or(Value::Null),
                        "diagnostics": diagnostics
                    });
                    self.store
                        .record_audit("result_agent_flashscore_no_match", skipped_item.clone())
                        .await
                        .ok();
                    skipped.push(skipped_item);
                }
                Err(error) => skipped.push(json!({
                    "task_kind": task_kind,
                    "reason": "flashscore_discovery_failed",
                    "hypothetical_stake": task_stake,
                    "priority_score": task_priority,
                    "selection": task.get("selection").cloned().unwrap_or(Value::Null),
                    "error": error.to_string()
                })),
            }
        }

        let summary = json!({
            "enabled": true,
            "paper_only": true,
            "agent": "rust_flashscore_result_agent",
            "cycle_limit": self.settings.result_agent_per_cycle_limit,
            "attempted_count": results.len(),
            "task_attempted_count": task_attempted_count,
            "queued_task_count": queue.get("task_count").cloned().unwrap_or(Value::Null),
            "queued_task_exposure": queued_exposure,
            "selected_task_count": limit,
            "selected_task_exposure": selected_exposure,
            "task_attempted_exposure": task_attempted_exposure,
            "task_skipped_exposure": task_skipped_exposure,
            "max_selected_priority": max_selected_priority,
            "settled_count": settled_count,
            "skipped_count": skipped.len(),
            "results": results,
            "skipped": skipped
        });
        self.store
            .record_audit("result_agent_cycle_completed", summary.clone())
            .await
            .ok();
        summary
    }

    pub async fn performance_report(&self) -> Value {
        match self
            .store
            .performance_report(
                self.settings.default_stake,
                self.settings.auto_paper_per_scan_limit,
                self.settings.auto_paper_max_open_exposure,
                self.settings.settlement_lookup_cooldown_minutes,
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

fn result_agent_task(item: &Value, account_agent: &Value) -> Option<Value> {
    let recommendation = item
        .get("recommendation")
        .and_then(Value::as_str)
        .unwrap_or("await_more_evidence");
    if !matches!(
        recommendation,
        "external_result_required"
            | "expected_finish_passed_recheck"
            | "manual_grade_ready"
            | "manual_void_or_refund_review"
    ) {
        return None;
    }

    let item_type = item
        .get("item_type")
        .and_then(Value::as_str)
        .unwrap_or("single");
    let source_links = result_agent_source_links(item);
    let account_available = account_agent
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let has_links = !source_links.is_empty();
    let task_kind = match (item_type, recommendation, account_available, has_links) {
        (_, "manual_void_or_refund_review", true, _) => "account_history_void_or_refund_check",
        (_, "manual_void_or_refund_review", false, _) => "public_source_void_or_refund_check",
        ("coupon", _, true, _) => "account_history_coupon_result_check",
        ("coupon", _, false, true) => "public_coupon_leg_result_check",
        ("coupon", _, false, false) => "public_coupon_leg_source_discovery",
        (_, _, true, _) => "account_history_result_check",
        (_, _, false, true) => "public_result_evidence_check",
        _ => "public_result_source_discovery",
    };
    let automation_status = match (account_available, has_links) {
        (true, _) => "account_browser_ready",
        (false, true) => "public_browser_or_direct_source_ready",
        (false, false) => "needs_automated_source_discovery",
    };
    let agent_action = match task_kind {
        "account_history_result_check" | "account_history_coupon_result_check" => {
            "open read-only Danske Spil account/coupon history and extract sanitized settlement truth"
        }
        "account_history_void_or_refund_check" => {
            "open read-only Danske Spil account history and check cancellation/refund outcome"
        }
        "public_result_evidence_check" | "public_coupon_leg_result_check" => {
            "collect final-score evidence from configured public result links"
        }
        "public_source_void_or_refund_check" => {
            "check official/public result sources for cancellation, postponement, void, or refund truth"
        }
        _ => "discover a stable official or public match-result page without operator prompts",
    };

    let event_names = result_agent_event_names(item);
    let search_terms = result_agent_search_terms(item, &event_names);
    let hypothetical_stake = number(item, "hypothetical_stake");
    let overdue_minutes = item
        .get("overdue_minutes")
        .and_then(Value::as_i64)
        .unwrap_or_default()
        .max(0) as f64;
    let priority_score = hypothetical_stake * (1.0 + overdue_minutes / 120.0);
    let legs = if item_type == "coupon" {
        item.get("legs").cloned().unwrap_or_else(|| json!([]))
    } else {
        json!([])
    };

    Some(json!({
        "item_type": item_type,
        "task_kind": task_kind,
        "automation_status": automation_status,
        "agent_action": agent_action,
        "recommendation": recommendation,
        "hypothetical_stake": hypothetical_stake,
        "priority_score": priority_score,
        "overdue_minutes": item.get("overdue_minutes").cloned().unwrap_or(Value::Null),
        "expected_result_check_after": item.get("expected_result_check_after").cloned().unwrap_or(Value::Null),
        "last_lookup_at": item.get("last_lookup_at").cloned().unwrap_or(Value::Null),
        "lookup_stale": item.get("lookup_stale").cloned().unwrap_or(Value::Bool(true)),
        "ids": {
            "bet_id": item.get("bet_id").cloned().unwrap_or(Value::Null),
            "coupon_simulation_id": item.get("coupon_simulation_id").cloned().unwrap_or(Value::Null),
            "candidate_id": item.get("candidate_id").cloned().unwrap_or(Value::Null),
            "event_id": item.get("event_id").cloned().unwrap_or(Value::Null)
        },
        "selection": {
            "event_name": item.get("event_name").cloned().unwrap_or(Value::Null),
            "event_names": event_names,
            "sport_key": item.get("sport_key").cloned().unwrap_or(Value::Null),
            "competition": item.get("competition").cloned().unwrap_or(Value::Null),
            "market_name": item.get("market_name").cloned().unwrap_or(Value::Null),
            "market_kind": item.get("market_kind").cloned().unwrap_or(Value::Null),
            "outcome_name": item.get("outcome_name").cloned().unwrap_or(Value::Null),
            "legs": legs
        },
        "event_state": {
            "event_status": item.get("event_status").cloned().unwrap_or(Value::Null),
            "event_resulted": item.get("event_resulted").cloned().unwrap_or(Value::Null),
            "event_settled": item.get("event_settled").cloned().unwrap_or(Value::Null)
        },
        "source_links": source_links,
        "search_terms": search_terms,
        "candidate_sources": [
            "danskespil_account_history",
            "official_competition_results",
            "flashscore_results",
            "sofascore_results",
            "xscores_results",
            "livescore_results"
        ],
        "evidence_endpoint": "/api/settlement/external-evidence",
        "source_link_endpoint": "/api/settlement/source-link"
    }))
}

fn account_history_request(item: &Value) -> Option<Value> {
    let recommendation = item
        .get("recommendation")
        .and_then(Value::as_str)
        .unwrap_or("await_more_evidence");
    if !matches!(
        recommendation,
        "external_result_required"
            | "expected_finish_passed_recheck"
            | "manual_grade_ready"
            | "manual_void_or_refund_review"
    ) {
        return None;
    }

    let item_type = item
        .get("item_type")
        .and_then(Value::as_str)
        .unwrap_or("single");
    let request_kind = match (item_type, recommendation) {
        (_, "manual_void_or_refund_review") => "account_history_void_or_refund_check",
        ("coupon", _) => "account_history_coupon_result_check",
        _ => "account_history_result_check",
    };
    let event_names = result_agent_event_names(item);
    let legs = if item_type == "coupon" {
        item.get("legs").cloned().unwrap_or_else(|| json!([]))
    } else {
        json!([])
    };
    let hypothetical_stake = number(item, "hypothetical_stake");
    let overdue_minutes = item
        .get("overdue_minutes")
        .and_then(Value::as_i64)
        .unwrap_or_default()
        .max(0) as f64;
    let priority_score = hypothetical_stake * (1.0 + overdue_minutes / 120.0);
    let selection = json!({
        "event_name": item.get("event_name").cloned().unwrap_or(Value::Null),
        "event_names": event_names,
        "sport_key": item.get("sport_key").cloned().unwrap_or(Value::Null),
        "competition": item.get("competition").cloned().unwrap_or(Value::Null),
        "market_name": item.get("market_name").cloned().unwrap_or(Value::Null),
        "market_kind": item.get("market_kind").cloned().unwrap_or(Value::Null),
        "outcome_name": item.get("outcome_name").cloned().unwrap_or(Value::Null),
        "legs": legs
    });
    let expected_truth = match recommendation {
        "manual_void_or_refund_review" => {
            "bookmaker cancellation, refund, void, push, postponement, or abandoned-state truth"
        }
        "manual_grade_ready" => "bookmaker settled won/lost/void/refund status",
        _ => "bookmaker final settlement status or final-score evidence",
    };

    Some(json!({
        "item_type": item_type,
        "request_kind": request_kind,
        "recommendation": recommendation,
        "expected_truth": expected_truth,
        "hypothetical_stake": hypothetical_stake,
        "priority_score": priority_score,
        "overdue_minutes": item.get("overdue_minutes").cloned().unwrap_or(Value::Null),
        "expected_result_check_after": item.get("expected_result_check_after").cloned().unwrap_or(Value::Null),
        "last_lookup_at": item.get("last_lookup_at").cloned().unwrap_or(Value::Null),
        "lookup_stale": item.get("lookup_stale").cloned().unwrap_or(Value::Bool(true)),
        "ids": {
            "bet_id": item.get("bet_id").cloned().unwrap_or(Value::Null),
            "coupon_simulation_id": item.get("coupon_simulation_id").cloned().unwrap_or(Value::Null),
            "candidate_id": item.get("candidate_id").cloned().unwrap_or(Value::Null),
            "event_id": item.get("event_id").cloned().unwrap_or(Value::Null)
        },
        "selection": selection,
        "event_state": {
            "event_status": item.get("event_status").cloned().unwrap_or(Value::Null),
            "event_resulted": item.get("event_resulted").cloned().unwrap_or(Value::Null),
            "event_settled": item.get("event_settled").cloned().unwrap_or(Value::Null)
        },
        "source_key": "danskespil_account_history",
        "evidence_endpoint": "/api/settlement/external-evidence",
        "evidence_template": {
            "source_key": "danskespil_account_history",
            "bet_id": item.get("bet_id").cloned().unwrap_or(Value::Null),
            "coupon_simulation_id": item.get("coupon_simulation_id").cloned().unwrap_or(Value::Null),
            "event_name": item.get("event_name").cloned().unwrap_or(Value::Null),
            "event_names": selection.get("event_names").cloned().unwrap_or(Value::Null),
            "sport_key": item.get("sport_key").cloned().unwrap_or(Value::Null),
            "market_name": item.get("market_name").cloned().unwrap_or(Value::Null),
            "outcome_name": item.get("outcome_name").cloned().unwrap_or(Value::Null),
            "settle": false,
            "paper_only": true
        },
        "safety": {
            "read_only_browser": true,
            "store_credentials": false,
            "store_cookies": false,
            "store_full_account_payload": false,
            "submit_bets": false,
            "deposit_or_withdraw": false
        }
    }))
}

fn danskespil_account_agent_status() -> Value {
    let username_present = env_present("DANSKESPIL_USERNAME") || env_present("DANSKESPIL_EMAIL");
    let password_present = env_present("DANSKESPIL_PASSWORD");
    let login_url_present = env_present("DANSKESPIL_LOGIN_URL");
    json!({
        "available": username_present && password_present,
        "username_or_email_present": username_present,
        "password_present": password_present,
        "login_url_present": login_url_present,
        "session_mode": "local_operator_browser",
        "credential_values_exposed": false,
        "safe_use": "read_only_account_history_result_evidence"
    })
}

fn env_present(name: &str) -> bool {
    std::env::var(name)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn result_agent_source_links(item: &Value) -> Vec<Value> {
    item.get("external_result_links")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|link| {
            link.get("source_url")
                .and_then(Value::as_str)
                .is_some_and(|url| !url.trim().is_empty())
        })
        .collect()
}

fn result_agent_event_names(item: &Value) -> Vec<String> {
    if item.get("item_type").and_then(Value::as_str) == Some("coupon") {
        return item
            .get("legs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|leg| leg.get("event_name").and_then(Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect();
    }
    item.get("event_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| vec![value.to_owned()])
        .unwrap_or_default()
}

fn result_agent_search_terms(item: &Value, event_names: &[String]) -> Vec<String> {
    let sport = item
        .get("sport_key")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let competition = item
        .get("competition")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut terms = Vec::new();
    for event_name in event_names {
        terms.push(event_name.to_string());
        if !competition.is_empty() {
            terms.push(format!("{event_name} {competition}"));
            terms.push(format!("{competition} {event_name} final result"));
        }
        if !sport.is_empty() {
            terms.push(format!("{event_name} {sport} result"));
        }
        terms.push(format!("{event_name} final score"));
    }
    terms.sort();
    terms.dedup();
    terms
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

#[derive(Clone, Debug)]
struct FlashscoreParticipant {
    id: String,
    title: String,
    url: String,
    participant_type_id: Option<i64>,
}

#[derive(Clone, Debug)]
struct FlashscoreEvidence {
    source_url: String,
    sport_key: String,
    gender_scope: Option<String>,
    event_name: String,
    home_name: String,
    away_name: String,
    home_score: i32,
    away_score: i32,
    event_id: String,
    stage: String,
    finished: bool,
    match_score: f64,
    home_aliases: Vec<String>,
    away_aliases: Vec<String>,
    raw_text_excerpt: String,
}

#[derive(Clone, Debug)]
struct FlashscoreDiscovery {
    evidence: Option<FlashscoreEvidence>,
    status_evidence: Option<FlashscoreStatusEvidence>,
    diagnostics: Value,
}

#[derive(Clone, Debug)]
struct FlashscoreStatusEvidence {
    source_url: String,
    sport_key: String,
    gender_scope: Option<String>,
    event_name: String,
    home_name: String,
    away_name: String,
    event_id: String,
    stage: String,
    settlement_result: &'static str,
    confidence: f64,
    home_aliases: Vec<String>,
    away_aliases: Vec<String>,
    raw_text_excerpt: String,
}

impl FlashscoreEvidence {
    fn source_link_payload(&self) -> Value {
        json!({
            "source_key": FLASHSCORE_SOURCE_KEY,
            "source_url": self.source_url,
            "sport_key": self.sport_key,
            "gender_scope": self.gender_scope,
            "event_name": self.event_name,
            "home_aliases": self.home_aliases,
            "away_aliases": self.away_aliases,
            "requires_browser_automation": false,
            "notes": {
                "agent_discovered": true,
                "agent": "rust_flashscore_result_agent",
                "method": "flashscore_participant_feed",
                "event_id": self.event_id,
                "stage": self.stage,
                "paper_only": true
            }
        })
    }

    fn evidence_payload(&self, settle: bool) -> Value {
        json!({
            "source_key": FLASHSCORE_SOURCE_KEY,
            "source_url": self.source_url,
            "source_title": format!("{} - {} {}:{}", self.home_name, self.away_name, self.home_score, self.away_score),
            "event_name": self.event_name,
            "sport_key": self.sport_key,
            "gender_scope": self.gender_scope,
            "home_name": self.home_name,
            "away_name": self.away_name,
            "home_aliases": self.home_aliases,
            "away_aliases": self.away_aliases,
            "home_score": self.home_score,
            "away_score": self.away_score,
            "confidence": 0.82,
            "settle": settle,
            "browser_automation": {
                "tool": "rust_reqwest",
                "source": "flashscore_participant_feed",
                "event_id": self.event_id
            },
            "raw_text_excerpt": self.raw_text_excerpt,
            "paper_only": true
        })
    }
}

impl FlashscoreStatusEvidence {
    fn source_link_payload(&self) -> Value {
        json!({
            "source_key": FLASHSCORE_SOURCE_KEY,
            "source_url": self.source_url,
            "sport_key": self.sport_key,
            "gender_scope": self.gender_scope,
            "event_name": self.event_name,
            "home_aliases": self.home_aliases,
            "away_aliases": self.away_aliases,
            "requires_browser_automation": false,
            "notes": {
                "agent_discovered": true,
                "agent": "rust_flashscore_result_agent",
                "method": "flashscore_participant_feed_status",
                "event_id": self.event_id,
                "stage": self.stage,
                "settlement_result": self.settlement_result,
                "paper_only": true
            }
        })
    }

    fn settlement_notes(&self) -> String {
        json!({
            "mode": "flashscore_status_settlement",
            "source_key": FLASHSCORE_SOURCE_KEY,
            "source_url": self.source_url,
            "event_name": self.event_name,
            "home_name": self.home_name,
            "away_name": self.away_name,
            "event_id": self.event_id,
            "stage": self.stage,
            "observed_result": self.settlement_result,
            "raw_text_excerpt": self.raw_text_excerpt,
            "paper_only": true
        })
        .to_string()
    }
}

fn flashscore_http_client() -> anyhow::Result<HttpClient> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,application/json;q=0.8,*/*;q=0.7",
        ),
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("da-DK,da;q=0.9,en-US;q=0.8,en;q=0.7"),
    );
    Ok(HttpClient::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
        .default_headers(headers)
        .timeout(StdDuration::from_secs(20))
        .build()?)
}

async fn flashscore_discover(
    http: &HttpClient,
    task: &Value,
) -> anyhow::Result<FlashscoreDiscovery> {
    let selection = task.get("selection").unwrap_or(&Value::Null);
    let sport_key = text(selection, "sport_key")
        .unwrap_or_default()
        .to_lowercase();
    let mut diagnostics = serde_json::Map::new();
    diagnostics.insert("source_key".to_string(), json!(FLASHSCORE_SOURCE_KEY));
    diagnostics.insert("sport_key".to_string(), json!(sport_key));
    diagnostics.insert("selection".to_string(), selection.clone());
    if flashscore_sport_id(&sport_key).is_none() {
        diagnostics.insert("reason".to_string(), json!("unsupported_flashscore_sport"));
        return Ok(FlashscoreDiscovery {
            evidence: None,
            status_evidence: None,
            diagnostics: Value::Object(diagnostics),
        });
    }
    let Some(event_name) = text(selection, "event_name").map(str::trim) else {
        diagnostics.insert("reason".to_string(), json!("missing_event_name"));
        return Ok(FlashscoreDiscovery {
            evidence: None,
            status_evidence: None,
            diagnostics: Value::Object(diagnostics),
        });
    };
    diagnostics.insert("event_name".to_string(), json!(event_name));
    let Some((home_name, away_name)) = split_event_name(event_name) else {
        diagnostics.insert("reason".to_string(), json!("unsupported_event_name_shape"));
        return Ok(FlashscoreDiscovery {
            evidence: None,
            status_evidence: None,
            diagnostics: Value::Object(diagnostics),
        });
    };
    diagnostics.insert("home_name".to_string(), json!(home_name));
    diagnostics.insert("away_name".to_string(), json!(away_name));
    let gender_scope = infer_selection_gender_scope(selection);
    diagnostics.insert("gender_scope".to_string(), json!(gender_scope));
    let expected_check_after = text(task, "expected_result_check_after")
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));
    diagnostics.insert(
        "expected_result_check_after".to_string(),
        json!(expected_check_after),
    );
    if sport_key == "tennis" && is_doubles_event(&home_name, &away_name) {
        return flashscore_discover_tennis_doubles(
            http,
            event_name,
            &home_name,
            &away_name,
            gender_scope,
            expected_check_after,
            diagnostics,
        )
        .await;
    }
    let home_candidates =
        ranked_flashscore_participants(http, &home_name, &sport_key, gender_scope.as_deref())
            .await?;
    diagnostics.insert(
        "home_participant_candidates".to_string(),
        flashscore_participant_candidates_json(&home_candidates),
    );
    let Some((_, home)) = home_candidates.first().cloned() else {
        diagnostics.insert("reason".to_string(), json!("home_participant_not_found"));
        return Ok(FlashscoreDiscovery {
            evidence: None,
            status_evidence: None,
            diagnostics: Value::Object(diagnostics),
        });
    };
    let away_candidates =
        ranked_flashscore_participants(http, &away_name, &sport_key, gender_scope.as_deref())
            .await?;
    diagnostics.insert(
        "away_participant_candidates".to_string(),
        flashscore_participant_candidates_json(&away_candidates),
    );
    let Some((_, away)) = away_candidates.first().cloned() else {
        diagnostics.insert("reason".to_string(), json!("away_participant_not_found"));
        return Ok(FlashscoreDiscovery {
            evidence: None,
            status_evidence: None,
            diagnostics: Value::Object(diagnostics),
        });
    };
    diagnostics.insert(
        "selected_participants".to_string(),
        json!({
            "home": flashscore_participant_json(None, &home),
            "away": flashscore_participant_json(None, &away)
        }),
    );
    let mut best: Option<(f64, bool, Value, String)> = None;
    let mut feed_diagnostics = Vec::new();
    for feed_participant in [&home, &away] {
        let Some(feed_sign) =
            fetch_flashscore_feed_sign(http, feed_participant, &sport_key).await?
        else {
            feed_diagnostics.push(json!({
                "participant": flashscore_participant_json(None, feed_participant),
                "reason": "feed_sign_not_available"
            }));
            continue;
        };
        let feed_name = format!("pe_2_2_{}_x", feed_participant.id);
        let feed_url = format!("{FLASHSCORE_BASE_URL}/x/feed/{feed_name}");
        let feed = http
            .get(feed_url)
            .header("x-fsign", feed_sign)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let rows = parse_flashscore_feed_rows(&feed);
        let rows_seen = rows.len();
        let mut best_in_feed: Option<(f64, bool, Value)> = None;

        for row in rows {
            let (score, reversed_side) = score_flashscore_row(
                &row,
                &home_name,
                &away_name,
                &home.id,
                &away.id,
                expected_check_after,
            );
            if best_in_feed
                .as_ref()
                .map(|(current, _, _)| score > *current)
                .unwrap_or(true)
            {
                best_in_feed = Some((score, reversed_side, row.clone()));
            }
            if score >= 90.0
                && best
                    .as_ref()
                    .map(|(current, _, _, _)| score > *current)
                    .unwrap_or(true)
            {
                best = Some((score, reversed_side, row, feed_name.clone()));
            }
        }
        feed_diagnostics.push(json!({
            "feed_name": feed_name,
            "participant": flashscore_participant_json(None, feed_participant),
            "rows_seen": rows_seen,
            "best_row": best_in_feed.as_ref().map(|(score, reversed_side, row)| {
                json!({
                    "score": score,
                    "reversed_side": reversed_side,
                    "row": flashscore_row_preview(row)
                })
            })
        }));
    }
    diagnostics.insert("feeds_checked".to_string(), json!(feed_diagnostics));
    let Some((match_score, reversed_side, row, feed_name)) = best else {
        diagnostics.insert(
            "reason".to_string(),
            json!("no_feed_row_above_match_threshold"),
        );
        diagnostics.insert("match_threshold".to_string(), json!(90.0));
        return Ok(FlashscoreDiscovery {
            evidence: None,
            status_evidence: None,
            diagnostics: Value::Object(diagnostics),
        });
    };
    let mut feed_home = row_text(&row, "AE")
        .or_else(|| row_text(&row, "FH"))
        .or_else(|| row_text(&row, "WM"))
        .unwrap_or_else(|| home_name.clone());
    let mut feed_away = row_text(&row, "AF")
        .or_else(|| row_text(&row, "FK"))
        .or_else(|| row_text(&row, "WN"))
        .unwrap_or_else(|| away_name.clone());
    if unresolved_flashscore_name(&feed_home) {
        feed_home = strip_country_suffix(&home.title);
    }
    if unresolved_flashscore_name(&feed_away) {
        feed_away = strip_country_suffix(&away.title);
    }
    let mut home_score = row_text(&row, "AG")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or_default();
    let mut away_score = row_text(&row, "AH")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or_default();
    if reversed_side {
        std::mem::swap(&mut feed_home, &mut feed_away);
        std::mem::swap(&mut home_score, &mut away_score);
    }
    let event_id = row_text(&row, "AA").unwrap_or_default();
    let stage = row_text(&row, "AC").unwrap_or_default();
    let source_url = flashscore_match_url(&sport_key, &event_id, &home, &away);
    let home_aliases = flashscore_aliases(&home_name, &feed_home, &home.title, &sport_key);
    let away_aliases = flashscore_aliases(&away_name, &feed_away, &away.title, &sport_key);
    let evidence = FlashscoreEvidence {
        source_url,
        sport_key,
        gender_scope,
        event_name: event_name.to_string(),
        home_name: feed_home.clone(),
        away_name: feed_away.clone(),
        home_score,
        away_score,
        event_id: event_id.clone(),
        stage: stage.clone(),
        finished: FLASHSCORE_FINISHED_STAGES.contains(&stage.as_str()),
        match_score,
        home_aliases,
        away_aliases,
        raw_text_excerpt: format!(
            "Flashscore feed {feed_name} matched {event_id}; stage={stage}; score={home_score}:{away_score}"
        ),
    };
    diagnostics.insert("reason".to_string(), json!("match_found"));
    diagnostics.insert("matched_feed_name".to_string(), json!(feed_name));
    diagnostics.insert("matched_row".to_string(), flashscore_row_preview(&row));
    Ok(FlashscoreDiscovery {
        evidence: Some(evidence),
        status_evidence: None,
        diagnostics: Value::Object(diagnostics),
    })
}

async fn flashscore_discover_tennis_doubles(
    http: &HttpClient,
    event_name: &str,
    home_name: &str,
    away_name: &str,
    gender_scope: Option<String>,
    expected_check_after: Option<DateTime<Utc>>,
    mut diagnostics: serde_json::Map<String, Value>,
) -> anyhow::Result<FlashscoreDiscovery> {
    let sport_key = "tennis".to_string();
    diagnostics.insert("match_shape".to_string(), json!("tennis_doubles"));
    let Some(home_players) = split_doubles_side(home_name) else {
        diagnostics.insert("reason".to_string(), json!("home_doubles_side_not_split"));
        return Ok(FlashscoreDiscovery {
            evidence: None,
            status_evidence: None,
            diagnostics: Value::Object(diagnostics),
        });
    };
    let Some(away_players) = split_doubles_side(away_name) else {
        diagnostics.insert("reason".to_string(), json!("away_doubles_side_not_split"));
        return Ok(FlashscoreDiscovery {
            evidence: None,
            status_evidence: None,
            diagnostics: Value::Object(diagnostics),
        });
    };

    let home_candidates =
        ranked_flashscore_doubles_players(http, &home_players, gender_scope.as_deref()).await?;
    diagnostics.insert(
        "home_player_candidates".to_string(),
        flashscore_doubles_candidates_json(&home_players, &home_candidates),
    );
    if home_candidates.iter().any(Vec::is_empty) {
        diagnostics.insert("reason".to_string(), json!("home_doubles_player_not_found"));
        return Ok(FlashscoreDiscovery {
            evidence: None,
            status_evidence: None,
            diagnostics: Value::Object(diagnostics),
        });
    }

    let away_candidates =
        ranked_flashscore_doubles_players(http, &away_players, gender_scope.as_deref()).await?;
    diagnostics.insert(
        "away_player_candidates".to_string(),
        flashscore_doubles_candidates_json(&away_players, &away_candidates),
    );
    if away_candidates.iter().any(Vec::is_empty) {
        diagnostics.insert("reason".to_string(), json!("away_doubles_player_not_found"));
        return Ok(FlashscoreDiscovery {
            evidence: None,
            status_evidence: None,
            diagnostics: Value::Object(diagnostics),
        });
    }

    let home_groups = candidate_id_groups(&home_candidates);
    let away_groups = candidate_id_groups(&away_candidates);
    let mut feed_participants = Vec::new();
    let mut seen_feed_ids = HashSet::new();
    for (_, participant) in home_candidates
        .iter()
        .chain(away_candidates.iter())
        .flat_map(|side| side.iter())
    {
        if seen_feed_ids.insert(participant.id.clone()) {
            feed_participants.push(participant.clone());
        }
    }

    diagnostics.insert(
        "selected_feed_participants".to_string(),
        Value::Array(
            feed_participants
                .iter()
                .map(|participant| flashscore_participant_json(None, participant))
                .collect(),
        ),
    );

    let mut best: Option<(f64, bool, Value, String)> = None;
    let mut feed_diagnostics = Vec::new();
    for feed_participant in &feed_participants {
        let Some(feed_sign) =
            fetch_flashscore_feed_sign(http, feed_participant, &sport_key).await?
        else {
            feed_diagnostics.push(json!({
                "participant": flashscore_participant_json(None, feed_participant),
                "reason": "feed_sign_not_available"
            }));
            continue;
        };
        let feed_name = format!("pe_2_2_{}_x", feed_participant.id);
        let feed_url = format!("{FLASHSCORE_BASE_URL}/x/feed/{feed_name}");
        let feed = http
            .get(feed_url)
            .header("x-fsign", feed_sign)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let rows = parse_flashscore_feed_rows(&feed);
        let rows_seen = rows.len();
        let mut best_in_feed: Option<(f64, bool, Value)> = None;
        for row in rows {
            let (score, reversed_side) = score_flashscore_doubles_row(
                &row,
                &home_groups,
                &away_groups,
                expected_check_after,
            );
            if best_in_feed
                .as_ref()
                .map(|(current, _, _)| score > *current)
                .unwrap_or(true)
            {
                best_in_feed = Some((score, reversed_side, row.clone()));
            }
            if score >= 90.0
                && best
                    .as_ref()
                    .map(|(current, _, _, _)| score > *current)
                    .unwrap_or(true)
            {
                best = Some((score, reversed_side, row, feed_name.clone()));
            }
        }
        feed_diagnostics.push(json!({
            "feed_name": feed_name,
            "participant": flashscore_participant_json(None, feed_participant),
            "rows_seen": rows_seen,
            "best_row": best_in_feed.as_ref().map(|(score, reversed_side, row)| {
                json!({
                    "score": score,
                    "reversed_side": reversed_side,
                    "row": flashscore_row_preview(row)
                })
            })
        }));
    }
    diagnostics.insert("feeds_checked".to_string(), json!(feed_diagnostics));

    let Some((match_score, reversed_side, row, feed_name)) = best else {
        diagnostics.insert(
            "reason".to_string(),
            json!("no_doubles_feed_row_above_match_threshold"),
        );
        diagnostics.insert("match_threshold".to_string(), json!(90.0));
        return Ok(FlashscoreDiscovery {
            evidence: None,
            status_evidence: None,
            diagnostics: Value::Object(diagnostics),
        });
    };

    let event_id = row_text(&row, "AA").unwrap_or_default();
    let stage = row_text(&row, "AC").unwrap_or_default();
    let (matched_home_ids, matched_away_ids) = oriented_doubles_side_ids(&row, reversed_side);
    let source_home = first_matching_participant(&home_candidates, &matched_home_ids)
        .or_else(|| first_candidate(&home_candidates))
        .cloned();
    let source_away = first_matching_participant(&away_candidates, &matched_away_ids)
        .or_else(|| first_candidate(&away_candidates))
        .cloned();
    let source_url = match (source_home.as_ref(), source_away.as_ref()) {
        (Some(home), Some(away)) => flashscore_match_url(&sport_key, &event_id, home, away),
        _ => format!("{FLASHSCORE_BASE_URL}/match/tennis/?mid={event_id}"),
    };
    let home_aliases = doubles_side_aliases(
        home_name,
        &home_players,
        &home_candidates,
        &matched_home_ids,
    );
    let away_aliases = doubles_side_aliases(
        away_name,
        &away_players,
        &away_candidates,
        &matched_away_ids,
    );

    if let (Some(mut home_score), Some(mut away_score)) = (
        row_text(&row, "AG").and_then(|value| value.parse::<i32>().ok()),
        row_text(&row, "AH").and_then(|value| value.parse::<i32>().ok()),
    ) {
        if reversed_side {
            std::mem::swap(&mut home_score, &mut away_score);
        }
        let evidence = FlashscoreEvidence {
            source_url,
            sport_key,
            gender_scope,
            event_name: event_name.to_string(),
            home_name: home_name.to_string(),
            away_name: away_name.to_string(),
            home_score,
            away_score,
            event_id: event_id.clone(),
            stage: stage.clone(),
            finished: FLASHSCORE_FINISHED_STAGES.contains(&stage.as_str()),
            match_score,
            home_aliases,
            away_aliases,
            raw_text_excerpt: format!(
                "Flashscore doubles feed {feed_name} matched {event_id}; stage={stage}; score={home_score}:{away_score}"
            ),
        };
        diagnostics.insert("reason".to_string(), json!("doubles_match_found"));
        diagnostics.insert("matched_feed_name".to_string(), json!(feed_name));
        diagnostics.insert("matched_row".to_string(), flashscore_row_preview(&row));
        return Ok(FlashscoreDiscovery {
            evidence: Some(evidence),
            status_evidence: None,
            diagnostics: Value::Object(diagnostics),
        });
    }

    if let Some(settlement_result) = flashscore_status_result(&row) {
        let status_evidence = FlashscoreStatusEvidence {
            source_url,
            sport_key,
            gender_scope,
            event_name: event_name.to_string(),
            home_name: home_name.to_string(),
            away_name: away_name.to_string(),
            event_id: event_id.clone(),
            stage: stage.clone(),
            settlement_result,
            confidence: 0.82,
            home_aliases,
            away_aliases,
            raw_text_excerpt: format!(
                "Flashscore doubles feed {feed_name} matched {event_id}; stage={stage}; status={settlement_result}; no_score=true"
            ),
        };
        diagnostics.insert("reason".to_string(), json!("doubles_status_match_found"));
        diagnostics.insert("matched_feed_name".to_string(), json!(feed_name));
        diagnostics.insert("matched_row".to_string(), flashscore_row_preview(&row));
        return Ok(FlashscoreDiscovery {
            evidence: None,
            status_evidence: Some(status_evidence),
            diagnostics: Value::Object(diagnostics),
        });
    }

    diagnostics.insert(
        "reason".to_string(),
        json!("doubles_match_found_without_score_or_status"),
    );
    diagnostics.insert("matched_feed_name".to_string(), json!(feed_name));
    diagnostics.insert("matched_row".to_string(), flashscore_row_preview(&row));
    Ok(FlashscoreDiscovery {
        evidence: None,
        status_evidence: None,
        diagnostics: Value::Object(diagnostics),
    })
}

async fn ranked_flashscore_doubles_players(
    http: &HttpClient,
    players: &[String],
    gender_scope: Option<&str>,
) -> anyhow::Result<Vec<Vec<(f64, FlashscoreParticipant)>>> {
    let mut sides = Vec::new();
    for player in players {
        let candidates = ranked_flashscore_participants(http, player, "tennis", gender_scope)
            .await?
            .into_iter()
            .take(4)
            .collect();
        sides.push(candidates);
    }
    Ok(sides)
}

async fn ranked_flashscore_participants(
    http: &HttpClient,
    name: &str,
    sport_key: &str,
    gender_scope: Option<&str>,
) -> anyhow::Result<Vec<(f64, FlashscoreParticipant)>> {
    let queries = flashscore_name_variants(name, sport_key);
    let mut candidates = Vec::new();
    for query in queries {
        for participant in flashscore_search_participants(http, &query, sport_key).await? {
            let score =
                flashscore_participant_score(name, &query, &participant, sport_key, gender_scope);
            if score >= 0.5 {
                candidates.push((score, participant));
            }
        }
    }
    candidates.sort_by(|(left_score, _), (right_score, _)| {
        right_score
            .partial_cmp(left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(candidates)
}

async fn flashscore_search_participants(
    http: &HttpClient,
    name: &str,
    sport_key: &str,
) -> anyhow::Result<Vec<FlashscoreParticipant>> {
    let Some(sport_id) = flashscore_sport_id(sport_key) else {
        return Ok(Vec::new());
    };
    let url = Url::parse_with_params(
        FLASHSCORE_SEARCH_URL,
        &[
            ("q", name),
            ("l", "1"),
            ("s", &sport_id.to_string()),
            ("f", "1;1;1"),
            ("pid", "2"),
            ("sid", "1"),
        ],
    )?;
    let response = http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let data = parse_jsonp(&response)?;
    let items = data
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| text(item, "type") == Some("participants"))
        .filter(|item| json_i64(item.get("sport_id")) == Some(sport_id))
        .filter_map(|item| {
            Some(FlashscoreParticipant {
                id: text(item, "id")?.to_string(),
                title: text(item, "title")?.to_string(),
                url: text(item, "url")?.to_string(),
                participant_type_id: json_i64(item.get("participant_type_id")),
            })
        })
        .collect();
    Ok(items)
}

async fn fetch_flashscore_feed_sign(
    http: &HttpClient,
    participant: &FlashscoreParticipant,
    sport_key: &str,
) -> anyhow::Result<Option<String>> {
    let prefix = if sport_key == "tennis" {
        "player"
    } else {
        "team"
    };
    let url = format!(
        "{FLASHSCORE_BASE_URL}/{prefix}/{}/{}/",
        participant.url, participant.id
    );
    let page = http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(Some(
        extract_between(&page, r#""feed_sign":""#, r#"""#)
            .map(str::to_string)
            .unwrap_or_else(|| FLASHSCORE_DEFAULT_XFSIGN.to_string()),
    ))
}

fn parse_jsonp(value: &str) -> anyhow::Result<Value> {
    let start = value
        .find('{')
        .ok_or_else(|| anyhow::anyhow!("Flashscore search response had no JSON body"))?;
    let end = value
        .rfind('}')
        .ok_or_else(|| anyhow::anyhow!("Flashscore search response had no JSON body"))?;
    Ok(serde_json::from_str(&value[start..=end])?)
}

fn parse_flashscore_feed_rows(feed: &str) -> Vec<Value> {
    feed.split('~')
        .filter_map(|raw_row| {
            let mut object = serde_json::Map::new();
            for cell in raw_row.split('¬') {
                let Some((key, value)) = cell.split_once('÷') else {
                    continue;
                };
                if key.is_empty() {
                    continue;
                }
                object.insert(key.to_string(), Value::String(htmlish_unescape(value)));
            }
            if object.is_empty() {
                None
            } else {
                Some(Value::Object(object))
            }
        })
        .collect()
}

fn flashscore_participant_candidates_json(candidates: &[(f64, FlashscoreParticipant)]) -> Value {
    Value::Array(
        candidates
            .iter()
            .take(5)
            .map(|(score, participant)| flashscore_participant_json(Some(*score), participant))
            .collect(),
    )
}

fn flashscore_doubles_candidates_json(
    players: &[String],
    candidates: &[Vec<(f64, FlashscoreParticipant)>],
) -> Value {
    Value::Array(
        players
            .iter()
            .zip(candidates.iter())
            .map(|(player, player_candidates)| {
                json!({
                    "player": player,
                    "candidates": flashscore_participant_candidates_json(player_candidates)
                })
            })
            .collect(),
    )
}

fn flashscore_participant_json(score: Option<f64>, participant: &FlashscoreParticipant) -> Value {
    json!({
        "id": participant.id,
        "title": participant.title,
        "url": participant.url,
        "participant_type_id": participant.participant_type_id,
        "score": score
    })
}

fn flashscore_row_preview(row: &Value) -> Value {
    let mut preview = serde_json::Map::new();
    for key in ["AA", "AD", "AC", "AE", "AF", "AG", "AH", "PX", "PY", "AW"] {
        if let Some(value) = row_text(row, key) {
            preview.insert(key.to_string(), Value::String(value));
        }
    }
    Value::Object(preview)
}

fn is_doubles_event(home_name: &str, away_name: &str) -> bool {
    split_doubles_side(home_name).is_some() && split_doubles_side(away_name).is_some()
}

fn split_doubles_side(side: &str) -> Option<Vec<String>> {
    let players = side
        .split('/')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    (players.len() == 2).then_some(players)
}

fn candidate_id_groups(candidates: &[Vec<(f64, FlashscoreParticipant)>]) -> Vec<HashSet<String>> {
    candidates
        .iter()
        .map(|player_candidates| {
            player_candidates
                .iter()
                .map(|(_, participant)| participant.id.clone())
                .collect()
        })
        .collect()
}

fn side_ids_match(side_ids: &HashSet<String>, candidate_groups: &[HashSet<String>]) -> bool {
    !candidate_groups.is_empty()
        && candidate_groups
            .iter()
            .all(|group| group.iter().any(|id| side_ids.contains(id)))
}

fn score_flashscore_doubles_row(
    row: &Value,
    home_groups: &[HashSet<String>],
    away_groups: &[HashSet<String>],
    expected_check_after: Option<DateTime<Utc>>,
) -> (f64, bool) {
    if row_text(row, "AA").is_none() {
        return (0.0, false);
    }
    let home_ids = participant_ids(row_text(row, "PX").as_deref());
    let away_ids = participant_ids(row_text(row, "PY").as_deref());
    let normal = side_ids_match(&home_ids, home_groups) && side_ids_match(&away_ids, away_groups);
    let reversed_side =
        side_ids_match(&home_ids, away_groups) && side_ids_match(&away_ids, home_groups);
    let mut score = if normal || reversed_side { 100.0 } else { 0.0 };
    if score == 0.0 {
        return (0.0, false);
    }
    if let (Some(expected), Some(start_epoch)) = (
        expected_check_after,
        row_text(row, "AD").and_then(|value| value.parse::<i64>().ok()),
    ) {
        if let Some(start) = DateTime::<Utc>::from_timestamp(start_epoch, 0) {
            let hours = (expected - start).num_seconds().unsigned_abs() as f64 / 3600.0;
            score += 24.0 - hours.min(24.0);
        }
    }
    (score, reversed_side)
}

fn flashscore_status_result(row: &Value) -> Option<&'static str> {
    let has_home_score = row_text(row, "AG")
        .and_then(|value| value.parse::<i32>().ok())
        .is_some();
    let has_away_score = row_text(row, "AH")
        .and_then(|value| value.parse::<i32>().ok())
        .is_some();
    if row_text(row, "AC").as_deref() == Some("9")
        && row_text(row, "AW").as_deref() == Some("1")
        && !has_home_score
        && !has_away_score
    {
        return Some("refunded");
    }
    None
}

fn first_candidate(
    candidates: &[Vec<(f64, FlashscoreParticipant)>],
) -> Option<&FlashscoreParticipant> {
    candidates
        .iter()
        .flat_map(|player_candidates| player_candidates.iter())
        .map(|(_, participant)| participant)
        .next()
}

fn first_matching_participant<'a>(
    candidates: &'a [Vec<(f64, FlashscoreParticipant)>],
    ids: &HashSet<String>,
) -> Option<&'a FlashscoreParticipant> {
    candidates
        .iter()
        .flat_map(|player_candidates| player_candidates.iter())
        .map(|(_, participant)| participant)
        .find(|participant| ids.contains(&participant.id))
}

fn oriented_doubles_side_ids(
    row: &Value,
    reversed_side: bool,
) -> (HashSet<String>, HashSet<String>) {
    let feed_home_ids = participant_ids(row_text(row, "PX").as_deref());
    let feed_away_ids = participant_ids(row_text(row, "PY").as_deref());
    if reversed_side {
        (feed_away_ids, feed_home_ids)
    } else {
        (feed_home_ids, feed_away_ids)
    }
}

fn doubles_side_aliases(
    requested_side: &str,
    players: &[String],
    candidates: &[Vec<(f64, FlashscoreParticipant)>],
    matched_ids: &HashSet<String>,
) -> Vec<String> {
    let mut aliases = vec![
        requested_side.to_string(),
        players.join(" / "),
        players.join("/"),
    ];
    aliases.extend(players.iter().cloned());
    for player_candidates in candidates {
        let matched = player_candidates
            .iter()
            .find(|(_, participant)| matched_ids.contains(&participant.id))
            .or_else(|| player_candidates.first());
        if let Some((_, participant)) = matched {
            aliases.push(strip_country_suffix(&participant.title));
        }
    }
    dedup_texts(aliases)
}

fn score_flashscore_row(
    row: &Value,
    home_name: &str,
    away_name: &str,
    home_id: &str,
    away_id: &str,
    expected_check_after: Option<DateTime<Utc>>,
) -> (f64, bool) {
    if row_text(row, "AA").is_none()
        || row_text(row, "AG").is_none()
        || row_text(row, "AH").is_none()
    {
        return (0.0, false);
    }
    let home_ids = participant_ids(row_text(row, "PX").as_deref());
    let away_ids = participant_ids(row_text(row, "PY").as_deref());
    let normal = home_ids.contains(home_id) && away_ids.contains(away_id);
    let reversed_side = home_ids.contains(away_id) && away_ids.contains(home_id);
    let mut score = if normal || reversed_side { 100.0 } else { 0.0 };
    let mut feed_home = row_text(row, "AE")
        .or_else(|| row_text(row, "FH"))
        .or_else(|| row_text(row, "WM"))
        .unwrap_or_default();
    let mut feed_away = row_text(row, "AF")
        .or_else(|| row_text(row, "FK"))
        .or_else(|| row_text(row, "WN"))
        .unwrap_or_default();
    if reversed_side {
        std::mem::swap(&mut feed_home, &mut feed_away);
    }
    score += 25.0 * token_score(home_name, &feed_home);
    score += 25.0 * token_score(away_name, &feed_away);
    if let (Some(expected), Some(start_epoch)) = (
        expected_check_after,
        row_text(row, "AD").and_then(|value| value.parse::<i64>().ok()),
    ) {
        if let Some(start) = DateTime::<Utc>::from_timestamp(start_epoch, 0) {
            let hours = (expected - start).num_seconds().unsigned_abs() as f64 / 3600.0;
            score += 24.0 - hours.min(24.0);
        }
    }
    (score, reversed_side)
}

fn flashscore_match_url(
    sport_key: &str,
    event_id: &str,
    home: &FlashscoreParticipant,
    away: &FlashscoreParticipant,
) -> String {
    let sport_path = match sport_key {
        "soccer" => "football",
        "football" | "tennis" | "basketball" => sport_key,
        _ => "sport",
    };
    format!(
        "{FLASHSCORE_BASE_URL}/match/{sport_path}/{}-{}/{}-{}/?mid={event_id}",
        home.url, home.id, away.url, away.id
    )
}

fn flashscore_sport_id(sport_key: &str) -> Option<i64> {
    match sport_key {
        "football" | "soccer" => Some(1),
        "tennis" => Some(2),
        "basketball" => Some(3),
        _ => None,
    }
}

fn split_event_name(event_name: &str) -> Option<(String, String)> {
    [" - ", " vs ", " v "].iter().find_map(|separator| {
        event_name
            .split_once(separator)
            .map(|(home, away)| (home.trim().to_string(), away.trim().to_string()))
    })
}

fn flashscore_name_variants(name: &str, sport_key: &str) -> Vec<String> {
    let trimmed = name.trim();
    let mut variants = vec![trimmed.to_string(), strip_country_suffix(trimmed)];
    if let Some(country) = localized_country_alias(trimmed) {
        variants.push(country.to_string());
    }
    variants.extend(flashscore_known_name_aliases(trimmed));

    let normalized = normalize_token_text(trimmed);
    if sport_key == "tennis" {
        let tokens = normalized.split_whitespace().collect::<Vec<_>>();
        if tokens.len() >= 2 {
            let last = tokens.last().copied().unwrap_or_default();
            let first = tokens[..tokens.len() - 1].join(" ");
            variants.push(format!("{last} {first}"));
        }
    } else {
        variants.extend(team_name_variants(trimmed));
    }

    dedup_texts(variants)
}

fn flashscore_aliases(
    requested_name: &str,
    feed_name: &str,
    flashscore_title: &str,
    sport_key: &str,
) -> Vec<String> {
    let mut aliases = vec![
        requested_name.to_string(),
        feed_name.to_string(),
        strip_country_suffix(flashscore_title),
    ];
    aliases.extend(flashscore_name_variants(requested_name, sport_key));
    aliases.extend(flashscore_known_name_aliases(requested_name));
    dedup_texts(aliases)
}

fn flashscore_participant_score(
    requested_name: &str,
    query: &str,
    participant: &FlashscoreParticipant,
    sport_key: &str,
    gender_scope: Option<&str>,
) -> f64 {
    let title = strip_country_suffix(&participant.title);
    let mut score = token_score(query, &title).max(token_score(requested_name, &title));
    let title_gender = flashscore_title_gender(&participant.title);
    match (gender_scope, title_gender.as_deref()) {
        (Some("women"), Some("women")) => score += 0.25,
        (Some("women"), Some("men")) => score *= 0.35,
        (Some("men"), Some("women")) => score *= 0.2,
        (None, Some("women")) if !selection_name_has_women_marker(requested_name) => score *= 0.2,
        _ => {}
    }
    if participant.participant_type_id == Some(2) && !sport_uses_individual_participants(sport_key)
    {
        score *= 0.35;
    }
    score
}

fn sport_uses_individual_participants(sport_key: &str) -> bool {
    matches!(sport_key, "tennis" | "motorsports" | "golf" | "cycling")
}

fn flashscore_title_gender(title: &str) -> Option<String> {
    let normalized = format!(" {} ", normalize_token_text(title));
    if normalized.contains(" w ")
        || normalized.contains(" women ")
        || normalized.contains(" womens ")
        || normalized.contains(" female ")
    {
        return Some("women".to_string());
    }
    if normalized.contains(" men ")
        || normalized.contains(" mens ")
        || normalized.contains(" male ")
    {
        return Some("men".to_string());
    }
    None
}

fn selection_name_has_women_marker(name: &str) -> bool {
    let normalized = format!(" {} ", normalize_token_text(name));
    normalized.contains(" w ")
        || normalized.contains(" women ")
        || normalized.contains(" womens ")
        || normalized.contains(" dame ")
        || normalized.contains(" damer ")
        || normalized.contains(" kvinde ")
        || normalized.contains(" kvinder ")
        || normalized.contains(" k ")
}

fn localized_country_alias(name: &str) -> Option<&'static str> {
    let normalized = normalize_token_text(name);
    match normalized.as_str() {
        "bosnien hercegovina" | "bosnien herzogovina" | "bosnien herzgovina" => {
            Some("Bosnia and Herzegovina")
        }
        "danmark" => Some("Denmark"),
        "england" => Some("England"),
        "finland" => Some("Finland"),
        "frankrig" => Some("France"),
        "graekenland" => Some("Greece"),
        "holland" | "nederlandene" => Some("Netherlands"),
        "hviderusland" => Some("Belarus"),
        "indien" => Some("India"),
        "irak" => Some("Iraq"),
        "irland" => Some("Ireland"),
        "island" => Some("Iceland"),
        "italien" => Some("Italy"),
        "japan" => Some("Japan"),
        "kina" => Some("China"),
        "kroatien" => Some("Croatia"),
        "norge" => Some("Norway"),
        "nordmakedonien" => Some("North Macedonia"),
        "polen" => Some("Poland"),
        "portugal" => Some("Portugal"),
        "schweiz" => Some("Switzerland"),
        "serbien" => Some("Serbia"),
        "spanien" => Some("Spain"),
        "storbritannien" => Some("Great Britain"),
        "sverige" => Some("Sweden"),
        "sydafrika" => Some("South Africa"),
        "sydkorea" => Some("South Korea"),
        "tjekkiet" => Some("Czech Republic"),
        "tyrkiet" => Some("Turkey"),
        "tyskland" => Some("Germany"),
        "ungarn" => Some("Hungary"),
        "usa" => Some("USA"),
        "oestrig" => Some("Austria"),
        _ => None,
    }
}

fn flashscore_known_name_aliases(name: &str) -> Vec<String> {
    match normalize_token_text(name).as_str() {
        "derthona basket" | "derthona" => {
            vec!["Tortona".to_string(), "Derthona Tortona".to_string()]
        }
        "reyer venezia" | "umana reyer venezia" => vec!["Venezia".to_string()],
        "brescia leonessa" => vec!["Brescia".to_string()],
        "trieste 2004" | "pallacanestro trieste 2004" => vec!["Trieste".to_string()],
        "team fog naestved" | "team fog naestved basketball" => {
            vec!["Naestved".to_string(), "Team FOG Naestved".to_string()]
        }
        "bakken bears" => vec!["Bakken Bears".to_string()],
        "cd maristas palencia" | "maristas palencia" => vec!["Palencia".to_string()],
        "cb fuenlabrada" => vec!["Fuenlabrada".to_string()],
        "nsa" => vec!["NSA".to_string()],
        "club antonin sportif" | "club antonine sportif" => vec![
            "Antonine".to_string(),
            "Antonin".to_string(),
            "Club Antonine".to_string(),
        ],
        "rinascita basket rimini" | "dole rimini" => {
            vec!["Rimini".to_string(), "Basket Rimini".to_string()]
        }
        "ueb cividale" | "ueb gesteco cividale" | "cividale del friuli" => {
            vec!["Cividale".to_string(), "UEB Cividale".to_string()]
        }
        "baskonia vitoria gasteiz" | "baskonia vitoria gasteiz sad" | "saski baskonia" => {
            vec!["Baskonia".to_string(), "Saski Baskonia".to_string()]
        }
        "cb malaga" | "cb malaga 2002" | "unicaja malaga" => {
            vec!["Malaga".to_string(), "Unicaja".to_string()]
        }
        "naestved bk" | "naestved if" => vec!["Naestved".to_string()],
        "ab" | "akademisk boldklub" => vec!["AB Copenhagen".to_string(), "AB".to_string()],
        "paris sg" | "paris saint germain" | "paris saint germain fc" | "psg" => vec![
            "PSG".to_string(),
            "Paris Saint-Germain".to_string(),
            "Paris SG".to_string(),
        ],
        "america de cali sa k" | "america de cali w" | "america de cali women" => {
            vec![
                "America de Cali W".to_string(),
                "America de Cali".to_string(),
            ]
        }
        "internacional de palmira w"
        | "internacional de palmira k"
        | "internacional de palmira women"
        | "inter palmira w"
        | "inter palmira k" => vec![
            "Inter Palmira W".to_string(),
            "Inter Palmira".to_string(),
            "Internacional de Palmira W".to_string(),
            "Internacional de Palmira".to_string(),
        ],
        "cucuta deportivo fc" | "cucuta deportivo" => {
            vec!["Cucuta".to_string(), "Cucuta Deportivo".to_string()]
        }
        "fortaleza c e i f fc" | "fortaleza c e i f" | "fortaleza ceif fc" | "fortaleza ceif" => {
            vec![
                "Fortaleza".to_string(),
                "Fortaleza C.E.I.F.".to_string(),
                "Fortaleza FC".to_string(),
            ]
        }
        "cr vasco da gama w" | "cr vasco da gama k" | "vasco da gama w" | "vasco da gama k" => {
            vec![
                "Vasco W".to_string(),
                "Vasco da Gama W".to_string(),
                "Vasco da Gama".to_string(),
                "CR Vasco da Gama".to_string(),
            ]
        }
        "america mg k" | "america mg w" | "america mineiro w" | "america mineiro k" => vec![
            "America Mineiro W".to_string(),
            "America Mineiro".to_string(),
            "America MG".to_string(),
        ],
        "kolding if k" | "kolding if kvinder" | "kolding if women" => {
            vec!["KoldingQ".to_string(), "Kolding IF W".to_string()]
        }
        "fortuna hjorring k" | "fortuna hjorring kvinder" | "fortuna hjorring women" => {
            vec!["Fortuna Hjorring W".to_string()]
        }
        "scu torreense" => vec!["Torreense".to_string()],
        "casa pia ac" => vec!["Casa Pia".to_string()],
        _ => Vec::new(),
    }
}

fn team_name_variants(name: &str) -> Vec<String> {
    let mut variants = Vec::new();
    let normalized = name.trim();
    for suffix in [" FC", " SC", " AC", " IF", " BK", " KK"] {
        if let Some(stripped) = normalized.strip_suffix(suffix) {
            variants.push(stripped.trim().to_string());
        }
    }
    for prefix in ["FC ", "SC ", "AC ", "IF ", "BK ", "KK "] {
        if let Some(stripped) = normalized.strip_prefix(prefix) {
            variants.push(stripped.trim().to_string());
        }
    }
    variants
}

fn infer_selection_gender_scope(selection: &Value) -> Option<String> {
    let text = ["competition", "market_name", "event_name", "outcome_name"]
        .iter()
        .filter_map(|key| text(selection, key))
        .collect::<Vec<_>>()
        .join(" ");
    let normalized = format!(" {} ", normalize_token_text(&text));
    let women_markers = [
        "women",
        "womens",
        "female",
        "dame",
        "damer",
        "damesingle",
        "kvinde",
        "kvinder",
        "wta",
        "k",
    ];
    let men_markers = [
        "men",
        "mens",
        "male",
        "herre",
        "herrer",
        "herresingle",
        "atp",
        "m",
    ];
    if women_markers
        .iter()
        .any(|marker| normalized.contains(&format!(" {marker} ")))
    {
        return Some("women".to_string());
    }
    if men_markers
        .iter()
        .any(|marker| normalized.contains(&format!(" {marker} ")))
    {
        return Some("men".to_string());
    }
    None
}

fn token_score(query: &str, candidate: &str) -> f64 {
    let query_tokens = token_set(query);
    let candidate_tokens = token_set(candidate);
    if query_tokens.is_empty() || candidate_tokens.is_empty() {
        return 0.0;
    }
    let overlap = query_tokens.intersection(&candidate_tokens).count();
    overlap as f64 / query_tokens.len().max(1) as f64
}

fn token_set(value: &str) -> HashSet<String> {
    let stop = ["a", "ac", "bk", "fc", "if", "kk", "team", "the"];
    normalize_token_text(value)
        .split_whitespace()
        .filter(|token| token.len() > 1 && !stop.contains(token))
        .map(str::to_string)
        .collect()
}

fn normalize_token_text(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars().flat_map(|ch| match ch {
        'Æ' | 'æ' => "ae".chars().collect::<Vec<_>>(),
        'Ø' | 'ø' => "o".chars().collect::<Vec<_>>(),
        'Å' | 'å' | 'Ä' | 'ä' | 'Á' | 'á' | 'À' | 'à' | 'Â' | 'â' => {
            vec!['a']
        }
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

fn participant_ids(value: Option<&str>) -> HashSet<String> {
    value
        .unwrap_or_default()
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn row_text(row: &Value, key: &str) -> Option<String> {
    text(row, key).map(str::to_string)
}

fn json_i64(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64).or_else(|| {
        value
            .and_then(Value::as_str)
            .and_then(|value| value.parse().ok())
    })
}

fn strip_country_suffix(value: &str) -> String {
    value
        .rsplit_once(" (")
        .map(|(prefix, suffix)| {
            if suffix.ends_with(')') {
                prefix.trim().to_string()
            } else {
                value.trim().to_string()
            }
        })
        .unwrap_or_else(|| value.trim().to_string())
}

fn unresolved_flashscore_name(value: &str) -> bool {
    value.is_empty() || value.contains('{') || value.contains('}')
}

fn extract_between<'a>(value: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let start = value.find(prefix)? + prefix.len();
    let rest = &value[start..];
    let end = rest.find(suffix)?;
    Some(&rest[..end])
}

fn htmlish_unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn dedup_texts(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.to_ascii_lowercase()))
        .collect()
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn number(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or_default()
}

fn boolish(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(false)
}

fn hermes_promotion_gates(strategy: &Value, ledger_summary: Option<&LedgerSummary>) -> Vec<Value> {
    let settled_count = ledger_summary
        .map(|summary| summary.settled_count)
        .unwrap_or_default();
    let open_count = ledger_summary
        .map(|summary| summary.open_count)
        .unwrap_or_default();
    let Some(experiments) = strategy.get("experiments").and_then(Value::as_array) else {
        return Vec::new();
    };

    experiments
        .iter()
        .filter(|experiment| {
            !matches!(
                experiment.get("status").and_then(Value::as_str),
                Some("promoted" | "rejected" | "rolled_back")
            )
        })
        .map(|experiment| {
            let status = experiment
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let variable_name = experiment
                .get("variable_name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let replay_evidence_present = experiment
                .get("decision_payload")
                .and_then(|payload| payload.get("replay_evidence"))
                .map(|value| !value.is_null())
                .unwrap_or(false);
            let paper_only = experiment
                .get("evidence")
                .and_then(|evidence| evidence.get("safety"))
                .and_then(|safety| safety.get("paper_only"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let one_variable = !variable_name.is_empty()
                && experiment
                    .get("baseline_value")
                    .zip(experiment.get("proposed_value"))
                    .map(|(baseline, proposed)| baseline != proposed)
                    .unwrap_or(false);

            let mut blockers = Vec::new();
            if status != "active_simulation" {
                blockers.push("experiment_not_active_simulation");
            }
            if !one_variable {
                blockers.push("not_one_variable_change");
            }
            if !replay_evidence_present {
                blockers.push("missing_replay_evidence");
            }
            if settled_count < HERMES_MIN_SETTLED_FOR_PROMOTION {
                blockers.push("insufficient_settled_paper_positions");
            }
            if open_count > 0 {
                blockers.push("open_or_awaiting_paper_exposure");
            }
            if !paper_only {
                blockers.push("paper_only_safety_not_confirmed");
            }

            let eligible_for_promotion = blockers.is_empty();
            let recommendation = if eligible_for_promotion {
                "Eligible for operator promotion review; still paper-only and never enables real-money placement."
            } else {
                "Keep observing or replaying. Do not promote until all blockers are cleared."
            };
            json!({
                "experiment_id": experiment.get("id").cloned().unwrap_or(Value::Null),
                "title": experiment.get("title").cloned().unwrap_or(Value::Null),
                "status": status,
                "variable_name": variable_name,
                "eligible_for_promotion": eligible_for_promotion,
                "blockers": blockers,
                "policy": {
                    "min_settled_paper_positions": HERMES_MIN_SETTLED_FOR_PROMOTION,
                    "settled_paper_positions": settled_count,
                    "open_or_awaiting_paper_positions": open_count,
                    "replay_evidence_present": replay_evidence_present,
                    "one_variable_change": one_variable,
                    "paper_only": paper_only,
                    "requires_status": "active_simulation"
                },
                "recommendation": recommendation
            })
        })
        .collect()
}

fn compact_hermes_cycle_event(event: Value) -> Value {
    let details = event.get("details").cloned().unwrap_or_else(|| json!({}));
    let reflection = details
        .get("reflection")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let replay_refresh = details
        .get("replay_refresh")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let refreshed = replay_refresh
        .get("refreshed")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    let replay = item
                        .get("replay_evidence")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    json!({
                        "experiment_id": item.get("experiment_id").cloned().unwrap_or(Value::Null),
                        "title": item.get("title").cloned().unwrap_or(Value::Null),
                        "status": item.get("status").cloned().unwrap_or(Value::Null),
                        "variable_name": replay.get("variable_name").cloned().unwrap_or(Value::Null),
                        "candidate_count": replay.get("candidate_count").cloned().unwrap_or(Value::Null),
                        "delta": replay.get("delta").cloned().unwrap_or(Value::Null),
                        "replayed_at": replay.get("replayed_at").cloned().unwrap_or(Value::Null)
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let skipped = replay_refresh
        .get("skipped")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    json!({
        "id": event.get("id").cloned().unwrap_or(Value::Null),
        "created_at": event.get("created_at").cloned().unwrap_or(Value::Null),
        "event_type": event.get("event_type").cloned().unwrap_or(Value::Null),
        "details": {
            "enabled": details.get("enabled").cloned().unwrap_or(Value::Null),
            "trigger": details.get("trigger").cloned().unwrap_or(Value::Null),
            "paper_only": details.get("paper_only").cloned().unwrap_or(Value::Null),
            "mode": details.get("mode").cloned().unwrap_or(Value::Null),
            "reflection": {
                "id": reflection.get("id").cloned().unwrap_or(Value::Null),
                "title": reflection.get("title").cloned().unwrap_or(Value::Null),
                "status": reflection.get("status").cloned().unwrap_or(Value::Null),
                "created_at": reflection.get("created_at").cloned().unwrap_or(Value::Null),
                "summary": reflection.get("summary").cloned().unwrap_or(Value::Null)
            },
            "replay_refresh": {
                "paper_only": replay_refresh.get("paper_only").cloned().unwrap_or(Value::Null),
                "does_not_place_paper_bets": replay_refresh.get("does_not_place_paper_bets").cloned().unwrap_or(Value::Null),
                "does_not_change_experiment_status": replay_refresh.get("does_not_change_experiment_status").cloned().unwrap_or(Value::Null),
                "requested_limit": replay_refresh.get("requested_limit").cloned().unwrap_or(Value::Null),
                "refreshed_count": replay_refresh.get("refreshed_count").cloned().unwrap_or(Value::Null),
                "skipped_count": replay_refresh.get("skipped_count").cloned().unwrap_or(Value::Null),
                "refreshed": refreshed,
                "skipped": skipped,
                "error": replay_refresh.get("error").cloned().unwrap_or(Value::Null)
            },
            "strategy": details.get("strategy").cloned().unwrap_or(Value::Null),
            "ledger_summary": details.get("ledger_summary").cloned().unwrap_or(Value::Null),
            "safety": details.get("safety").cloned().unwrap_or(Value::Null)
        }
    })
}

fn compact_result_agent_cycle_event(event: Value) -> Value {
    let details = event.get("details").cloned().unwrap_or_else(|| json!({}));
    json!({
        "id": event.get("id").cloned().unwrap_or(Value::Null),
        "created_at": event.get("created_at").cloned().unwrap_or(Value::Null),
        "event_type": event.get("event_type").cloned().unwrap_or(Value::Null),
        "details": {
            "enabled": details.get("enabled").cloned().unwrap_or(Value::Null),
            "paper_only": details.get("paper_only").cloned().unwrap_or(Value::Null),
            "agent": details.get("agent").cloned().unwrap_or(Value::Null),
            "cycle_limit": details.get("cycle_limit").cloned().unwrap_or(Value::Null),
            "queued_task_count": details.get("queued_task_count").cloned().unwrap_or(Value::Null),
            "queued_task_exposure": details.get("queued_task_exposure").cloned().unwrap_or(Value::Null),
            "selected_task_count": details.get("selected_task_count").cloned().unwrap_or(Value::Null),
            "selected_task_exposure": details.get("selected_task_exposure").cloned().unwrap_or(Value::Null),
            "task_attempted_count": details.get("task_attempted_count").cloned().unwrap_or(Value::Null),
            "task_attempted_exposure": details.get("task_attempted_exposure").cloned().unwrap_or(Value::Null),
            "task_skipped_exposure": details.get("task_skipped_exposure").cloned().unwrap_or(Value::Null),
            "max_selected_priority": details.get("max_selected_priority").cloned().unwrap_or(Value::Null),
            "attempted_count": details.get("attempted_count").cloned().unwrap_or(Value::Null),
            "settled_count": details.get("settled_count").cloned().unwrap_or(Value::Null),
            "skipped_count": details.get("skipped_count").cloned().unwrap_or(Value::Null),
            "results": details.get("results").cloned().unwrap_or_else(|| json!([])),
            "skipped": details.get("skipped").cloned().unwrap_or_else(|| json!([]))
        }
    })
}

fn compact_result_agent_cycle_summary_event(event: Value) -> Value {
    let details = event.get("details").cloned().unwrap_or_else(|| json!({}));
    json!({
        "id": event.get("id").cloned().unwrap_or(Value::Null),
        "created_at": event.get("created_at").cloned().unwrap_or(Value::Null),
        "event_type": event.get("event_type").cloned().unwrap_or(Value::Null),
        "details": {
            "enabled": details.get("enabled").cloned().unwrap_or(Value::Null),
            "paper_only": details.get("paper_only").cloned().unwrap_or(Value::Null),
            "agent": details.get("agent").cloned().unwrap_or(Value::Null),
            "cycle_limit": details.get("cycle_limit").cloned().unwrap_or(Value::Null),
            "queued_task_count": details.get("queued_task_count").cloned().unwrap_or(Value::Null),
            "queued_task_exposure": details.get("queued_task_exposure").cloned().unwrap_or(Value::Null),
            "selected_task_count": details.get("selected_task_count").cloned().unwrap_or(Value::Null),
            "selected_task_exposure": details.get("selected_task_exposure").cloned().unwrap_or(Value::Null),
            "task_attempted_count": details.get("task_attempted_count").cloned().unwrap_or(Value::Null),
            "task_attempted_exposure": details.get("task_attempted_exposure").cloned().unwrap_or(Value::Null),
            "task_skipped_exposure": details.get("task_skipped_exposure").cloned().unwrap_or(Value::Null),
            "max_selected_priority": details.get("max_selected_priority").cloned().unwrap_or(Value::Null),
            "attempted_count": details.get("attempted_count").cloned().unwrap_or(Value::Null),
            "settled_count": details.get("settled_count").cloned().unwrap_or(Value::Null),
            "skipped_count": details.get("skipped_count").cloned().unwrap_or(Value::Null)
        }
    })
}

fn result_agent_cycle_health(
    latest_cycle: Option<&Value>,
    enabled: bool,
    interval_seconds: u64,
) -> Value {
    if !enabled {
        return json!({
            "enabled": false,
            "status": "disabled",
            "healthy": true,
            "interval_seconds": interval_seconds,
            "stale_after_seconds": 0
        });
    }

    let stale_after_seconds = interval_seconds.saturating_mul(2).max(60) as i64;
    let latest_completed_at = latest_cycle
        .and_then(|event| event.get("created_at"))
        .and_then(value_datetime_utc);
    let age_seconds =
        latest_completed_at.map(|completed_at| (Utc::now() - completed_at).num_seconds().max(0));
    let stale = age_seconds
        .map(|age| age > stale_after_seconds)
        .unwrap_or(true);
    let next_due_at = latest_completed_at
        .map(|completed_at| completed_at + Duration::seconds(interval_seconds as i64));

    json!({
        "enabled": true,
        "status": if latest_completed_at.is_none() {
            "no_cycle"
        } else if stale {
            "stale"
        } else {
            "current"
        },
        "healthy": !stale,
        "interval_seconds": interval_seconds,
        "stale_after_seconds": stale_after_seconds,
        "latest_completed_at": latest_completed_at,
        "latest_age_seconds": age_seconds,
        "next_due_at": next_due_at,
        "overdue_seconds": age_seconds.map(|age| (age - stale_after_seconds).max(0))
    })
}

fn value_datetime_utc(value: &Value) -> Option<DateTime<Utc>> {
    value
        .as_str()
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|value| value.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn test_ledger_summary(settled_count: usize, open_count: usize) -> LedgerSummary {
        LedgerSummary {
            count: settled_count + open_count,
            open_count,
            settled_count,
            turnover: 0.0,
            open_exposure: 0.0,
            simulated_return: 0.0,
            profit_loss: 0.0,
            hit_rate: None,
            average_odds: None,
            by_status: BTreeMap::new(),
        }
    }

    #[test]
    fn hermes_promotion_gate_blocks_insufficient_sample_and_open_exposure() {
        let strategy = json!({
            "experiments": [{
                "id": "experiment-1",
                "title": "Example",
                "status": "active_simulation",
                "variable_name": "excluded_risk_flags",
                "baseline_value": [],
                "proposed_value": ["large_odds_movement"],
                "evidence": {"safety": {"paper_only": true}},
                "decision_payload": {"replay_evidence": {"paper_only": true}}
            }]
        });
        let ledger = test_ledger_summary(27, 3);

        let gates = hermes_promotion_gates(&strategy, Some(&ledger));

        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0]["eligible_for_promotion"], json!(false));
        let blockers = gates[0]["blockers"].as_array().unwrap();
        assert!(blockers.contains(&json!("insufficient_settled_paper_positions")));
        assert!(blockers.contains(&json!("open_or_awaiting_paper_exposure")));
    }

    #[test]
    fn hermes_promotion_gate_allows_clear_active_simulation() {
        let strategy = json!({
            "experiments": [{
                "id": "experiment-1",
                "title": "Example",
                "status": "active_simulation",
                "variable_name": "max_decimal_odds",
                "baseline_value": 8.0,
                "proposed_value": 6.0,
                "evidence": {"safety": {"paper_only": true}},
                "decision_payload": {"replay_evidence": {"paper_only": true}}
            }]
        });
        let ledger = test_ledger_summary(100, 0);

        let gates = hermes_promotion_gates(&strategy, Some(&ledger));

        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0]["eligible_for_promotion"], json!(true));
        assert!(gates[0]["blockers"].as_array().unwrap().is_empty());
    }

    #[test]
    fn flashscore_variants_expand_localized_and_known_names() {
        assert!(flashscore_name_variants("Indien", "football").contains(&"India".to_string()));
        assert!(flashscore_name_variants("Bosnien-Hercegovina", "football")
            .contains(&"Bosnia and Herzegovina".to_string()));
        assert!(flashscore_name_variants("Irak", "football").contains(&"Iraq".to_string()));
        assert!(flashscore_name_variants("Nordmakedonien", "football")
            .contains(&"North Macedonia".to_string()));
        assert!(
            flashscore_name_variants("CD Maristas Palencia", "basketball")
                .contains(&"Palencia".to_string())
        );
        assert!(flashscore_name_variants("Cb Fuenlabrada", "basketball")
            .contains(&"Fuenlabrada".to_string()));
        assert!(
            flashscore_name_variants("Club Antonin Sportif", "basketball")
                .contains(&"Antonine".to_string())
        );
        assert!(
            flashscore_name_variants("Rinascita Basket Rimini", "basketball")
                .contains(&"Rimini".to_string())
        );
        assert!(flashscore_name_variants("Ueb Cividale", "basketball")
            .contains(&"Cividale".to_string()));
        assert!(
            flashscore_name_variants("Baskonia Vitoria-Gasteiz", "basketball")
                .contains(&"Baskonia".to_string())
        );
        assert!(flashscore_name_variants("Cb Malaga", "basketball").contains(&"Malaga".to_string()));
        assert!(
            flashscore_name_variants("Næstved BK", "football").contains(&"Naestved".to_string())
        );
        assert!(
            flashscore_name_variants("Fortaleza C.E.I.F. FC", "football")
                .contains(&"Fortaleza".to_string())
        );
        assert!(
            flashscore_name_variants("Internacional de Palmira (W)", "football")
                .contains(&"Inter Palmira W".to_string())
        );
        assert!(flashscore_name_variants("Derthona Basket", "basketball")
            .contains(&"Tortona".to_string()));
        assert!(flashscore_name_variants("Kamil Majchrzak", "tennis")
            .contains(&"majchrzak kamil".to_string()));
        assert!(flashscore_name_variants("Kolding IF (k)", "football")
            .contains(&"KoldingQ".to_string()));
        assert!(flashscore_name_variants("CR Vasco da Gama (W)", "football")
            .contains(&"Vasco W".to_string()));
        assert!(flashscore_name_variants("America MG (k)", "football")
            .contains(&"America Mineiro W".to_string()));
        assert!(flashscore_name_variants("Paris SG", "football").contains(&"PSG".to_string()));
        assert!(flashscore_name_variants("Paris Saint-Germain", "football")
            .contains(&"Paris SG".to_string()));
    }

    #[test]
    fn flashscore_participant_score_penalizes_wrong_gender() {
        let participant = FlashscoreParticipant {
            id: "example".to_string(),
            title: "Derthona Basket W (Italy)".to_string(),
            url: "derthona-basket".to_string(),
            participant_type_id: Some(1),
        };

        assert!(
            flashscore_participant_score(
                "Derthona Basket",
                "Derthona Basket",
                &participant,
                "basketball",
                None
            ) < 0.5
        );
        assert!(
            flashscore_participant_score(
                "Derthona Basket W",
                "Derthona Basket W",
                &participant,
                "basketball",
                Some("women"),
            ) > 1.0
        );
    }

    #[test]
    fn flashscore_participant_score_penalizes_team_sport_players() {
        let player = FlashscoreParticipant {
            id: "player".to_string(),
            title: "Kolding Mie (Vejgaard W)".to_string(),
            url: "kolding-mie".to_string(),
            participant_type_id: Some(2),
        };
        let team = FlashscoreParticipant {
            id: "team".to_string(),
            title: "KoldingQ W (Denmark)".to_string(),
            url: "koldingq".to_string(),
            participant_type_id: Some(1),
        };

        let player_score = flashscore_participant_score(
            "Kolding IF (k)",
            "KoldingQ",
            &player,
            "football",
            Some("women"),
        );
        let team_score = flashscore_participant_score(
            "Kolding IF (k)",
            "KoldingQ",
            &team,
            "football",
            Some("women"),
        );

        assert!(team_score > player_score);
    }

    #[test]
    fn infers_danish_k_marker_as_women_scope() {
        let selection = json!({
            "competition": "A-Liga",
            "event_name": "Kolding IF (k) - Fortuna Hjørring (k)",
            "market_name": "Kampvinder",
            "outcome_name": "Kolding IF (k)"
        });

        assert_eq!(
            infer_selection_gender_scope(&selection).as_deref(),
            Some("women")
        );
    }

    #[test]
    fn flashscore_row_scoring_handles_reversed_participant_ids() {
        let row = json!({
            "AA": "example-match",
            "AD": "1780000000",
            "AE": "Schoeman Marcus",
            "AF": "Loh Brendan",
            "AG": "2",
            "AH": "1",
            "PX": "away-id",
            "PY": "home-id"
        });
        let expected = DateTime::<Utc>::from_timestamp(1780003600, 0);

        let (score, reversed_side) = score_flashscore_row(
            &row,
            "Brendan Loh",
            "Marcus Schoeman",
            "home-id",
            "away-id",
            expected,
        );

        assert!(reversed_side);
        assert!(score >= 90.0);
    }

    #[test]
    fn detects_tennis_doubles_event_shape() {
        assert!(is_doubles_event(
            "Shimizu Y / Watanabe S",
            "Basel V / Oliveira B"
        ));
        assert!(!is_doubles_event("Casper Ruud", "Tommy Paul"));
    }

    #[test]
    fn flashscore_doubles_row_scoring_matches_pair_ids_and_status() {
        let row = json!({
            "AA": "6c0jXTg5",
            "AD": "1780000000",
            "AC": "9",
            "AW": "1",
            "PX": "IH1MWYjh/MmFrgePQ",
            "PY": "jgW0GVuP/b5qN4zQn"
        });
        let home_groups = vec![
            HashSet::from(["MmFrgePQ".to_string()]),
            HashSet::from(["IH1MWYjh".to_string()]),
        ];
        let away_groups = vec![
            HashSet::from(["b5qN4zQn".to_string()]),
            HashSet::from(["jgW0GVuP".to_string(), "pG57oMJR".to_string()]),
        ];
        let expected = DateTime::<Utc>::from_timestamp(1780003600, 0);

        let (score, reversed_side) =
            score_flashscore_doubles_row(&row, &home_groups, &away_groups, expected);

        assert!(!reversed_side);
        assert!(score >= 90.0);
        assert_eq!(flashscore_status_result(&row), Some("refunded"));
    }

    #[test]
    fn tennis_doubles_aliases_prefer_matched_feed_players() {
        let candidates = vec![
            vec![
                (
                    1.0,
                    FlashscoreParticipant {
                        id: "MmFrgePQ".to_string(),
                        title: "Shimizu Yuta (Japan)".to_string(),
                        url: "shimizu-yuta".to_string(),
                        participant_type_id: Some(2),
                    },
                ),
                (
                    1.0,
                    FlashscoreParticipant {
                        id: "wrong-shimizu".to_string(),
                        title: "Shimizu Ayano (Japan)".to_string(),
                        url: "shimizu-ayano".to_string(),
                        participant_type_id: Some(2),
                    },
                ),
            ],
            vec![
                (
                    1.0,
                    FlashscoreParticipant {
                        id: "IH1MWYjh".to_string(),
                        title: "Watanabe Seita (Japan)".to_string(),
                        url: "watanabe-seita".to_string(),
                        participant_type_id: Some(2),
                    },
                ),
                (
                    1.0,
                    FlashscoreParticipant {
                        id: "wrong-watanabe".to_string(),
                        title: "Watanabe-Giltz Jolene (France)".to_string(),
                        url: "watanabe-giltz-jolene".to_string(),
                        participant_type_id: Some(2),
                    },
                ),
            ],
        ];
        let matched_ids = HashSet::from(["IH1MWYjh".to_string(), "MmFrgePQ".to_string()]);

        let aliases = doubles_side_aliases(
            "Shimizu Y / Watanabe S",
            &["Shimizu Y".to_string(), "Watanabe S".to_string()],
            &candidates,
            &matched_ids,
        );

        assert!(aliases.contains(&"Shimizu Yuta".to_string()));
        assert!(aliases.contains(&"Watanabe Seita".to_string()));
        assert!(!aliases.contains(&"Shimizu Ayano".to_string()));
        assert!(!aliases.contains(&"Watanabe-Giltz Jolene".to_string()));
    }
}
