use axum::body::Bytes;
use axum::extract::{OriginalUri, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use danske_spil_gambler::config::Settings;
use danske_spil_gambler::service::GamblerService;
use danske_spil_gambler::store::Store;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
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
    let loop_service = service.clone();
    let interval_seconds = settings.result_agent_interval_seconds;
    tokio::spawn(async move {
        loop {
            let result_agent_summary = loop_service.run_result_agent_once().await;
            tracing::info!(
                attempted_count = result_agent_summary
                    .get("attempted_count")
                    .and_then(|value| value.as_u64())
                    .unwrap_or_default(),
                settled_count = result_agent_summary
                    .get("settled_count")
                    .and_then(|value| value.as_u64())
                    .unwrap_or_default(),
                "result_agent_cycle_completed"
            );
            tokio::time::sleep(Duration::from_secs(interval_seconds)).await;
        }
    });

    run_http(settings, service).await
}

async fn run_http(settings: Settings, service: GamblerService) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", settings.host, settings.port).parse()?;
    let state = Arc::new(AppState { settings, service });
    let app = Router::new()
        .route("/", get(get_handler).post(post_handler))
        .route("/*path", get(get_handler).post(post_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    tracing::info!(%addr, "starting result-agent service");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn get_handler(State(state): State<Arc<AppState>>, uri: OriginalUri) -> Response {
    let path = normalized_path(uri.0.path(), &state.settings.base_path);
    match path.as_str() {
        "/" | "/healthz" => {
            Json(json!({"ok": true, "component": state.settings.component})).into_response()
        }
        "/readyz" => Json(json!({"ok": true, "database": state.service.store().status().await}))
            .into_response(),
        "/api/status" => Json(state.service.status().await).into_response(),
        "/api/result-agent/queue" => Json(state.service.result_agent_queue().await).into_response(),
        "/api/result-agent/run" => {
            Json(state.service.run_result_agent_once().await).into_response()
        }
        "/api/settlement/source-links" => {
            match state.service.store().external_result_links(50).await {
                Ok(links) => Json(links).into_response(),
                Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
            }
        }
        "/api/settlement/external-evidence" => {
            match state.service.store().external_result_evidence(50).await {
                Ok(evidence) => Json(evidence).into_response(),
                Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
            }
        }
        "/api/settlement/lookup-attempts" => {
            match state.service.store().settlement_lookup_attempts(50).await {
                Ok(attempts) => Json(attempts).into_response(),
                Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
            }
        }
        "/api/aliases" => match state.service.store().entity_aliases(100).await {
            Ok(aliases) => Json(aliases).into_response(),
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
        "/api/result-agent/run" => {
            Json(state.service.run_result_agent_once().await).into_response()
        }
        "/api/settlement/external-evidence" => match state
            .service
            .store()
            .ingest_external_result_evidence(&payload)
            .await
        {
            Ok(summary) => {
                state
                    .service
                    .store()
                    .record_audit("external_result_evidence_ingested", summary.clone())
                    .await
                    .ok();
                Json(summary).into_response()
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, error),
        },
        "/api/settlement/source-link" => match state
            .service
            .store()
            .add_external_result_link(&payload)
            .await
        {
            Ok(summary) => {
                state
                    .service
                    .store()
                    .record_audit("external_result_link_added", summary.clone())
                    .await
                    .ok();
                Json(summary).into_response()
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, error),
        },
        "/api/aliases" => match state.service.store().add_entity_alias(&payload).await {
            Ok(summary) => {
                state
                    .service
                    .store()
                    .record_audit("entity_alias_added", summary.clone())
                    .await
                    .ok();
                Json(summary).into_response()
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, error),
        },
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
