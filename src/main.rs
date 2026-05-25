mod config;
mod danske_spil;
mod models;
mod service;
mod store;
mod ui;

use axum::body::Bytes;
use axum::extract::{OriginalUri, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use config::Settings;
use serde_json::{json, Value};
use service::GamblerService;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use store::Store;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct AppState {
    settings: Settings,
    service: GamblerService,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let settings = Settings::load();
    let store = Store::new(settings.database_url.clone());
    if let Err(error) = store.init_schema().await {
        tracing::warn!(%error, "initial schema setup failed; service will keep retrying");
    }
    let schema_store = store.clone();
    tokio::spawn(async move {
        loop {
            match schema_store.init_schema().await {
                Ok(()) => {
                    tracing::info!("schema setup available");
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "schema setup retry failed");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
        }
    });
    let service = GamblerService::new(settings.clone(), store);

    match std::env::args().nth(1).as_deref() {
        Some("worker") => run_worker(settings, service).await,
        _ => run_http(settings, service).await,
    }
}

async fn run_worker(settings: Settings, service: GamblerService) -> anyhow::Result<()> {
    loop {
        match service.scan(false).await {
            Ok(result) => tracing::info!(
                snapshot_id = result
                    .get("snapshot_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default(),
                candidate_count = result
                    .get("candidate_count")
                    .and_then(|value| value.as_u64())
                    .unwrap_or_default(),
                "scan_completed"
            ),
            Err(error) => tracing::warn!(%error, "scan_failed"),
        }
        let queue_summary = service.advance_settlement_queue().await;
        tracing::info!(
            transitioned_count = queue_summary
                .get("transitioned_count")
                .and_then(|value| value.as_u64())
                .unwrap_or_default(),
            "settlement_queue_advanced"
        );
        tokio::time::sleep(Duration::from_secs(settings.scan_interval_seconds)).await;
    }
}

async fn run_http(settings: Settings, service: GamblerService) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", settings.host, settings.port).parse()?;
    let state = Arc::new(AppState { settings, service });
    let app = Router::new()
        .route("/", get(get_handler).post(post_handler))
        .route("/*path", get(get_handler).post(post_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    tracing::info!(%addr, "starting rust/dioxus gambler service");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn get_handler(State(state): State<Arc<AppState>>, uri: OriginalUri) -> Response {
    let path = normalized_path(uri.0.path(), &state.settings.base_path);
    match path.as_str() {
        "/" | "/index.html" => Html(ui::render_index(&state.settings.base_path)).into_response(),
        "/healthz" => Json(json!({"ok": true, "component": state.settings.component})).into_response(),
        "/readyz" => Json(json!({"ok": true, "database": state.service.store().status().await})).into_response(),
        "/api/status" => Json(state.service.status().await).into_response(),
        "/api/snapshots/latest" => match state.service.store().latest_snapshot().await {
            Ok(item) => Json(json!({"item": item})).into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        },
        "/api/candidates" => match state.service.store().candidates(50).await {
            Ok(items) => Json(json!({"items": items})).into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        },
        "/api/ledger" => match state.service.store().simulated_bets(50).await {
            Ok(items) => Json(json!({"items": items})).into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        },
        "/api/ledger/summary" => match state.service.store().ledger_summary().await {
            Ok(summary) => Json(summary).into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        },
        "/api/ledger/queue" => Json(state.service.advance_settlement_queue().await).into_response(),
        "/api/catalog/coverage" => match state.service.store().market_catalog_coverage().await {
            Ok(coverage) => Json(coverage).into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        },
        "/api/intelligence/coverage" => {
            match state.service.store().intelligence_coverage().await {
                Ok(coverage) => Json(coverage).into_response(),
                Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
            }
        },
        "/api/hermes" => match state.service.store().hermes_reflections(25).await {
            Ok(reflections) => match state.service.store().strategy_state().await {
                Ok(strategy) => Json(json!({
                    "mode": "poc_view",
                    "summary": "Hermes integration is read-only in this POC. Reflections and one-variable experiment proposals are loaded from Postgres.",
                    "reflections": reflections,
                    "strategy": strategy
                }))
                .into_response(),
                Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
            },
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        },
        "/api/strategy" => match state.service.store().strategy_state().await {
            Ok(strategy) => Json(strategy).into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        },
        "/api/strategy/decisions" => match state.service.store().strategy_decisions(100).await {
            Ok(decisions) => Json(decisions).into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        },
        "/api/hermes/reflections" => match state.service.store().hermes_reflections(25).await {
            Ok(reflections) => Json(json!({
                "mode": "poc_view",
                "summary": "Hermes integration is read-only in this POC. Reflections are loaded from Postgres when available.",
                "reflections": reflections
            }))
            .into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        },
        _ => error_response(StatusCode::NOT_FOUND, anyhow::anyhow!("not found")),
    }
}

async fn post_handler(
    State(state): State<Arc<AppState>>,
    uri: OriginalUri,
    body: Bytes,
) -> Response {
    let path = normalized_path(uri.0.path(), &state.settings.base_path);
    let payload = parse_json_body(body);
    match path.as_str() {
        "/api/scan" => {
            let include_live = payload
                .get("include_live")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            match state.service.scan(include_live).await {
                Ok(result) => Json(result).into_response(),
                Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
            }
        }
        "/api/simulate" => {
            let candidate_id = payload
                .get("candidate_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let stake = payload
                .get("stake")
                .and_then(Value::as_f64)
                .unwrap_or(state.settings.default_stake);
            match state
                .service
                .store()
                .simulate_bet(candidate_id, stake)
                .await
            {
                Ok(item) => {
                    state
                        .service
                        .store()
                        .record_audit(
                            "paper_bet_created",
                            json!({"candidate_id": candidate_id, "stake": stake}),
                        )
                        .await
                        .ok();
                    (StatusCode::CREATED, Json(json!({"item": item}))).into_response()
                }
                Err(error) => error_response(StatusCode::BAD_REQUEST, error),
            }
        }
        "/api/simulate/selected" => {
            let snapshot_id = payload.get("snapshot_id").and_then(Value::as_str);
            let stake = payload
                .get("stake")
                .and_then(Value::as_f64)
                .unwrap_or(state.settings.default_stake);
            let limit = payload
                .get("limit")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(state.settings.auto_paper_per_scan_limit);
            let max_open_exposure = payload
                .get("max_open_exposure")
                .and_then(Value::as_f64)
                .unwrap_or(state.settings.auto_paper_max_open_exposure);
            match state
                .service
                .store()
                .paper_place_selected(snapshot_id, stake, limit, max_open_exposure)
                .await
            {
                Ok(summary) => {
                    state
                        .service
                        .store()
                        .record_audit("paper_selected_auto_placed", summary.clone())
                        .await
                        .ok();
                    Json(summary).into_response()
                }
                Err(error) => error_response(StatusCode::BAD_REQUEST, error),
            }
        }
        "/api/ledger/settle" => {
            let bet_id = payload
                .get("bet_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let result = payload
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let source = payload
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("manual_operator_review");
            let confidence = payload
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or(1.0);
            let notes = payload
                .get("notes")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match state
                .service
                .store()
                .settle_simulated_bet(bet_id, result, source, confidence, notes)
                .await
            {
                Ok(item) => {
                    state
                        .service
                        .store()
                        .record_audit(
                            "paper_bet_settled",
                            json!({"bet_id": item.id, "status": item.status, "source": source}),
                        )
                        .await
                        .ok();
                    Json(json!({"item": item})).into_response()
                }
                Err(error) => error_response(StatusCode::BAD_REQUEST, error),
            }
        }
        "/api/ledger/queue" => Json(state.service.advance_settlement_queue().await).into_response(),
        "/api/strategy/experiment/review" => {
            let experiment_id = payload
                .get("experiment_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let action = payload
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let notes = payload
                .get("notes")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match state
                .service
                .store()
                .review_strategy_experiment(experiment_id, action, notes)
                .await
            {
                Ok(item) => {
                    state
                        .service
                        .store()
                        .record_audit(
                            "strategy_experiment_reviewed",
                            json!({"experiment_id": item.get("id"), "status": item.get("status")}),
                        )
                        .await
                        .ok();
                    Json(json!({"item": item})).into_response()
                }
                Err(error) => error_response(StatusCode::BAD_REQUEST, error),
            }
        }
        _ => error_response(StatusCode::NOT_FOUND, anyhow::anyhow!("not found")),
    }
}

fn normalized_path(path: &str, base_path: &str) -> String {
    if !base_path.is_empty() && path.starts_with(base_path) {
        let stripped = &path[base_path.len()..];
        if stripped.is_empty() {
            "/".to_string()
        } else {
            stripped.to_string()
        }
    } else {
        path.to_string()
    }
}

fn parse_json_body(body: Bytes) -> Value {
    if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&body).unwrap_or_else(|_| json!({}))
    }
}

fn error_response(status: StatusCode, error: anyhow::Error) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        Json(json!({"error": error.to_string()})),
    )
        .into_response()
}
