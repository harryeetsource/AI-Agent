use std::env;
use std::net::SocketAddr;
use std::time::Duration;

use api::{MessageRequest, MessageResponse};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use reqwest::Client;
use serde::Serialize;
use tracing::{error, info};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8080;
const DEFAULT_RUNNER_BASE_URL: &str = "http://127.0.0.1:8081";
const DEFAULT_RUNNER_MESSAGES_PATH: &str = "/v1/messages";

#[derive(Clone)]
struct AppState {
    http: Client,
    runner_messages_url: String,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    runner_messages_url: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "clawd=info".into()),
        )
        .init();

    let host = env::var("CLAWD_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string());
    let port = env::var("CLAWD_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);

    let runner_base_url = normalize_base_url(
        env::var("CLAW_RUNNER_BASE_URL")
            .or_else(|_| env::var("LLAMA_BASE_URL"))
            .unwrap_or_else(|_| DEFAULT_RUNNER_BASE_URL.to_string()),
    );
    let runner_messages_path = env::var("CLAW_RUNNER_MESSAGES_PATH")
        .unwrap_or_else(|_| DEFAULT_RUNNER_MESSAGES_PATH.to_string());
    let runner_messages_url = format!(
        "{}{}",
        runner_base_url.trim_end_matches('/'),
        ensure_leading_slash(&runner_messages_path)
    );

    let http = Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?;

    let state = AppState {
        http,
        runner_messages_url: runner_messages_url.clone(),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/messages", post(messages))
        .with_state(state);

    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    info!(%addr, %runner_messages_url, "starting clawd offline daemon");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            sigterm.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok",
            runner_messages_url: state.runner_messages_url,
        }),
    )
}

async fn messages(
    State(state): State<AppState>,
    Json(request): Json<MessageRequest>,
) -> impl IntoResponse {
    match forward_message_request(&state, &request).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => {
            error!(%error, "failed to forward message request to local runner");
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    error: error.to_string(),
                }),
            )
                .into_response()
        }
    }
}

async fn forward_message_request(
    state: &AppState,
    request: &MessageRequest,
) -> Result<MessageResponse, Box<dyn std::error::Error + Send + Sync>> {
    let response = state
        .http
        .post(&state.runner_messages_url)
        .json(request)
        .send()
        .await?;

    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!(
            "runner returned HTTP {} from {}: {}",
            status,
            state.runner_messages_url,
            body
        )
        .into());
    }

    let message = serde_json::from_str::<MessageResponse>(&body)?;
    Ok(message)
}

fn normalize_base_url(value: String) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        DEFAULT_RUNNER_BASE_URL.to_string()
    } else {
        trimmed.trim_end_matches('/').to_string()
    }
}

fn ensure_leading_slash(value: &str) -> String {
    if value.starts_with('/') {
        value.to_string()
    } else {
        format!("/{value}")
    }
}
