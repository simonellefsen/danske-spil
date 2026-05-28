use crate::config::Settings;
use crate::danske_spil::scan_sports;
use crate::models::CandidateBet;
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
                "result_agent_interval_seconds": self.settings.result_agent_interval_seconds
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
        let tasks: Vec<Value> = items
            .iter()
            .filter_map(|item| result_agent_task(item, &account_agent))
            .collect();

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
            "review_count": items.len(),
            "task_count": tasks.len(),
            "items": tasks,
            "source_precedence": [
                "danskespil_account_history",
                "official_competition_results",
                "flashscore_results",
                "sofascore_results",
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
        let http = match flashscore_http_client() {
            Ok(client) => client,
            Err(error) => {
                return json!({
                    "enabled": true,
                    "attempted_count": 0,
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

        for task in tasks.into_iter().take(limit) {
            let task_kind = text(&task, "task_kind").unwrap_or_default();
            if task_kind != "public_result_source_discovery" {
                skipped.push(json!({
                    "task_kind": task_kind,
                    "reason": "handled_by_account_agent_or_configured_link_worker",
                    "selection": task.get("selection").cloned().unwrap_or(Value::Null)
                }));
                continue;
            }

            match flashscore_discover(&http, &task).await {
                Ok(Some(evidence)) => {
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
                        "source_link": source_link_result,
                        "evidence_result": evidence_result,
                        "paper_only": true
                    });
                    self.store
                        .record_audit("result_agent_flashscore_discovery", result.clone())
                        .await
                        .ok();
                    results.push(result);
                }
                Ok(None) => skipped.push(json!({
                    "task_kind": task_kind,
                    "reason": "flashscore_discovery_no_match",
                    "selection": task.get("selection").cloned().unwrap_or(Value::Null)
                })),
                Err(error) => skipped.push(json!({
                    "task_kind": task_kind,
                    "reason": "flashscore_discovery_failed",
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
            "livescore_results"
        ],
        "evidence_endpoint": "/api/settlement/external-evidence",
        "source_link_endpoint": "/api/settlement/source-link"
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
) -> anyhow::Result<Option<FlashscoreEvidence>> {
    let selection = task.get("selection").unwrap_or(&Value::Null);
    let sport_key = text(selection, "sport_key")
        .unwrap_or_default()
        .to_lowercase();
    if flashscore_sport_id(&sport_key).is_none() {
        return Ok(None);
    }
    let Some(event_name) = text(selection, "event_name").map(str::trim) else {
        return Ok(None);
    };
    let Some((home_name, away_name)) = split_event_name(event_name) else {
        return Ok(None);
    };
    let gender_scope = infer_selection_gender_scope(selection);
    let Some(home) =
        best_flashscore_participant(http, &home_name, &sport_key, gender_scope.as_deref()).await?
    else {
        return Ok(None);
    };
    let Some(away) =
        best_flashscore_participant(http, &away_name, &sport_key, gender_scope.as_deref()).await?
    else {
        return Ok(None);
    };
    let Some(feed_sign) = fetch_flashscore_feed_sign(http, &home, &sport_key).await? else {
        return Ok(None);
    };
    let feed_name = format!("pe_2_2_{}_x", home.id);
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
    let expected_check_after = text(task, "expected_result_check_after")
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));

    let mut best: Option<(f64, bool, Value)> = None;
    for row in rows {
        let (score, reversed_side) = score_flashscore_row(
            &row,
            &home_name,
            &away_name,
            &home.id,
            &away.id,
            expected_check_after,
        );
        if score >= 90.0
            && best
                .as_ref()
                .map(|(current, _, _)| score > *current)
                .unwrap_or(true)
        {
            best = Some((score, reversed_side, row));
        }
    }
    let Some((match_score, reversed_side, row)) = best else {
        return Ok(None);
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
    Ok(Some(FlashscoreEvidence {
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
    }))
}

async fn best_flashscore_participant(
    http: &HttpClient,
    name: &str,
    sport_key: &str,
    gender_scope: Option<&str>,
) -> anyhow::Result<Option<FlashscoreParticipant>> {
    let queries = flashscore_name_variants(name, sport_key);
    let mut candidates = Vec::new();
    for query in queries {
        for participant in flashscore_search_participants(http, &query, sport_key).await? {
            let score = flashscore_participant_score(name, &query, &participant, gender_scope);
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
    Ok(candidates.into_iter().map(|(_, item)| item).next())
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
    score
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
        "danmark" => Some("Denmark"),
        "england" => Some("England"),
        "finland" => Some("Finland"),
        "frankrig" => Some("France"),
        "graekenland" => Some("Greece"),
        "holland" | "nederlandene" => Some("Netherlands"),
        "hviderusland" => Some("Belarus"),
        "indien" => Some("India"),
        "irland" => Some("Ireland"),
        "island" => Some("Iceland"),
        "italien" => Some("Italy"),
        "japan" => Some("Japan"),
        "kina" => Some("China"),
        "kroatien" => Some("Croatia"),
        "norge" => Some("Norway"),
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
    ];
    let men_markers = [
        "men",
        "mens",
        "male",
        "herre",
        "herrer",
        "herresingle",
        "atp",
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

fn boolish(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flashscore_variants_expand_localized_and_known_names() {
        assert!(flashscore_name_variants("Indien", "football").contains(&"India".to_string()));
        assert!(flashscore_name_variants("Derthona Basket", "basketball")
            .contains(&"Tortona".to_string()));
        assert!(flashscore_name_variants("Kamil Majchrzak", "tennis")
            .contains(&"majchrzak kamil".to_string()));
    }

    #[test]
    fn flashscore_participant_score_penalizes_wrong_gender() {
        let participant = FlashscoreParticipant {
            id: "example".to_string(),
            title: "Derthona Basket W (Italy)".to_string(),
            url: "derthona-basket".to_string(),
        };

        assert!(
            flashscore_participant_score("Derthona Basket", "Derthona Basket", &participant, None)
                < 0.5
        );
        assert!(
            flashscore_participant_score(
                "Derthona Basket W",
                "Derthona Basket W",
                &participant,
                Some("women"),
            ) > 1.0
        );
    }
}
