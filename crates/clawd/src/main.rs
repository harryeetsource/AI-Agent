use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use api::{
    InputContentBlock, InputMessage, MessageRequest, MessageResponse, OutputContentBlock,
    ToolChoice, ToolDefinition,
};
use axum::{
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use plugins::{HookRunner, PluginManager, PluginManagerConfig};
use reqwest::Client;
use rusqlite::{params, Connection};
use runtime::{ConfigLoader, RuntimeConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tools::GlobalToolRegistry;
use tracing::{error, info};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8080;
const DEFAULT_RUNNER_BASE_URL: &str = "http://127.0.0.1:8081";
const DEFAULT_RUNNER_MESSAGES_PATH: &str = "/v1/messages";
const DEFAULT_DB_PATH: &str = "data/knowledge.db";

const MAX_FALLBACK_FILES: usize = 12;
const MAX_FALLBACK_FILE_CHARS: usize = 10_000;
const MAX_FALLBACK_TOTAL_CHARS: usize = 24_000;
const MAX_TOOL_ROUNDS: usize = 1;
const MAX_SUMMARY_FILES: usize = 12;
const MAX_TOOL_OUTPUT_CHARS: usize = 4000;

const FILE_REWRITE_MAX_TOKENS: u32 = 20_000;
const FILE_REWRITE_CONTINUATION_MAX_TOKENS: u32 = 8_000;
const MAX_FILE_REWRITE_CONTINUATIONS: usize = 3;

const MAX_HISTORY_MESSAGES_BEFORE_COMPACTION: usize = 12;
const KEEP_RECENT_MESSAGES: usize = 8;
const MAX_SUMMARY_CHARS: usize = 6_000;
const MAX_MESSAGE_TEXT_CHARS: usize = 2_000;

#[derive(Clone)]
struct AppState {
    http: Client,
    runner_messages_url: String,
    tool_registry: GlobalToolRegistry,
    hook_runner: HookRunner,
    db_path: PathBuf,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    runner_messages_url: String,
    db_path: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Deserialize)]
struct FeedbackRequest {
    prompt: String,
    response: String,
    repo_path: Option<String>,
    files_reviewed: Option<Vec<String>>,
    helpful: bool,
    note: Option<String>,
    duration_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
struct FeedbackResponse {
    status: &'static str,
    analysis_run_id: i64,
}

enum RewriteAssessment {
    Complete,
    Incomplete { partial_code: String },
    Malformed(String),
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

    let db_path = resolve_db_path();
    ensure_feedback_schema(&db_path)?;

    let http = Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()?;

    let (tool_registry, hook_runner) =
        build_tooling_state().map_err(std::io::Error::other)?;

    let state = AppState {
        http,
        runner_messages_url: runner_messages_url.clone(),
        tool_registry,
        hook_runner,
        db_path,
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/static/app.js", get(static_app_js))
        .route("/static/styles.css", get(static_styles_css))
        .route("/health", get(health))
        .route("/v1/messages", post(messages))
        .route("/feedback", post(feedback))
        .with_state(state);

    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    info!(%addr, %runner_messages_url, "starting clawd offline daemon");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn resolve_db_path() -> PathBuf {
    if let Ok(value) = env::var("CLAWD_DB_PATH") {
        return PathBuf::from(value);
    }
    PathBuf::from(DEFAULT_DB_PATH)
}

fn ensure_feedback_schema(db_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let conn = Connection::open(db_path)?;

    conn.execute_batch(
        r#"
        PRAGMA journal_mode=WAL;
        PRAGMA synchronous=NORMAL;
        PRAGMA foreign_keys=ON;

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS documents (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            path TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS chunks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            document_id INTEGER NOT NULL,
            chunk_index INTEGER NOT NULL,
            text TEXT NOT NULL,
            FOREIGN KEY(document_id) REFERENCES documents(id) ON DELETE CASCADE,
            UNIQUE(document_id, chunk_index)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
            text,
            content='chunks',
            content_rowid='id'
        );

        CREATE TABLE IF NOT EXISTS sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_key TEXT NOT NULL UNIQUE,
            summary TEXT,
            transcript_json TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS tool_results (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tool_name TEXT NOT NULL,
            input_json TEXT NOT NULL,
            output_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS analysis_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_key TEXT,
            prompt_text TEXT NOT NULL,
            response_text TEXT NOT NULL,
            repo_path TEXT,
            repo_path_sha256 TEXT,
            files_reviewed_json TEXT NOT NULL DEFAULT '[]',
            helpful INTEGER,
            feedback_note TEXT,
            duration_ms INTEGER,
            source_mode TEXT NOT NULL DEFAULT 'filesystem',
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_analysis_runs_repo_path
        ON analysis_runs(repo_path);

        CREATE INDEX IF NOT EXISTS idx_analysis_runs_helpful
        ON analysis_runs(helpful);

        CREATE TABLE IF NOT EXISTS approved_summaries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            analysis_run_id INTEGER NOT NULL UNIQUE,
            repo_path TEXT,
            summary_text TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(analysis_run_id) REFERENCES analysis_runs(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_approved_summaries_repo_path
        ON approved_summaries(repo_path);

        INSERT OR IGNORE INTO settings(key, value) VALUES ('schema_version', '2');
        "#,
    )?;

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
            runner_messages_url: state.runner_messages_url.clone(),
            db_path: state.db_path.display().to_string(),
        }),
    )
}

async fn feedback(
    State(state): State<AppState>,
    Json(payload): Json<FeedbackRequest>,
) -> impl IntoResponse {
    match store_feedback(&state.db_path, payload) {
        Ok(analysis_run_id) => (
            StatusCode::OK,
            Json(FeedbackResponse {
                status: "ok",
                analysis_run_id,
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

fn store_feedback(
    db_path: &Path,
    payload: FeedbackRequest,
) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
    let conn = Connection::open(db_path)?;

    let files_json = serde_json::to_string(&payload.files_reviewed.unwrap_or_default())?;
    let repo_path_sha256 = payload.repo_path.as_deref().map(sha256_hex);

    conn.execute(
        r#"
        INSERT INTO analysis_runs (
            session_key,
            prompt_text,
            response_text,
            repo_path,
            repo_path_sha256,
            files_reviewed_json,
            helpful,
            feedback_note,
            duration_ms,
            source_mode
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
        params![
            Option::<String>::None,
            payload.prompt,
            payload.response,
            payload.repo_path,
            repo_path_sha256,
            files_json,
            if payload.helpful { 1 } else { 0 },
            payload.note.unwrap_or_default(),
            payload.duration_ms,
            "filesystem",
        ],
    )?;

    let analysis_run_id = conn.last_insert_rowid();

    if payload.helpful {
        conn.execute(
            r#"
            INSERT INTO approved_summaries (
                analysis_run_id,
                repo_path,
                summary_text
            ) VALUES (?1, ?2, ?3)
            "#,
            params![analysis_run_id, payload.repo_path, payload.response],
        )?;
    }

    Ok(analysis_run_id)
}

fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();

    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{:02x}", byte);
    }
    out
}

async fn messages(
    State(state): State<AppState>,
    Json(request): Json<MessageRequest>,
) -> impl IntoResponse {
    match process_message_request(&state, &request).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => {
            error!(%error, "failed to process message request");
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

fn request_needs_repo_analysis(request: &MessageRequest) -> bool {
    let Some(text) = latest_user_text(request) else {
        return false;
    };

    let lower = text.to_ascii_lowercase();
    let detected = detect_existing_path(&text);
    let is_dir = detected.as_deref().map(|p| p.is_dir()).unwrap_or(false);

    is_dir
        && [
            "analyze",
            "analysis",
            "control flow",
            "overview",
            "repository",
            "repo",
            "codebase",
            "module",
            "entry point",
            "entrypoint",
            "rust code",
            "files that you reviewed",
            "important modules",
        ]
        .iter()
        .any(|term| lower.contains(term))
}

fn request_needs_file_fix(request: &MessageRequest) -> bool {
    let Some(text) = latest_user_text(request) else {
        return false;
    };

    let lower = text.to_ascii_lowercase();
    let detected = detect_existing_path(&text);
    let is_file = detected.as_deref().map(|p| p.is_file()).unwrap_or(false);

    is_file
        && [
            "fix",
            "full source code fix",
            "rewrite",
            "patch",
            "update this file",
            "modify this file",
            "correct this file",
            "repair this file",
            "provide a version of this file",
            "provide a version",
            "full rewrite",
            "full file rewrite",
            "full replacement",
            "complete file",
            "complete replacement",
            "return the full file",
            "provide the full file",
            "rewrite this file",
            "replacement file",
            "version of this file",
        ]
        .iter()
        .any(|term| lower.contains(term))
}

fn build_repo_fallback_context(request: &MessageRequest) -> Option<String> {
    let user_text = latest_user_text(request)?;
    let target_path = detect_existing_path(&user_text)?;
    build_repo_fallback_context_for_path(&target_path).ok()
}

fn build_repo_fallback_context_for_path(path: &Path) -> Result<String, std::io::Error> {
    if path.is_file() {
        return build_single_file_fallback(path);
    }

    if !path.is_dir() {
        return Ok(String::new());
    }

    let mut files = collect_source_files(path)?;
    if files.is_empty() {
        return Ok(String::new());
    }

    files.sort_by_key(|file| file_priority(file, path));

    let mut selected = Vec::new();

    for file in &files {
        let rel = display_relative(file, path);
        let rel_lower = rel.to_ascii_lowercase();

        if rel == "Cargo.toml"
            || rel_lower.ends_with("/cargo.toml")
            || rel_lower.ends_with("/src/main.rs")
            || rel_lower.ends_with("/src/lib.rs")
            || rel_lower.ends_with("/mod.rs")
        {
            selected.push(file.clone());
        }

        if selected.len() >= MAX_FALLBACK_FILES {
            break;
        }
    }

    if selected.is_empty() {
        selected.extend(files.into_iter().take(MAX_FALLBACK_FILES));
    }

    let mut out = String::new();
    out.push_str(&format!("Repository root: {}\n\n", path.display()));
    out.push_str("Selected files:\n");
    for file in &selected {
        out.push_str("- ");
        out.push_str(&display_relative(file, path));
        out.push('\n');
    }
    out.push('\n');

    let mut remaining = MAX_FALLBACK_TOTAL_CHARS.saturating_sub(out.len());

    for file in selected {
        if remaining < 512 {
            break;
        }

        let rel = display_relative(&file, path);
        let lang = code_fence_lang(&file);
        let content = fs::read_to_string(&file)?;

        let snippet = truncate_chars(&content, MAX_FALLBACK_FILE_CHARS.min(remaining));
        let section = format!("FILE: {rel}\n```{lang}\n{snippet}\n```\n\n");

        remaining = remaining.saturating_sub(section.len());
        out.push_str(&section);
    }

    Ok(out)
}

fn build_single_file_fallback(path: &Path) -> Result<String, std::io::Error> {
    let lang = code_fence_lang(path);
    let content = fs::read_to_string(path)?;
    let snippet = truncate_chars(&content, MAX_FALLBACK_FILE_CHARS);

    Ok(format!(
        "Requested file: {}\n\n```{}\n{}\n```\n",
        path.display(),
        lang,
        snippet
    ))
}

fn build_file_fix_fallback_request(
    request: &MessageRequest,
    file_path: &Path,
) -> Result<MessageRequest, std::io::Error> {
    let mut req = request.clone();
    req.stream = false;
    req.tools = None;
    req.tool_choice = None;
    req.max_tokens = FILE_REWRITE_MAX_TOKENS;

    let content = fs::read_to_string(file_path)?;
    let lang = code_fence_lang(file_path);

    let synthesis = format!(
        "You are rewriting a single source file.\n\
Return EXACTLY ONE fenced {} code block containing the COMPLETE replacement file.\n\
Do not include any prose before or after the code block.\n\
Do not include diagnosis.\n\
Do not include explanation.\n\
Do not shorten the file.\n\
Do not omit sections.\n\
Preserve the original file's structure, formatting style, comment density, helper layout, and function ordering unless a change is absolutely necessary.\n\
If you cannot complete the full file in one response, return a single fenced code block containing the longest correct continuation you can produce.\n\n\
TARGET FILE: {}\n\
```{}\n{}\n```",
        lang,
        file_path.display(),
        lang,
        content
    );

    match &mut req.system {
        Some(system) => {
            system.push_str("\n\n");
            system.push_str(&synthesis);
        }
        None => req.system = Some(synthesis),
    }

    Ok(req)
}

fn build_file_rewrite_continuation_request(
    request: &MessageRequest,
    file_path: &Path,
    lang: &str,
    source: &str,
    current_partial: &str,
) -> MessageRequest {
    let mut req = request.clone();
    req.stream = false;
    req.tools = None;
    req.tool_choice = None;
    req.max_tokens = FILE_REWRITE_CONTINUATION_MAX_TOKENS;

    let tail = last_n_lines(current_partial, 80);

    let synthesis = format!(
        "You are continuing a partially generated rewrite of a single source file.\n\
Return EXACTLY ONE fenced {} code block containing ONLY the NEXT continuation chunk.\n\
Do not restart from the beginning.\n\
Do not repeat large earlier sections.\n\
Continue directly after the existing partial output.\n\
Do not include prose.\n\
Do not include explanation.\n\
If the file is already complete, return EXACTLY the single word DONE.\n\n\
TARGET FILE PATH: {}\n\n\
ORIGINAL SOURCE:\n\
```{}\n{}\n```\n\n\
CURRENT PARTIAL OUTPUT TAIL:\n\
```{}\n{}\n```",
        lang,
        file_path.display(),
        lang,
        source,
        lang,
        tail
    );

    match &mut req.system {
        Some(system) => {
            system.push_str("\n\n");
            system.push_str(&synthesis);
        }
        None => req.system = Some(synthesis),
    }

    req
}

fn code_fence_lang(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => "rust",
        "toml" => "toml",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "md" => "markdown",
        "txt" => "text",
        "lock" => "toml",
        "sh" => "bash",
        "ps1" => "powershell",
        "c" => "c",
        "h" => "c",
        "cpp" | "cc" | "cxx" => "cpp",
        "hpp" => "cpp",
        "py" => "python",
        "js" => "javascript",
        "ts" => "typescript",
        "go" => "go",
        "java" => "java",
        "cs" => "csharp",
        _ => "",
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    out.push_str("\n/* ... truncated ... */");
    out
}

fn build_fallback_synthesis_request(
    request: &MessageRequest,
    fallback_context: &str,
) -> MessageRequest {
    let mut req = request.clone();
    req.stream = false;
    req.tools = None;
    req.tool_choice = None;

    let synthesis = format!(
        "You are being given a bounded local repository snapshot for analysis.\n\
Prioritize the ROOT project at the repository root unless the user explicitly asked for a nested subproject.\n\
Treat nested crates or subprojects as secondary unless they are required to explain the root project's behavior.\n\
Analyze the Rust codebase based only on the provided files.\n\
Give:\n\
1. the important modules in the root project\n\
2. the likely root entry points\n\
3. control flow between major root files/modules\n\
4. any important nested subprojects only if they materially affect the root project\n\
5. the files reviewed and each file's role\n\n\
REPOSITORY SNAPSHOT\n\
===================\n\
{fallback_context}"
    );

    match &mut req.system {
        Some(system) => {
            system.push_str("\n\n");
            system.push_str(&synthesis);
        }
        None => req.system = Some(synthesis),
    }

    req
}

async fn process_message_request(
    state: &AppState,
    request: &MessageRequest,
) -> Result<MessageResponse, Box<dyn std::error::Error + Send + Sync>> {
    let mut runner_request = maybe_enrich_request_with_local_sources(request);
    runner_request = compact_request_history(&runner_request);

    let require_tool_use = request_has_local_path(request);
    let wants_repo_analysis = request_needs_repo_analysis(request);
    let wants_file_fix = request_needs_file_fix(request);
    let detected_path = latest_user_text(request).and_then(|text| detect_existing_path(&text));
    let mut saw_real_tool_use = false;

    if wants_file_fix {
        if let Some(path) = detected_path.as_deref() {
            if path.is_file() {
                return run_single_file_rewrite(state, request, path).await;
            }
        }
    }

    runner_request.stream = false;

    if should_enable_tools(request) && !wants_repo_analysis && !wants_file_fix {
        let tool_specs = filtered_tool_specs(&state.tool_registry);
        if !tool_specs.is_empty() {
            runner_request.tools = Some(tool_specs);
            if runner_request.tool_choice.is_none() {
                runner_request.tool_choice = Some(ToolChoice::Auto);
            }
        }
    }

    for _round in 0..MAX_TOOL_ROUNDS {
        let response = call_runner(state, &runner_request).await?;
        let tool_uses = extract_tool_uses(&response);

        if tool_uses.is_empty() {
            let fake_plan = looks_like_fake_tool_plan(&response);

            if require_tool_use && !saw_real_tool_use {
                if wants_repo_analysis {
                    if let Some(fallback_context) = build_repo_fallback_context(request) {
                        if !fallback_context.trim().is_empty() {
                            let fallback_request =
                                build_fallback_synthesis_request(request, &fallback_context);
                            return call_runner(state, &fallback_request).await;
                        }
                    }
                }

                if fake_plan {
                    return Err(std::io::Error::other(
                        "model failed to emit real tool calls; it only produced a textual tool plan",
                    )
                    .into());
                }

                return Ok(response);
            }

            return Ok(response);
        }

        saw_real_tool_use = true;

        if let Some(assistant_message) = output_blocks_to_input_message(&response.content) {
            runner_request.messages.push(assistant_message);
        }

        for tool_use in tool_uses {
            let tool_input_json = serde_json::to_string(&tool_use.input)?;

            let pre = state
                .hook_runner
                .run_pre_tool_use(&tool_use.name, &tool_input_json);

            let (mut output, is_error) = if pre.is_denied() {
                let denied_message = if pre.messages().is_empty() {
                    format!("tool `{}` denied by PreToolUse hook", tool_use.name)
                } else {
                    pre.messages().join("\n")
                };
                (denied_message, true)
            } else {
                match state.tool_registry.execute(&tool_use.name, &tool_use.input) {
                    Ok(output) => (output, false),
                    Err(error) => (error, true),
                }
            };

            let post = state.hook_runner.run_post_tool_use(
                &tool_use.name,
                &tool_input_json,
                &output,
                is_error,
            );

            if !post.messages().is_empty() {
                if !output.is_empty() {
                    output.push('\n');
                }
                output.push_str(&post.messages().join("\n"));
            }

            let output = truncate_tool_output(&output, MAX_TOOL_OUTPUT_CHARS);

            runner_request.messages.push(InputMessage::user_tool_result(
                tool_use.id,
                output,
                is_error,
            ));
        }
    }

    if wants_repo_analysis {
        if let Some(fallback_context) = build_repo_fallback_context(request) {
            if !fallback_context.trim().is_empty() {
                let fallback_request =
                    build_fallback_synthesis_request(request, &fallback_context);
                return call_runner(state, &fallback_request).await;
            }
        }
    }

    Err(std::io::Error::other(format!(
        "tool loop exceeded {MAX_TOOL_ROUNDS} rounds"
    ))
    .into())
}

fn compact_request_history(request: &MessageRequest) -> MessageRequest {
    if request.messages.len() <= MAX_HISTORY_MESSAGES_BEFORE_COMPACTION {
        return request.clone();
    }

    let mut compacted = request.clone();
    let split_at = request.messages.len().saturating_sub(KEEP_RECENT_MESSAGES);
    let older = &request.messages[..split_at];
    let recent = &request.messages[split_at..];

    let summary = summarize_messages_for_context(older);
    compacted.messages = Vec::new();

    if !summary.is_empty() {
        compacted.messages.push(InputMessage {
            role: "system".to_string(),
            content: vec![InputContentBlock::Text {
                text: format!("Conversation summary of earlier messages:\n{summary}"),
            }],
        });
    }

    compacted.messages.extend(recent.iter().cloned());
    compacted
}

fn summarize_messages_for_context(messages: &[InputMessage]) -> String {
    let mut out = String::new();
    let mut used = 0usize;

    for (idx, message) in messages.iter().enumerate() {
        let role = if message.role.eq_ignore_ascii_case("user") {
            "User"
        } else if message.role.eq_ignore_ascii_case("assistant") {
            "Assistant"
        } else {
            "Message"
        };

        let text = message_text_summary(message);
        if text.is_empty() {
            continue;
        }

        let entry = format!("{}. {}: {}\n", idx + 1, role, text);
        if used + entry.len() > MAX_SUMMARY_CHARS {
            break;
        }

        used += entry.len();
        out.push_str(&entry);
    }

    out
}

fn message_text_summary(message: &InputMessage) -> String {
    let mut text = String::new();

    for block in &message.content {
        match block {
            InputContentBlock::Text { text: block_text } => {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(block_text);
            }
            InputContentBlock::ToolUse { name, .. } => {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(&format!("[tool_use:{name}]"));
            }
            _ => {}
        }
    }

    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated = truncate_string_chars(&compact, MAX_MESSAGE_TEXT_CHARS);
    truncated.trim().to_string()
}

fn truncate_string_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    out.push('…');
    out
}

async fn run_single_file_rewrite(
    state: &AppState,
    request: &MessageRequest,
    file_path: &Path,
) -> Result<MessageResponse, Box<dyn std::error::Error + Send + Sync>> {
    let source = fs::read_to_string(file_path)?;
    let initial_request = build_file_fix_fallback_request(request, file_path)?;
    let initial_response = call_runner(state, &initial_request).await?;

    match assess_single_file_rewrite_response(&initial_response, &source) {
        RewriteAssessment::Complete => Ok(initial_response),
        RewriteAssessment::Incomplete { partial_code } => {
            continue_file_rewrite(state, request, file_path, &source, &partial_code).await
        }
        RewriteAssessment::Malformed(reason) => Err(std::io::Error::other(reason).into()),
    }
}

fn assess_single_file_rewrite_response(
    response: &MessageResponse,
    source: &str,
) -> RewriteAssessment {
    let text = collect_text_blocks(response).trim().to_string();

    if text == "DONE" {
        return RewriteAssessment::Malformed(
            "rewrite returned DONE before a complete file was assembled".to_string(),
        );
    }

    let Some(code) = extract_single_fenced_code_block(&text) else {
        return RewriteAssessment::Malformed(
            "rewrite response was not exactly one fenced code block".to_string(),
        );
    };

    if looks_rewrite_complete(&code, source) {
        RewriteAssessment::Complete
    } else {
        RewriteAssessment::Incomplete { partial_code: code }
    }
}

async fn continue_file_rewrite(
    state: &AppState,
    request: &MessageRequest,
    file_path: &Path,
    source: &str,
    initial_partial: &str,
) -> Result<MessageResponse, Box<dyn std::error::Error + Send + Sync>> {
    let lang = code_fence_lang(file_path);
    let mut assembled = initial_partial.to_string();

    for _ in 0..MAX_FILE_REWRITE_CONTINUATIONS {
        let continuation_request =
            build_file_rewrite_continuation_request(request, file_path, lang, source, &assembled);

        let response = call_runner(state, &continuation_request).await?;
        let text = collect_text_blocks(&response).trim().to_string();

        if text == "DONE" {
            if looks_rewrite_complete(&assembled, source) {
                return Ok(replace_response_content(response, lang, &assembled));
            }

            return Err(std::io::Error::other(
                "model ended continuation before the full file was complete",
            )
            .into());
        }

        let Some(next_chunk) = extract_single_fenced_code_block(&text) else {
            return Err(std::io::Error::other(
                "continuation response was not exactly one fenced code block",
            )
            .into());
        };

        if next_chunk.trim().is_empty() {
            return Err(std::io::Error::other(
                "continuation response did not provide additional code",
            )
            .into());
        }

        let stitched = stitch_code_continuation(&assembled, &next_chunk);
        if stitched == assembled {
            return Err(std::io::Error::other(
                "continuation response did not extend the existing partial file",
            )
            .into());
        }

        assembled = stitched;

        if looks_rewrite_complete(&assembled, source) {
            return Ok(replace_response_content(response, lang, &assembled));
        }
    }

    Err(std::io::Error::other(
        "full-file rewrite remained incomplete after bounded continuation attempts",
    )
    .into())
}

fn replace_response_content(
    mut response: MessageResponse,
    lang: &str,
    code: &str,
) -> MessageResponse {
    response.content = vec![OutputContentBlock::Text {
        text: format!("```{lang}\n{code}\n```"),
    }];
    response
}

fn looks_rewrite_complete(code: &str, source: &str) -> bool {
    let src_lines = source.lines().count();
    let out_lines = code.lines().count();
    let src_chars = source.chars().count();
    let out_chars = code.chars().count();

    let lines_ok = out_lines.saturating_mul(100) >= src_lines.saturating_mul(60);
    let chars_ok = out_chars.saturating_mul(100) >= src_chars.saturating_mul(50);

    lines_ok && chars_ok
}

fn collect_text_blocks(response: &MessageResponse) -> String {
    response
        .content
        .iter()
        .filter_map(|block| match block {
            OutputContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_single_fenced_code_block(text: &str) -> Option<String> {
    let trimmed = text.trim();

    if !trimmed.starts_with("```") || !trimmed.ends_with("```") {
        return None;
    }

    let first_newline = trimmed.find('\n')?;
    let closing = trimmed.rfind("```")?;

    if closing <= first_newline {
        return None;
    }

    let inner = &trimmed[first_newline + 1..closing];
    if inner.contains("```") {
        return None;
    }

    Some(inner.trim_end_matches('\n').to_string())
}

fn stitch_code_continuation(existing: &str, next_chunk: &str) -> String {
    if existing.trim().is_empty() {
        return next_chunk.to_string();
    }

    let existing_lines: Vec<&str> = existing.lines().collect();
    let next_lines: Vec<&str> = next_chunk.lines().collect();

    let max_overlap = existing_lines.len().min(next_lines.len()).min(80);

    for overlap in (1..=max_overlap).rev() {
        if existing_lines[existing_lines.len() - overlap..] == next_lines[..overlap] {
            let mut out = existing.to_string();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            if overlap < next_lines.len() {
                out.push_str(&next_lines[overlap..].join("\n"));
            }
            return out;
        }
    }

    let mut out = existing.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(next_chunk);
    out
}

fn last_n_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

async fn call_runner(
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

#[derive(Debug, Clone)]
struct PendingToolUse {
    id: String,
    name: String,
    input: serde_json::Value,
}

fn extract_tool_uses(response: &MessageResponse) -> Vec<PendingToolUse> {
    response
        .content
        .iter()
        .filter_map(|block| match block {
            OutputContentBlock::ToolUse { id, name, input } => Some(PendingToolUse {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn output_blocks_to_input_message(blocks: &[OutputContentBlock]) -> Option<InputMessage> {
    let content = blocks
        .iter()
        .filter_map(|block| match block {
            OutputContentBlock::Text { text } => {
                Some(InputContentBlock::Text { text: text.clone() })
            }
            OutputContentBlock::ToolUse { id, name, input } => {
                Some(InputContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                })
            }
            OutputContentBlock::Thinking { .. } | OutputContentBlock::RedactedThinking { .. } => {
                None
            }
        })
        .collect::<Vec<_>>();

    if content.is_empty() {
        None
    } else {
        Some(InputMessage {
            role: "assistant".to_string(),
            content,
        })
    }
}

fn filtered_tool_specs(tool_registry: &GlobalToolRegistry) -> Vec<ToolDefinition> {
    let mut definitions = tool_registry.definitions(None);
    definitions.retain(|tool| !matches!(tool.name.as_str(), "SendUserMessage" | "ToolSearch"));
    definitions
}

fn build_tooling_state() -> Result<(GlobalToolRegistry, HookRunner), String> {
    let cwd = env::current_dir().map_err(|error| error.to_string())?;
    let loader = ConfigLoader::default_for(&cwd);
    let runtime_config = loader.load().map_err(|error| error.to_string())?;
    let plugin_manager = build_plugin_manager(&cwd, &loader, &runtime_config);
    let plugin_registry = plugin_manager
        .plugin_registry()
        .map_err(|error| error.to_string())?;
    let tool_registry = GlobalToolRegistry::with_plugin_tools(
        plugin_registry
            .aggregated_tools()
            .map_err(|error| error.to_string())?,
    )?;
    let hook_runner =
        HookRunner::from_registry(&plugin_registry).map_err(|error| error.to_string())?;
    Ok((tool_registry, hook_runner))
}

fn build_plugin_manager(
    cwd: &Path,
    loader: &ConfigLoader,
    runtime_config: &RuntimeConfig,
) -> PluginManager {
    let plugin_settings = runtime_config.plugins();
    let mut plugin_config = PluginManagerConfig::new(loader.config_home().to_path_buf());
    plugin_config.enabled_plugins = plugin_settings.enabled_plugins().clone();
    plugin_config.external_dirs = plugin_settings
        .external_directories()
        .iter()
        .map(|path| resolve_plugin_path(cwd, loader.config_home(), path))
        .collect();
    plugin_config.install_root = plugin_settings
        .install_root()
        .map(|path| resolve_plugin_path(cwd, loader.config_home(), path));
    plugin_config.registry_path = plugin_settings
        .registry_path()
        .map(|path| resolve_plugin_path(cwd, loader.config_home(), path));
    plugin_config.bundled_root = plugin_settings
        .bundled_root()
        .map(|path| resolve_plugin_path(cwd, loader.config_home(), path));
    PluginManager::new(plugin_config)
}

fn resolve_plugin_path(cwd: &Path, config_home: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else if value.starts_with('.') {
        cwd.join(path)
    } else {
        config_home.join(path)
    }
}

fn maybe_enrich_request_with_local_sources(request: &MessageRequest) -> MessageRequest {
    let mut runner_request = request.clone();

    let Some(user_text) = latest_user_text(&runner_request) else {
        return runner_request;
    };

    let Some(target_path) = detect_existing_path(&user_text) else {
        return runner_request;
    };

    let Ok(summary) = build_source_summary(&target_path) else {
        return runner_request;
    };

    if summary.trim().is_empty() {
        return runner_request;
    }

    let analysis_instructions = format!(
        "The user referenced a real filesystem path.\n\
Do not guess about the repository contents.\n\
If tools are available and useful, use them conservatively.\n\
Do not request the entire repository at once.\n\
Prefer bounded inspection of entry points and major modules.\n\n\
LOCAL PATH SUMMARY\n\
==================\n\
{summary}"
    );

    match &mut runner_request.system {
        Some(system) => {
            if !system.contains("LOCAL PATH SUMMARY") {
                system.push_str("\n\n");
                system.push_str(&analysis_instructions);
            }
        }
        None => runner_request.system = Some(analysis_instructions),
    }

    runner_request
}

fn request_has_local_path(request: &MessageRequest) -> bool {
    latest_user_text(request)
        .and_then(|text| detect_existing_path(&text))
        .is_some()
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
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
            )
        })
        .to_string()
}

fn build_source_summary(path: &Path) -> Result<String, std::io::Error> {
    if path.is_file() {
        return Ok(format!("Requested file:\n{}", path.display()));
    }

    if !path.is_dir() {
        return Ok(String::new());
    }

    let mut files = collect_source_files(path)?;

    if files.is_empty() {
        return Ok(format!(
            "Directory exists but no supported source files were found:\n{}",
            path.display()
        ));
    }

    files.sort_by_key(|file| file_priority(file, path));

    let mut out = String::new();
    out.push_str(&format!("Directory:\n{}\n\n", path.display()));
    out.push_str("Candidate files:\n");

    for file in files.iter().take(MAX_SUMMARY_FILES) {
        out.push_str("- ");
        out.push_str(&display_relative(file, path));
        out.push('\n');
    }

    if files.len() > MAX_SUMMARY_FILES {
        out.push_str(&format!(
            "- ... {} more files omitted",
            files.len() - MAX_SUMMARY_FILES
        ));
    }

    Ok(out)
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
    matches!(
        name,
        "target"
            | ".git"
            | "node_modules"
            | "dist"
            | "build"
            | "vendor"
            | ".idea"
            | ".vscode"
            | "__pycache__"
    )
}

fn is_supported_source_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    if matches!(name, "Cargo.toml" | "Cargo.lock" | "README.md" | "readme.md") {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "rs"
            | "toml"
            | "md"
            | "txt"
            | "asm"
            | "s"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cc"
            | "cxx"
            | "json"
            | "yml"
            | "yaml"
    )
}

fn file_priority(path: &Path, root: &Path) -> (u8, String) {
    let rel = display_relative(path, root);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
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

fn truncate_tool_output(output: &str, max_chars: usize) -> String {
    if output.chars().count() <= max_chars {
        return output.to_string();
    }

    let mut truncated = String::new();
    for ch in output.chars().take(max_chars) {
        truncated.push(ch);
    }
    truncated.push_str("\n... [tool output truncated] ...");
    truncated
}

fn should_enable_tools(request: &MessageRequest) -> bool {
    let Some(text) = latest_user_text(request) else {
        return false;
    };

    let lower = text.to_ascii_lowercase();

    if detect_existing_path(&text).is_some() {
        return true;
    }

    let toolish_terms = [
        "read_file",
        "glob_search",
        "grep_search",
        "inspect",
        "bash",
        "powershell",
        "edit file",
        "write file",
        "open file",
        "search files",
        "find files",
        "command",
    ];

    toolish_terms.iter().any(|term| lower.contains(term))
}

fn looks_like_fake_tool_plan(response: &MessageResponse) -> bool {
    let combined = response
        .content
        .iter()
        .filter_map(|block| match block {
            OutputContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let lower = combined.to_ascii_lowercase();

    (lower.contains("\"name\": \"glob_search\"")
        || lower.contains("\"name\": \"grep_search\"")
        || lower.contains("\"name\": \"read_file\"")
        || lower.contains("let's start by")
        || lower.contains("i'll start by")
        || lower.contains("once we have identified these files"))
        && !combined.trim().is_empty()
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
