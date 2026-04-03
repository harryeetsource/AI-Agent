use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use api::{InputContentBlock, MessageRequest, MessageResponse};
use axum::{
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
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
        .route("/", get(index))
        .route("/static/app.js", get(static_app_js))
        .route("/static/styles.css", get(static_styles_css))
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



async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn static_app_js() -> Response {
    let mut response = Response::new(include_str!("../static/app.js").into());
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/javascript; charset=utf-8"),
    );
    response
}

async fn static_styles_css() -> Response {
    let mut response = Response::new(include_str!("../static/styles.css").into());
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    response
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
    let mut runner_request = maybe_enrich_request_with_local_sources(request);
    // The local API client already synthesizes streaming events from a full MessageResponse.
    // Force non-streaming here so llama.cpp returns a single JSON object instead of SSE.
    runner_request.stream = false;

    let response = state
        .http
        .post(&state.runner_messages_url)
        .json(&runner_request)
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

    let message = serde_json::from_str::<MessageResponse>(&body).map_err(|error| {
        format!(
            "runner returned invalid JSON from {}: {} | body: {}",
            state.runner_messages_url,
            error,
            body
        )
    })?;
    Ok(message)
}



const MAX_ANALYSIS_FILES: usize = 24;
const MAX_ANALYSIS_CHARS: usize = 120_000;

fn maybe_enrich_request_with_local_sources(request: &MessageRequest) -> MessageRequest {
    let mut runner_request = request.clone();
    let Some(user_text) = latest_user_text(&runner_request) else {
        return runner_request;
    };

    let Some(target_path) = detect_existing_path(&user_text) else {
        return runner_request;
    };

    let Ok(context) = build_source_context(&target_path) else {
        return runner_request;
    };
    if context.trim().is_empty() {
        return runner_request;
    }

    let analysis_instructions = format!(
        "You have been given local filesystem context captured from the user's requested path. When the user asks to analyze a directory, crate, workspace, or source file, analyze the actual provided files and directory structure instead of suggesting shell commands. Prefer concrete observations about module boundaries, architecture, APIs, bugs, safety issues, performance, and code quality.\n\nLOCAL SOURCE CONTEXT\n====================\n{context}"
    );

    match &mut runner_request.system {
        Some(system) => {
            if !system.contains("LOCAL SOURCE CONTEXT") {
                system.push_str("\n\n");
                system.push_str(&analysis_instructions);
            }
        }
        None => runner_request.system = Some(analysis_instructions),
    }

    runner_request
}

