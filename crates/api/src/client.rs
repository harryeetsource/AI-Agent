use std::collections::VecDeque;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::ApiError;
use crate::types::{
    ContentBlockDelta, ContentBlockDeltaEvent, ContentBlockStartEvent, ContentBlockStopEvent,
    InputContentBlock, InputMessage, MessageDelta, MessageDeltaEvent, MessageRequest,
    MessageResponse, MessageStartEvent, MessageStopEvent, OutputContentBlock, StreamEvent,
    ToolChoice, ToolDefinition, ToolResultContentBlock, Usage,
};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_MAX_RETRIES: u32 = 2;
const DEFAULT_INITIAL_BACKOFF: Duration = Duration::from_millis(200);
const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(2);
const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const TEXT_STREAM_CHUNK_BYTES: usize = 96;

#[derive(Debug, Clone)]
pub struct LocalModelClient {
    http: reqwest::Client,
    base_url: String,
    max_retries: u32,
    initial_backoff: Duration,
    max_backoff: Duration,
}

pub type AnthropicClient = LocalModelClient;

impl LocalModelClient {
    #[must_use]
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: read_base_url(),
            max_retries: DEFAULT_MAX_RETRIES,
            initial_backoff: DEFAULT_INITIAL_BACKOFF,
            max_backoff: DEFAULT_MAX_BACKOFF,
        }
    }

    pub fn from_env() -> Result<Self, ApiError> {
        Ok(Self::new())
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = normalize_base_url(base_url.into());
        self
    }

    #[must_use]
    pub fn with_retry_policy(
        mut self,
        max_retries: u32,
        initial_backoff: Duration,
        max_backoff: Duration,
    ) -> Self {
        self.max_retries = max_retries;
        self.initial_backoff = initial_backoff;
        self.max_backoff = max_backoff;
        self
    }

    pub async fn send_message(
        &self,
        request: &MessageRequest,
    ) -> Result<MessageResponse, ApiError> {
        let payload = build_local_chat_request(request);
        let response = self.send_with_retry(&payload).await?;
        let body = response.json::<LocalChatCompletionResponse>().await?;
        completion_to_message_response(request, body)
    }

    pub async fn stream_message(
        &self,
        request: &MessageRequest,
    ) -> Result<MessageStream, ApiError> {
        let response = self.send_message(request).await?;
        Ok(MessageStream::from_response(response))
    }

    async fn send_with_retry(
        &self,
        request: &LocalChatCompletionRequest,
    ) -> Result<reqwest::Response, ApiError> {
        let mut attempts = 0;
        let mut last_error: Option<ApiError> = None;

        loop {
            attempts += 1;
            match self.send_raw_request(request).await {
                Ok(response) => match expect_success(response).await {
                    Ok(response) => return Ok(response),
                    Err(error) if error.is_retryable() && attempts <= self.max_retries + 1 => {
                        last_error = Some(error);
                    }
                    Err(error) => return Err(error),
                },
                Err(error) if error.is_retryable() && attempts <= self.max_retries + 1 => {
                    last_error = Some(error);
                }
                Err(error) => return Err(error),
            }

            if attempts > self.max_retries {
                break;
            }

            tokio::time::sleep(self.backoff_for_attempt(attempts)?).await;
        }

        Err(ApiError::RetriesExhausted {
            attempts,
            last_error: Box::new(last_error.unwrap_or_else(|| {
                ApiError::Auth("local model request failed without a captured root cause".to_string())
            })),
        })
    }

    async fn send_raw_request(
        &self,
        request: &LocalChatCompletionRequest,
    ) -> Result<reqwest::Response, ApiError> {
        let request_url = format!("{}{}", self.base_url.trim_end_matches('/'), CHAT_COMPLETIONS_PATH);
        self.http
            .post(request_url)
            .header("content-type", "application/json")
            .json(request)
            .send()
            .await
            .map_err(ApiError::from)
    }

    fn backoff_for_attempt(&self, attempt: u32) -> Result<Duration, ApiError> {
        let Some(multiplier) = 1_u32.checked_shl(attempt.saturating_sub(1)) else {
            return Err(ApiError::BackoffOverflow {
                attempt,
                base_delay: self.initial_backoff,
            });
        };
        Ok(self
            .initial_backoff
            .checked_mul(multiplier)
            .map_or(self.max_backoff, |delay| delay.min(self.max_backoff)))
    }
}

#[must_use]
pub fn read_base_url() -> String {
    let raw = std::env::var("CLAW_LOCAL_BASE_URL")
        .or_else(|_| std::env::var("OLLAMA_HOST"))
        .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
    normalize_base_url(raw)
}

fn normalize_base_url(raw: String) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.trim_end_matches('/').to_string()
    } else {
        format!("http://{}", trimmed.trim_end_matches('/'))
    }
}

#[derive(Debug)]
pub struct MessageStream {
    request_id: Option<String>,
    pending: VecDeque<StreamEvent>,
}

