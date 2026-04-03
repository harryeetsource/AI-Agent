use std::collections::VecDeque;
use std::time::Duration;

use serde_json::{json, Value};
use crate::error::ApiError;
use crate::types::{
    ContentBlockDelta, ContentBlockDeltaEvent, ContentBlockStartEvent, ContentBlockStopEvent,
    MessageRequest, MessageResponse, MessageStartEvent, MessageStopEvent, OutputContentBlock,
    StreamEvent, Usage, MessageDelta, MessageDeltaEvent,
};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8080";
const DEFAULT_MAX_RETRIES: u32 = 2;
const DEFAULT_INITIAL_BACKOFF: Duration = Duration::from_millis(200);
const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(2);
const MESSAGES_PATH: &str = "/v1/messages";
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

impl Default for LocalModelClient {
    fn default() -> Self {
        Self::new()
    }
}

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
        let response = self.send_with_retry(request).await?;
        response.json::<MessageResponse>().await.map_err(ApiError::from)
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
        request: &MessageRequest,
    ) -> Result<reqwest::Response, ApiError> {
        let mut attempts = 0;

        loop {
            attempts += 1;
            let error = match self.send_raw_request(request).await {
                Ok(response) => match expect_success(response).await {
                    Ok(response) => return Ok(response),
                    Err(error) => error,
                },
                Err(error) => error,
            };

            if !error.is_retryable() {
                return Err(error);
            }

            if attempts > self.max_retries {
                return Err(ApiError::RetriesExhausted {
                    attempts,
                    last_error: Box::new(error),
                });
            }

            tokio::time::sleep(self.backoff_for_attempt(attempts)?).await;
        }
    }

    async fn send_raw_request(
        &self,
        request: &MessageRequest,
    ) -> Result<reqwest::Response, ApiError> {
        let request_url = format!("{}{}", self.base_url.trim_end_matches('/'), MESSAGES_PATH);
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