fn latest_user_text(request: &MessageRequest) -> Option<String> {
    request
        .messages
        .iter()
        .rev()
        .find(|message| message.role.eq_ignore_ascii_case("user"))
        .map(|message| {
            message
                .content
                .iter()
                .filter_map(|block| match block {
                    InputContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.trim().is_empty())
}

fn detect_existing_path(text: &str) -> Option<PathBuf> {
    for line in text.lines().rev() {
        let candidate = sanitize_path_candidate(line);
        if candidate.is_empty() {
            continue;
        }
        let path = PathBuf::from(&candidate);
        if path.exists() {
            return Some(path);
        }
    }

    for token in text.split_whitespace().rev() {
        let candidate = sanitize_path_candidate(token);
        if candidate.is_empty() {
            continue;
        }
        let path = PathBuf::from(&candidate);
        if path.exists() {
            return Some(path);
        }
    }

    None
}

fn sanitize_path_candidate(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'))
        .to_string()
}

fn build_source_context(path: &Path) -> Result<String, std::io::Error> {
    if path.is_file() {
        return build_file_context(path, MAX_ANALYSIS_CHARS);
    }
    if !path.is_dir() {
        return Ok(String::new());
    }

    let mut files = collect_source_files(path)?;
    if files.is_empty() {
        return Ok(format!("Requested path exists but no supported source files were found: {}", path.display()));
    }

    files.sort_by_key(|file| file_priority(file, path));

    let mut out = String::new();
    out.push_str(&format!("Requested directory: {}\n\n", path.display()));
    out.push_str("Discovered source files:\n");
    for file in files.iter().take(MAX_ANALYSIS_FILES) {
        out.push_str("- " );
        out.push_str(&display_relative(file, path));
        out.push('\n');
    }
    if files.len() > MAX_ANALYSIS_FILES {
        out.push_str(&format!("- ... {} more files omitted from listing\n", files.len() - MAX_ANALYSIS_FILES));
    }
    out.push_str("\n");

    let mut remaining = MAX_ANALYSIS_CHARS.saturating_sub(out.len());
    let mut included = 0usize;
    for file in files.into_iter().take(MAX_ANALYSIS_FILES) {
        if remaining < 800 {
            break;
        }
        let section = build_file_section(&file, path, remaining)?;
        if section.is_empty() {
            continue;
        }
        remaining = remaining.saturating_sub(section.len());
        out.push_str(&section);
        included += 1;
    }

    if included == 0 {
        out.push_str("No readable source file content could be captured.");
    }

    Ok(out)
}

fn build_file_context(path: &Path, budget: usize) -> Result<String, std::io::Error> {
    if !is_supported_source_file(path) {
        return Ok(format!("Requested file is not a supported source type: {}", path.display()));
    }
    build_file_section(path, path.parent().unwrap_or_else(|| Path::new(".")), budget)
}

fn build_file_section(path: &Path, root: &Path, budget: usize) -> Result<String, std::io::Error> {
    let content = fs::read_to_string(path)?;
    let mut section = String::new();
    let rel = display_relative(path, root);
    let lang = code_fence_lang(path);
    section.push_str(&format!("FILE: {rel}\n```{lang}\n"));

    let header_len = section.len() + 5;
    let allowance = budget.saturating_sub(header_len).max(256);
    let mut body = content;
    if body.len() > allowance {
        let mut cutoff = allowance.min(body.len());
        while !body.is_char_boundary(cutoff) && cutoff > 0 {
            cutoff -= 1;
        }
        body.truncate(cutoff);
        body.push_str("\n/* ... truncated for context budget ... */");
    }
    section.push_str(&body);
    section.push_str("\n```\n\n");
    Ok(section)
}

fn collect_source_files(root: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut out = Vec::new();
    collect_source_files_inner(root, &mut out)?;
    Ok(out)
}

fn collect_source_files_inner(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if path.is_dir() {
            if should_skip_dir(&name) {
                continue;
            }
            collect_source_files_inner(&path, out)?;
        } else if is_supported_source_file(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn should_skip_dir(name: &str) -> bool {
    matches!(name, "target" | ".git" | "node_modules" | "dist" | "build" | "vendor" | ".idea" | ".vscode" | "__pycache__")
}

fn is_supported_source_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    if matches!(name, "Cargo.toml" | "Cargo.lock" | "README.md" | "readme.md") {
        return true;
    }
    matches!(
        path.extension().and_then(|value| value.to_str()).unwrap_or_default().to_ascii_lowercase().as_str(),
        "rs" | "toml" | "md" | "txt" | "asm" | "s" | "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" | "json" | "yml" | "yaml"
    )
}

fn file_priority(path: &Path, root: &Path) -> (u8, String) {
    let rel = display_relative(path, root);
    let name = path.file_name().and_then(|value| value.to_str()).unwrap_or_default();
    let score = match name {
        "Cargo.toml" => 0,
        "lib.rs" => 1,
        "main.rs" => 2,
        "mod.rs" => 3,
        _ if rel.ends_with("README.md") || rel.ends_with("readme.md") => 4,
        _ if rel.ends_with(".rs") => 5,
        _ if rel.ends_with(".toml") => 6,
        _ => 7,
    };
    (score, rel)
}

fn display_relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn code_fence_lang(path: &Path) -> &'static str {
    match path.extension().and_then(|value| value.to_str()).unwrap_or_default().to_ascii_lowercase().as_str() {
        "rs" => "rust",
        "toml" => "toml",
        "md" => "markdown",
        "asm" | "s" => "asm",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" | "cxx" => "cpp",
        "json" => "json",
        "yml" | "yaml" => "yaml",
        _ => "text",
    }
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