impl MessageStream {
    fn from_response(response: MessageResponse) -> Self {
        let request_id = response.request_id.clone();
        let mut pending = VecDeque::new();

        pending.push_back(StreamEvent::MessageStart(MessageStartEvent {
            message: MessageResponse {
                id: response.id.clone(),
                kind: response.kind.clone(),
                role: response.role.clone(),
                content: Vec::new(),
                model: response.model.clone(),
                stop_reason: None,
                stop_sequence: None,
                usage: Usage {
                    input_tokens: response.usage.input_tokens,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                    output_tokens: 0,
                },
                request_id: response.request_id.clone(),
            },
        }));

        for (index, block) in response.content.iter().cloned().enumerate() {
            let index = index as u32;
            match block {
                OutputContentBlock::Text { text } => {
                    pending.push_back(StreamEvent::ContentBlockStart(ContentBlockStartEvent {
                        index,
                        content_block: OutputContentBlock::Text {
                            text: String::new(),
                        },
                    }));
                    for chunk in split_text_chunks(&text) {
                        pending.push_back(StreamEvent::ContentBlockDelta(ContentBlockDeltaEvent {
                            index,
                            delta: ContentBlockDelta::TextDelta { text: chunk },
                        }));
                    }
                    pending.push_back(StreamEvent::ContentBlockStop(ContentBlockStopEvent {
                        index,
                    }));
                }
                OutputContentBlock::ToolUse { id, name, input } => {
                    pending.push_back(StreamEvent::ContentBlockStart(ContentBlockStartEvent {
                        index,
                        content_block: OutputContentBlock::ToolUse {
                            id,
                            name,
                            input: json!({}),
                        },
                    }));
                    pending.push_back(StreamEvent::ContentBlockDelta(ContentBlockDeltaEvent {
                        index,
                        delta: ContentBlockDelta::InputJsonDelta {
                            partial_json: input.to_string(),
                        },
                    }));
                    pending.push_back(StreamEvent::ContentBlockStop(ContentBlockStopEvent {
                        index,
                    }));
                }
                OutputContentBlock::Thinking { .. }
                | OutputContentBlock::RedactedThinking { .. } => {}
            }
        }

        pending.push_back(StreamEvent::MessageDelta(MessageDeltaEvent {
            delta: MessageDelta {
                stop_reason: response.stop_reason.clone(),
                stop_sequence: response.stop_sequence.clone(),
            },
            usage: response.usage,
        }));
        pending.push_back(StreamEvent::MessageStop(MessageStopEvent {}));

        Self { request_id, pending }
    }

    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    pub async fn next_event(&mut self) -> Result<Option<StreamEvent>, ApiError> {
        Ok(self.pending.pop_front())
    }
}

fn split_text_chunks(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if current.len() >= TEXT_STREAM_CHUNK_BYTES {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[derive(Debug, Serialize)]
struct LocalChatCompletionRequest {
    model: String,
    messages: Vec<LocalChatMessage>,
    max_completion_tokens: u32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<LocalToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<Value>,
}

#[derive(Debug, Serialize)]
struct LocalChatMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<LocalToolCall>>,
}

#[derive(Debug, Serialize)]
struct LocalToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: LocalToolCallFunction,
}

#[derive(Debug, Serialize)]
struct LocalToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct LocalToolDefinition {
    #[serde(rename = "type")]
    kind: String,
    function: LocalToolFunction,
}

#[derive(Debug, Serialize)]
struct LocalToolFunction {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    parameters: Value,
}

#[derive(Debug, Deserialize)]
struct LocalChatCompletionResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<LocalChoice>,
    #[serde(default)]
    usage: Option<LocalUsage>,
}

#[derive(Debug, Deserialize)]
struct LocalChoice {
    message: LocalAssistantMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalAssistantMessage {
    #[serde(default)]
    content: Option<Value>,
    #[serde(default)]
    tool_calls: Vec<LocalToolCallResponse>,
}

#[derive(Debug, Deserialize)]
struct LocalToolCallResponse {
    #[serde(default)]
    id: Option<String>,
    function: LocalToolCallFunctionResponse,
}

#[derive(Debug, Deserialize)]
struct LocalToolCallFunctionResponse {
    name: String,
    #[serde(default)]
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct LocalUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

fn build_local_chat_request(request: &MessageRequest) -> LocalChatCompletionRequest {
    let mut messages = Vec::new();
    if let Some(system) = request.system.as_ref().filter(|value| !value.trim().is_empty()) {
        messages.push(LocalChatMessage {
            role: "system".to_string(),
            content: Some(Value::String(system.clone())),
            tool_call_id: None,
            tool_calls: None,
        });
    }
    for message in &request.messages {
        extend_local_messages(&mut messages, message);
    }

    LocalChatCompletionRequest {
        model: request.model.clone(),
        messages,
        max_completion_tokens: request.max_tokens,
        stream: false,
        tools: request.tools.as_ref().map(convert_tool_definitions),
        tool_choice: request.tool_choice.as_ref().map(convert_tool_choice),
    }
}

fn extend_local_messages(target: &mut Vec<LocalChatMessage>, message: &InputMessage) {
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for block in &message.content {
        match block {
            InputContentBlock::Text { text } => {
                if !text.is_empty() {
                    text_parts.push(text.clone());
                }
            }
            InputContentBlock::ToolUse { id, name, input } => {
                tool_calls.push(LocalToolCall {
                    id: id.clone(),
                    kind: "function".to_string(),
                    function: LocalToolCallFunction {
                        name: name.clone(),
                        arguments: canonical_json_string(input),
                    },
                });
            }
            InputContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                target.push(LocalChatMessage {
                    role: "tool".to_string(),
                    content: Some(Value::String(render_tool_result_content(content))),
                    tool_call_id: Some(tool_use_id.clone()),
                    tool_calls: None,
                });
            }
        }
    }

    if text_parts.is_empty() && tool_calls.is_empty() {
        return;
    }

    let role = if message.role.eq_ignore_ascii_case("assistant") {
        "assistant"
    } else {
        "user"
    };
    target.push(LocalChatMessage {
        role: role.to_string(),
        content: (!text_parts.is_empty()).then(|| Value::String(text_parts.join("\n\n"))),
        tool_call_id: None,
        tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
    });
}

fn convert_tool_definitions(tools: &Vec<ToolDefinition>) -> Vec<LocalToolDefinition> {
    tools
        .iter()
        .map(|tool| LocalToolDefinition {
            kind: "function".to_string(),
            function: LocalToolFunction {
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.input_schema.clone(),
            },
        })
        .collect()
}

fn convert_tool_choice(choice: &ToolChoice) -> Value {
    match choice {
        ToolChoice::Auto => Value::String("auto".to_string()),
        ToolChoice::Any => Value::String("required".to_string()),
        ToolChoice::Tool { name } => json!({
            "type": "function",
            "function": { "name": name },
        }),
    }
}

fn render_tool_result_content(content: &[ToolResultContentBlock]) -> String {
    let parts = content
        .iter()
        .map(|block| match block {
            ToolResultContentBlock::Text { text } => text.clone(),
            ToolResultContentBlock::Json { value } => canonical_json_string(value),
        })
        .collect::<Vec<_>>();
    parts.join("\n")
}

fn completion_to_message_response(
    request: &MessageRequest,
    response: LocalChatCompletionResponse,
) -> Result<MessageResponse, ApiError> {
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::Auth("local model returned no completion choices".to_string()))?;

    let mut content = Vec::new();
    let assistant_text = extract_assistant_text(choice.message.content.as_ref());
    if !assistant_text.is_empty() {
        content.push(OutputContentBlock::Text { text: assistant_text });
    }

    for (index, tool_call) in choice.message.tool_calls.into_iter().enumerate() {
        let id = tool_call
            .id
            .unwrap_or_else(|| format!("tool_call_{index}"));
        let input = parse_tool_arguments(&tool_call.function.arguments);
        content.push(OutputContentBlock::ToolUse {
            id,
            name: tool_call.function.name,
            input,
        });
    }

    let usage = response.usage.unwrap_or(LocalUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
    });

    Ok(MessageResponse {
        id: response.id.unwrap_or_else(|| "local-msg".to_string()),
        kind: "message".to_string(),
        role: "assistant".to_string(),
        content,
        model: response.model.unwrap_or_else(|| request.model.clone()),
        stop_reason: choice.finish_reason,
        stop_sequence: None,
        usage: Usage {
            input_tokens: usage.prompt_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            output_tokens: usage.completion_tokens,
        },
        request_id: None,
    })
}

fn extract_assistant_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part {
                Value::Object(object) if object.get("type") == Some(&Value::String("text".to_string())) => {
                    object.get("text").and_then(Value::as_str).map(ToOwned::to_owned)
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn parse_tool_arguments(arguments: &str) -> Value {
    serde_json::from_str(arguments).unwrap_or_else(|_| json!({ "raw": arguments }))
}

fn canonical_json_string(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

async fn expect_success(response: reqwest::Response) -> Result<reqwest::Response, ApiError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let body = response.text().await.unwrap_or_else(|_| String::new());
    let parsed_message = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|value| {
            value.get("error")
                .and_then(|error| error.get("message").or(Some(error)))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| value.get("message").and_then(Value::as_str).map(ToOwned::to_owned))
        });
    let retryable = is_retryable_status(status);

    Err(ApiError::Api {
        status,
        error_type: Some("local_backend".to_string()),
        message: parsed_message,
        body,
        retryable,
    })
}

const fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 409 | 425 | 429 | 500 | 502 | 503 | 504)
}
