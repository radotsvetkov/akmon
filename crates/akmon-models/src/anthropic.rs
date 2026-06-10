//! [Anthropic](https://www.anthropic.com/) Messages API backend (streaming SSE).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use akmon_core::Secret;
use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::config::CompletionConfig;
use crate::error::ModelError;
use crate::message::{Message, MessageRole};
use crate::stream::{CompletionStream, ModelToolCall, StopReason, StreamEvent, UsageReport};
use crate::tool_def::ToolDefinition;
use crate::{AttemptObserver, LlmProvider};
use akmon_journal::{AttemptRecord, AttemptStatus, Hash, HashAlgorithm};

/// Default Anthropic Messages API host (no trailing slash).
pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";

/// Default model id for [`AnthropicBackend::new`] (dated Claude Haiku 4.5 snapshot; required for stable behavior and prompt caching).
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-haiku-4-5-20251001";

/// Default advertised context window for Claude 3.5 Haiku-class models.
pub const DEFAULT_ANTHROPIC_CONTEXT_WINDOW: usize = 200_000;

struct AnthropicInner {
    api_key: Secret<String>,
    model: String,
    base_url: String,
    context_window: usize,
    client: reqwest::Client,
}

/// Client for Anthropic's `/v1/messages` endpoint with SSE streaming.
///
/// API credentials are held in [`Secret`]; this type does **not** implement [`std::fmt::Debug`]
/// so secrets cannot be logged accidentally via `{:?}`.
pub struct AnthropicBackend {
    inner: Arc<AnthropicInner>,
    attempt_observer: Arc<RwLock<Option<Arc<dyn AttemptObserver>>>>,
}

impl AnthropicBackend {
    /// Builds a backend with [`DEFAULT_ANTHROPIC_BASE_URL`] and [`DEFAULT_ANTHROPIC_CONTEXT_WINDOW`].
    pub fn new(api_key: Secret<String>, model: impl Into<String>) -> Self {
        Self::with_base_url(api_key, model, DEFAULT_ANTHROPIC_BASE_URL)
    }

    /// Same as [`AnthropicBackend::new`] but sets the API base URL (trailing slashes stripped).
    pub fn with_base_url(
        api_key: Secret<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        let mut base_url = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        let model_str = model.into();
        let model = if model_str.is_empty() {
            DEFAULT_ANTHROPIC_MODEL.to_string()
        } else {
            model_str
        };
        Self {
            inner: Arc::new(AnthropicInner {
                api_key,
                model,
                base_url,
                context_window: DEFAULT_ANTHROPIC_CONTEXT_WINDOW,
                client: build_anthropic_http_client(),
            }),
            attempt_observer: Arc::new(RwLock::new(None)),
        }
    }

    /// Exposes the configured API base (without trailing slash).
    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    /// Exposes the configured model id.
    pub fn model(&self) -> &str {
        &self.inner.model
    }

    fn observer(&self) -> Option<Arc<dyn AttemptObserver>> {
        self.attempt_observer
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

fn build_anthropic_http_client() -> reqwest::Client {
    crate::http_client::build_http_client(10, 300).unwrap_or_else(|e| {
        panic!("anthropic HTTP client: {e}");
    })
}

/// Joins all [`MessageRole::System`] bodies in order with `\n\n`.
///
/// Matches the `text` field of the cached `system` block sent by [`AnthropicBackend`].
pub fn anthropic_system_block_text(messages: &[Message]) -> String {
    anthropic_system_and_rest(messages).0
}

/// Concatenates all [`MessageRole::System`] bodies in order with `\n\n`; returns non-system
/// messages as borrowed refs in order.
pub(crate) fn anthropic_system_and_rest(messages: &[Message]) -> (String, Vec<&Message>) {
    let mut system_parts: Vec<&str> = Vec::new();
    let mut rest: Vec<&Message> = Vec::new();
    for m in messages {
        if m.role == MessageRole::System {
            system_parts.push(m.content.as_str());
        } else {
            rest.push(m);
        }
    }
    (system_parts.join("\n\n"), rest)
}

/// Splits leading [`MessageRole::System`] messages into separate API `system` content blocks.
pub(crate) fn anthropic_leading_system_blocks(
    messages: &[Message],
) -> (Vec<String>, Vec<&Message>) {
    let mut blocks = Vec::new();
    let mut i = 0usize;
    while i < messages.len() && messages[i].role == MessageRole::System {
        blocks.push(messages[i].content.clone());
        i += 1;
    }
    (blocks, messages[i..].iter().collect())
}

fn tool_message_to_result_block(m: &Message) -> Option<Value> {
    let v: Value = serde_json::from_str(&m.content).ok()?;
    let id = v.get("tool_call_id").and_then(|x| x.as_str())?;
    let output = v.get("output").cloned().unwrap_or(json!({}));
    let content_str = serde_json::to_string(&output).ok()?;
    Some(json!({
        "type": "tool_result",
        "tool_use_id": id,
        "content": content_str,
    }))
}

fn non_tool_message_to_anthropic(m: &Message) -> Option<Value> {
    match m.role {
        MessageRole::User => Some(json!({
            "role": "user",
            "content": m.content,
        })),
        MessageRole::Assistant => {
            if let Ok(v) = serde_json::from_str::<Value>(&m.content)
                && let Some(arr) = v.get("tool_calls").and_then(|a| a.as_array())
                && !arr.is_empty()
            {
                let mut blocks: Vec<Value> = Vec::new();
                if let Some(t) = v.get("text").and_then(|x| x.as_str())
                    && !t.trim().is_empty()
                {
                    blocks.push(json!({"type": "text", "text": t}));
                }
                for tc in arr {
                    let id = tc.get("id").and_then(|x| x.as_str()).unwrap_or("");
                    let name = tc.get("name").and_then(|x| x.as_str()).unwrap_or("");
                    let input = tc.get("arguments").cloned().unwrap_or(json!({}));
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input,
                    }));
                }
                return Some(json!({ "role": "assistant", "content": blocks }));
            }
            Some(json!({
                "role": "assistant",
                "content": m.content,
            }))
        }
        _ => None,
    }
}

pub(crate) fn build_anthropic_api_messages(msgs: &[&Message]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut i: usize = 0;
    while i < msgs.len() {
        let m = msgs[i];
        if m.role == MessageRole::Tool {
            let mut blocks: Vec<Value> = Vec::new();
            while i < msgs.len() && msgs[i].role == MessageRole::Tool {
                if let Some(b) = tool_message_to_result_block(msgs[i]) {
                    blocks.push(b);
                }
                i += 1;
            }
            if !blocks.is_empty() {
                out.push(json!({ "role": "user", "content": blocks }));
            }
            continue;
        }
        if let Some(v) = non_tool_message_to_anthropic(m) {
            out.push(v);
        }
        i += 1;
    }
    out
}

pub(crate) fn anthropic_tools_from_config(config: &CompletionConfig) -> Vec<Value> {
    config
        .tools
        .iter()
        .map(|t: &ToolDefinition| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            })
        })
        .collect()
}

fn u32_from_json_token_field(v: Option<&Value>) -> u32 {
    v.and_then(|x| x.as_u64())
        .map(|n| (n.min(u64::from(u32::MAX))) as u32)
        .unwrap_or(0)
}

fn build_request_json(
    model: &str,
    system_blocks: &[String],
    messages: Vec<Value>,
    tools: Vec<Value>,
    config: &CompletionConfig,
) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("model".to_string(), json!(model));
    map.insert("max_tokens".to_string(), json!(config.max_tokens));
    map.insert("stream".to_string(), json!(true));
    map.insert("temperature".to_string(), json!(config.temperature));
    // Automatic prompt caching (documented): one breakpoint at the last cacheable prefix.
    map.insert("cache_control".to_string(), json!({ "type": "ephemeral" }));
    if !system_blocks.is_empty() {
        let arr: Vec<Value> = system_blocks
            .iter()
            .map(|text| {
                json!({
                    "type": "text",
                    "text": text,
                })
            })
            .collect();
        map.insert("system".to_string(), Value::Array(arr));
    }
    map.insert("messages".to_string(), Value::Array(messages));
    if !tools.is_empty() {
        map.insert("tools".to_string(), Value::Array(tools));
    }
    Value::Object(map)
}

/// JSON body for **Amazon Bedrock** `InvokeModelWithResponseStream` (Claude Messages-shaped payload).
pub(crate) fn build_bedrock_anthropic_invoke_json(
    messages: &[Message],
    config: &CompletionConfig,
) -> Value {
    let (system, rest) = anthropic_system_and_rest(messages);
    let api_messages = build_anthropic_api_messages(&rest);
    let mut tools = anthropic_tools_from_config(config);
    for t in &mut tools {
        if let Some(obj) = t.as_object_mut() {
            obj.remove("cache_control");
        }
    }
    let mut map = serde_json::Map::new();
    map.insert("anthropic_version".to_string(), json!("bedrock-2023-05-31"));
    map.insert("max_tokens".to_string(), json!(config.max_tokens));
    map.insert("temperature".to_string(), json!(config.temperature));
    if !system.is_empty() {
        map.insert(
            "system".to_string(),
            json!([{ "type": "text", "text": system }]),
        );
    }
    map.insert("messages".to_string(), Value::Array(api_messages));
    if !tools.is_empty() {
        map.insert("tools".to_string(), Value::Array(tools));
    }
    Value::Object(map)
}

fn map_anthropic_http_status(status: StatusCode, body: &str, headers: &HeaderMap) -> ModelError {
    match status.as_u16() {
        401 => ModelError::AuthError,
        429 => ModelError::RateLimited {
            retry_after_secs: headers
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok()),
        },
        529 => ModelError::BackendUnavailable {
            message: format!("HTTP {status}: {tb}", tb = truncate_body(body)),
        },
        503 => ModelError::BackendUnavailable {
            message: format!("HTTP {status}"),
        },
        _ => ModelError::BackendUnavailable {
            message: format!("HTTP {status}: {tb}", tb = truncate_body(body)),
        },
    }
}

fn truncate_body(s: &str) -> String {
    const MAX: usize = 512;
    if s.len() <= MAX {
        s.to_string()
    } else {
        format!(
            "{prefix}…",
            prefix = crate::text::truncate_at_char_boundary(s, MAX)
        )
    }
}

fn map_reqwest_send_error(e: reqwest::Error) -> ModelError {
    if e.is_connect() {
        return ModelError::BackendUnavailable {
            message: e.to_string(),
        };
    }
    if e.is_timeout() {
        return ModelError::FirstTokenTimeout;
    }
    ModelError::BackendUnavailable {
        message: e.to_string(),
    }
}

fn map_reqwest_stream_error(e: reqwest::Error) -> ModelError {
    ModelError::StreamInterrupted {
        message: e.to_string(),
    }
}

#[derive(Debug, Default, serde::Serialize)]
struct AnthropicSynthesizedResponse {
    text: String,
    tool_calls: Vec<ModelToolCall>,
    stop_reason: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct AnthropicStreamEventChunk {
    #[serde(rename = "kind")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ModelToolCall>>,
}

fn canonical_cbor_bytes<T: serde::Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, ModelError> {
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(value, &mut bytes).map_err(|err| {
        ModelError::BackendUnavailable {
            message: format!("canonical cbor encode failed: {err}"),
        }
    })?;
    Ok(bytes)
}

fn sentinel_hash() -> Hash {
    Hash::from_bytes(HashAlgorithm::Sha256, [0_u8; 32])
}

fn stop_reason_label(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::ToolUse => "tool_use",
    }
}

fn map_model_error_to_attempt_status(err: &ModelError) -> AttemptStatus {
    match err {
        ModelError::RateLimited { .. } => AttemptStatus::RateLimited,
        ModelError::FirstTokenTimeout | ModelError::StreamInterrupted { .. } => {
            AttemptStatus::NetworkError
        }
        ModelError::BackendUnavailable { .. } => AttemptStatus::ServerError,
        ModelError::AuthError
        | ModelError::ContextWindowExceeded
        | ModelError::ModelNotFound { .. } => AttemptStatus::ClientError,
    }
}

fn map_http_status_to_attempt_status(status: StatusCode) -> AttemptStatus {
    match status.as_u16() {
        429 => AttemptStatus::RateLimited,
        529 => AttemptStatus::ServerError,
        500..=599 => AttemptStatus::ServerError,
        400..=499 => AttemptStatus::ClientError,
        _ => AttemptStatus::Other(format!("unexpected HTTP status {status}")),
    }
}

fn stream_event_chunk(event: &StreamEvent) -> AnthropicStreamEventChunk {
    match event {
        StreamEvent::ProviderReady { provider, model } => AnthropicStreamEventChunk {
            kind: "provider_ready",
            provider: Some(provider.clone()),
            model: Some(model.clone()),
            message: None,
            text: None,
            stop_reason: None,
            tool_calls: None,
        },
        StreamEvent::StatusHint { message } => AnthropicStreamEventChunk {
            kind: "status_hint",
            provider: None,
            model: None,
            message: Some(message.clone()),
            text: None,
            stop_reason: None,
            tool_calls: None,
        },
        StreamEvent::TextDelta { text } => AnthropicStreamEventChunk {
            kind: "text_delta",
            provider: None,
            model: None,
            message: None,
            text: Some(text.clone()),
            stop_reason: None,
            tool_calls: None,
        },
        StreamEvent::Done {
            stop_reason,
            tool_calls,
        } => AnthropicStreamEventChunk {
            kind: "done",
            provider: None,
            model: None,
            message: None,
            text: None,
            stop_reason: Some(stop_reason_label(stop_reason).to_owned()),
            tool_calls: Some(tool_calls.clone()),
        },
        StreamEvent::UsageReport(_) => AnthropicStreamEventChunk {
            kind: "usage_report",
            provider: None,
            model: None,
            message: None,
            text: None,
            stop_reason: None,
            tool_calls: None,
        },
        StreamEvent::Error { error } => AnthropicStreamEventChunk {
            kind: "error",
            provider: None,
            model: None,
            message: Some(error.to_string()),
            text: None,
            stop_reason: None,
            tool_calls: None,
        },
    }
}

fn accumulate_response(event: &StreamEvent, response: &mut AnthropicSynthesizedResponse) {
    match event {
        StreamEvent::TextDelta { text } => response.text.push_str(text),
        StreamEvent::Done {
            stop_reason,
            tool_calls,
        } => {
            response.stop_reason = Some(stop_reason_label(stop_reason).to_owned());
            response.tool_calls = tool_calls.clone();
        }
        _ => {}
    }
}

/// Parses one SSE event block (lines between blank lines).
fn sse_block_to_json(block: &str) -> Result<Option<Value>, ModelError> {
    let mut data_payload: Option<String> = None;
    for line in block.lines() {
        let line = line.trim_end();
        if let Some(rest) = line.strip_prefix("data:") {
            let payload = rest.trim();
            if payload.is_empty() {
                continue;
            }
            data_payload = Some(match data_payload.take() {
                Some(prev) => format!("{prev}\n{payload}"),
                None => payload.to_string(),
            });
        }
    }
    let Some(s) = data_payload else {
        return Ok(None);
    };
    if s == "[DONE]" {
        return Ok(None);
    }
    serde_json::from_str::<Value>(&s)
        .map(Some)
        .map_err(|e| ModelError::StreamInterrupted {
            message: format!("invalid SSE JSON: {e}"),
        })
}

/// Incremental tool-use state for Anthropic-style streaming JSON (shared with Bedrock Claude streaming).
pub(crate) struct ToolAccum {
    id: String,
    name: String,
    partial_json: String,
}

async fn read_next_sse_event<S>(
    buf: &mut String,
    stream: &mut S,
) -> Result<Option<String>, ModelError>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
    loop {
        if let Some(pos) = buf.find("\n\n") {
            let block = buf[..pos].to_string();
            buf.drain(..pos + 2);
            if block.trim().is_empty() {
                continue;
            }
            return Ok(Some(block));
        }
        match stream.next().await {
            None => {
                let rest = std::mem::take(buf);
                let trimmed = rest.trim();
                if trimmed.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(trimmed.to_string()));
            }
            Some(Err(e)) => return Err(map_reqwest_stream_error(e)),
            Some(Ok(bytes)) => buf.push_str(&String::from_utf8_lossy(&bytes)),
        }
    }
}

/// Applies one Anthropic/Bedrock Claude streaming protocol JSON object (`type` discriminant).
pub(crate) fn apply_anthropic_sse_json(
    v: &Value,
    tool_builds: &mut BTreeMap<usize, ToolAccum>,
    finished_tools: &mut BTreeMap<usize, ModelToolCall>,
    usage_acc: &mut Option<UsageReport>,
) -> Result<Vec<StreamEvent>, ModelError> {
    let mut out: Vec<StreamEvent> = Vec::new();
    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    match ty {
        "message_start" => {
            if let Some(msg) = v.get("message")
                && let Some(u) = msg.get("usage")
            {
                *usage_acc = Some(UsageReport {
                    input_tokens: u32_from_json_token_field(u.get("input_tokens")),
                    output_tokens: u32_from_json_token_field(u.get("output_tokens")),
                    cache_creation_tokens: u32_from_json_token_field(
                        u.get("cache_creation_input_tokens"),
                    ),
                    cache_read_tokens: u32_from_json_token_field(u.get("cache_read_input_tokens")),
                });
            }
        }
        "error" => {
            let msg = v
                .pointer("/error/message")
                .and_then(|x| x.as_str())
                .unwrap_or("anthropic error");
            return Err(ModelError::BackendUnavailable {
                message: msg.to_string(),
            });
        }
        "content_block_delta" => {
            let index = v.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
            if let Some(delta) = v.get("delta") {
                let dt = delta.get("type").and_then(|x| x.as_str()).unwrap_or("");
                if dt == "text_delta" {
                    if let Some(text) = delta.get("text").and_then(|x| x.as_str())
                        && !text.is_empty()
                    {
                        out.push(StreamEvent::TextDelta {
                            text: text.to_string(),
                        });
                    }
                } else if dt == "input_json_delta"
                    && let Some(p) = delta.get("partial_json").and_then(|x| x.as_str())
                    && let Some(acc) = tool_builds.get_mut(&index)
                {
                    acc.partial_json.push_str(p);
                }
            }
        }
        "content_block_start" => {
            let index = v.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
            if let Some(cb) = v.get("content_block") {
                let cbt = cb.get("type").and_then(|x| x.as_str()).unwrap_or("");
                if cbt == "tool_use" {
                    let id = cb
                        .get("id")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = cb
                        .get("name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    tool_builds.insert(
                        index,
                        ToolAccum {
                            id,
                            name,
                            partial_json: String::new(),
                        },
                    );
                }
            }
        }
        "content_block_stop" => {
            let index = v.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
            if let Some(acc) = tool_builds.remove(&index) {
                let arguments = if acc.partial_json.trim().is_empty() {
                    json!({})
                } else {
                    match serde_json::from_str::<Value>(&acc.partial_json) {
                        Ok(obj) => obj,
                        Err(_) => json!({}),
                    }
                };
                finished_tools.insert(
                    index,
                    ModelToolCall {
                        id: acc.id,
                        name: acc.name,
                        arguments,
                    },
                );
            }
        }
        "message_delta" => {
            if let Some(u) = v.get("usage")
                && let Some(acc) = usage_acc.as_mut()
            {
                if u.get("input_tokens").is_some() {
                    acc.input_tokens = u32_from_json_token_field(u.get("input_tokens"));
                }
                if u.get("output_tokens").is_some() {
                    acc.output_tokens = u32_from_json_token_field(u.get("output_tokens"));
                }
                if u.get("cache_creation_input_tokens").is_some() {
                    acc.cache_creation_tokens =
                        u32_from_json_token_field(u.get("cache_creation_input_tokens"));
                }
                if u.get("cache_read_input_tokens").is_some() {
                    acc.cache_read_tokens =
                        u32_from_json_token_field(u.get("cache_read_input_tokens"));
                }
            }
            if let Some(delta) = v.get("delta")
                && let Some(sr) = delta.get("stop_reason").and_then(|x| x.as_str())
            {
                let stop = match sr {
                    "end_turn" => StopReason::EndTurn,
                    "tool_use" => StopReason::ToolUse,
                    "max_tokens" => StopReason::MaxTokens,
                    _ => StopReason::EndTurn,
                };
                let tool_calls: Vec<ModelToolCall> = finished_tools.values().cloned().collect();
                if let Some(snap) = usage_acc.as_ref() {
                    out.push(StreamEvent::UsageReport(snap.clone()));
                }
                out.push(StreamEvent::Done {
                    stop_reason: stop,
                    tool_calls,
                });
            }
        }
        _ => {}
    }
    Ok(out)
}

async fn run_anthropic_stream(
    inner: Arc<AnthropicInner>,
    mut body: Value,
    config: CompletionConfig,
    observer: Option<Arc<dyn AttemptObserver>>,
    tx: mpsc::Sender<Result<StreamEvent, ModelError>>,
) {
    let url = format!("{base}/v1/messages", base = inner.base_url);

    const RETRY_MAX: u32 = 5;
    const RETRY_BASE_SECS: u64 = 1;
    const RETRY_MULTIPLIER: u64 = 2;
    const RETRY_CAP_SECS: u64 = 60;

    let mut rate_limit_attempts: u32 = 0;
    let mut fallback_applied = false;
    loop {
        if !fallback_applied
            && rate_limit_attempts == 3
            && let Some(ref fb) = config.fallback_model
            && let Some(obj) = body.as_object_mut()
        {
            obj.insert("model".to_string(), json!(fb));
            fallback_applied = true;
            let _ = tx
                .send(Ok(StreamEvent::StatusHint {
                    message: format!(
                        "⟳ rate limited — retrying with fallback model `{fb}` ({}/{RETRY_MAX})",
                        rate_limit_attempts + 1
                    ),
                }))
                .await;
        }

        let attempt_number = rate_limit_attempts.saturating_add(1);
        let started_at = time::OffsetDateTime::now_utc();
        let request_hash = if let Some(obs) = observer.as_ref() {
            let request_bytes = match canonical_cbor_bytes(&body) {
                Ok(bytes) => bytes,
                Err(err) => {
                    let ended_at = time::OffsetDateTime::now_utc();
                    obs.record_attempt(AttemptRecord {
                        attempt_number,
                        started_at,
                        ended_at,
                        status: AttemptStatus::Other(err.to_string()),
                        request_hash: sentinel_hash(),
                        response_hash: None,
                        stream_hash: None,
                        error_message: Some(err.to_string()),
                    });
                    let _ = tx.send(Err(err)).await;
                    return;
                }
            };
            match obs.put_object(&request_bytes) {
                Ok(hash) => Some(hash),
                Err(err) => {
                    let ended_at = time::OffsetDateTime::now_utc();
                    let msg = format!("journal write failed: {err}");
                    obs.record_attempt(AttemptRecord {
                        attempt_number,
                        started_at,
                        ended_at,
                        status: AttemptStatus::Other(msg.clone()),
                        request_hash: sentinel_hash(),
                        response_hash: None,
                        stream_hash: None,
                        error_message: Some(msg.clone()),
                    });
                    let _ = tx
                        .send(Err(ModelError::BackendUnavailable { message: msg }))
                        .await;
                    return;
                }
            }
        } else {
            None
        };

        let resp = match inner
            .client
            .post(&url)
            .header("x-api-key", inner.api_key.expose_secret().as_str())
            .header("anthropic-version", "2023-06-01")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let err = map_reqwest_send_error(e);
                if let Some(request_hash) = request_hash
                    && let Some(obs) = observer.as_ref()
                {
                    let ended_at = time::OffsetDateTime::now_utc();
                    obs.record_attempt(AttemptRecord {
                        attempt_number,
                        started_at,
                        ended_at,
                        status: map_model_error_to_attempt_status(&err),
                        request_hash,
                        response_hash: None,
                        stream_hash: None,
                        error_message: Some(err.to_string()),
                    });
                }
                let _ = tx.send(Err(err)).await;
                return;
            }
        };

        let status = resp.status();
        if !status.is_success() {
            let overloaded = status.as_u16() == 529;
            let rate_limited = status == StatusCode::TOO_MANY_REQUESTS || overloaded;
            let status_kind = map_http_status_to_attempt_status(status);
            if rate_limited {
                rate_limit_attempts = rate_limit_attempts.saturating_add(1);
                let headers = resp.headers().clone();
                let body_text = resp.text().await.unwrap_or_default();
                if let Some(request_hash) = request_hash
                    && let Some(obs) = observer.as_ref()
                {
                    let ended_at = time::OffsetDateTime::now_utc();
                    obs.record_attempt(AttemptRecord {
                        attempt_number,
                        started_at,
                        ended_at,
                        status: status_kind,
                        request_hash,
                        response_hash: None,
                        stream_hash: None,
                        error_message: Some(format!(
                            "HTTP {status}: {}",
                            truncate_body(&body_text)
                        )),
                    });
                }
                if rate_limit_attempts > RETRY_MAX {
                    let _ = tx
                        .send(Err(map_anthropic_http_status(status, &body_text, &headers)))
                        .await;
                    return;
                }
                let header_wait = headers
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());
                let exp = RETRY_BASE_SECS
                    .saturating_mul(RETRY_MULTIPLIER.pow(rate_limit_attempts.saturating_sub(1)));
                let wait_secs = header_wait.unwrap_or(exp).clamp(1, RETRY_CAP_SECS);
                for remaining in (1..=wait_secs).rev() {
                    let _ = tx
                        .send(Ok(StreamEvent::StatusHint {
                            message: format!(
                                "⟳ rate limited — retrying in {remaining}s ({rate_limit_attempts}/{RETRY_MAX})",
                            ),
                        }))
                        .await;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                continue;
            }

            let headers = resp.headers().clone();
            let body_text = match resp.text().await {
                Ok(t) => t,
                Err(e) => {
                    let err = ModelError::StreamInterrupted {
                        message: e.to_string(),
                    };
                    if let Some(request_hash) = request_hash
                        && let Some(obs) = observer.as_ref()
                    {
                        let ended_at = time::OffsetDateTime::now_utc();
                        obs.record_attempt(AttemptRecord {
                            attempt_number,
                            started_at,
                            ended_at,
                            status: map_model_error_to_attempt_status(&err),
                            request_hash,
                            response_hash: None,
                            stream_hash: None,
                            error_message: Some(err.to_string()),
                        });
                    }
                    let _ = tx.send(Err(err)).await;
                    return;
                }
            };
            let err = map_anthropic_http_status(status, &body_text, &headers);
            if let Some(request_hash) = request_hash
                && let Some(obs) = observer.as_ref()
            {
                let ended_at = time::OffsetDateTime::now_utc();
                obs.record_attempt(AttemptRecord {
                    attempt_number,
                    started_at,
                    ended_at,
                    status: status_kind,
                    request_hash,
                    response_hash: None,
                    stream_hash: None,
                    error_message: Some(err.to_string()),
                });
            }
            let _ = tx.send(Err(err)).await;
            return;
        }

        let ready_ev = StreamEvent::ProviderReady {
            provider: "Anthropic".into(),
            model: inner.model.clone(),
        };
        if let Some(obs) = observer.as_ref()
            && let Ok(bytes) = canonical_cbor_bytes(&stream_event_chunk(&ready_ev))
        {
            let _ = obs.put_object(&bytes);
        }
        let _ = tx.send(Ok(ready_ev)).await;

        let deadline = Duration::from_millis(config.first_token_deadline_ms);
        let mut byte_stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut chunk_hashes: Vec<Hash> = Vec::new();
        let mut response_synth = AnthropicSynthesizedResponse::default();
        let first =
            match tokio::time::timeout(deadline, read_next_sse_event(&mut buf, &mut byte_stream))
                .await
            {
                Err(_) => {
                    let err = ModelError::FirstTokenTimeout;
                    if let Some(request_hash) = request_hash.clone()
                        && let Some(obs) = observer.as_ref()
                    {
                        let ended_at = time::OffsetDateTime::now_utc();
                        obs.record_attempt(AttemptRecord {
                            attempt_number,
                            started_at,
                            ended_at,
                            status: map_model_error_to_attempt_status(&err),
                            request_hash,
                            response_hash: None,
                            stream_hash: None,
                            error_message: Some(err.to_string()),
                        });
                    }
                    let _ = tx.send(Err(err)).await;
                    return;
                }
                Ok(Err(e)) => {
                    if let Some(request_hash) = request_hash.clone()
                        && let Some(obs) = observer.as_ref()
                    {
                        let ended_at = time::OffsetDateTime::now_utc();
                        obs.record_attempt(AttemptRecord {
                            attempt_number,
                            started_at,
                            ended_at,
                            status: map_model_error_to_attempt_status(&e),
                            request_hash,
                            response_hash: None,
                            stream_hash: None,
                            error_message: Some(e.to_string()),
                        });
                    }
                    let _ = tx.send(Err(e)).await;
                    return;
                }
                Ok(Ok(None)) => {
                    let err = ModelError::StreamInterrupted {
                        message: "empty SSE stream".into(),
                    };
                    if let Some(request_hash) = request_hash.clone()
                        && let Some(obs) = observer.as_ref()
                    {
                        let ended_at = time::OffsetDateTime::now_utc();
                        obs.record_attempt(AttemptRecord {
                            attempt_number,
                            started_at,
                            ended_at,
                            status: map_model_error_to_attempt_status(&err),
                            request_hash,
                            response_hash: None,
                            stream_hash: None,
                            error_message: Some(err.to_string()),
                        });
                    }
                    let _ = tx.send(Err(err)).await;
                    return;
                }
                Ok(Ok(Some(o))) => o,
            };

        let mut tool_builds: BTreeMap<usize, ToolAccum> = BTreeMap::new();
        let mut finished_tools: BTreeMap<usize, ModelToolCall> = BTreeMap::new();
        let mut usage_acc: Option<UsageReport> = None;
        let mut done_sent = false;
        let mut pending_block: Option<String> = Some(first);
        loop {
            let block = match pending_block.take() {
                Some(b) => b,
                None => match read_next_sse_event(&mut buf, &mut byte_stream).await {
                    Ok(Some(b)) => b,
                    Ok(None) => break,
                    Err(e) => {
                        if let Some(request_hash) = request_hash.clone()
                            && let Some(obs) = observer.as_ref()
                        {
                            let ended_at = time::OffsetDateTime::now_utc();
                            let response_hash = canonical_cbor_bytes(&response_synth)
                                .ok()
                                .and_then(|bytes| obs.put_object(&bytes).ok());
                            let stream_hash = canonical_cbor_bytes(&chunk_hashes)
                                .ok()
                                .and_then(|bytes| obs.put_object(&bytes).ok());
                            obs.record_attempt(AttemptRecord {
                                attempt_number,
                                started_at,
                                ended_at,
                                status: map_model_error_to_attempt_status(&e),
                                request_hash,
                                response_hash,
                                stream_hash,
                                error_message: Some(e.to_string()),
                            });
                        }
                        let _ = tx.send(Err(e)).await;
                        return;
                    }
                },
            };

            let v = match sse_block_to_json(&block) {
                Ok(Some(x)) => x,
                Ok(None) => continue,
                Err(e) => {
                    if let Some(request_hash) = request_hash.clone()
                        && let Some(obs) = observer.as_ref()
                    {
                        let ended_at = time::OffsetDateTime::now_utc();
                        let response_hash = canonical_cbor_bytes(&response_synth)
                            .ok()
                            .and_then(|bytes| obs.put_object(&bytes).ok());
                        let stream_hash = canonical_cbor_bytes(&chunk_hashes)
                            .ok()
                            .and_then(|bytes| obs.put_object(&bytes).ok());
                        obs.record_attempt(AttemptRecord {
                            attempt_number,
                            started_at,
                            ended_at,
                            status: map_model_error_to_attempt_status(&e),
                            request_hash,
                            response_hash,
                            stream_hash,
                            error_message: Some(e.to_string()),
                        });
                    }
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            };

            let events = match apply_anthropic_sse_json(
                &v,
                &mut tool_builds,
                &mut finished_tools,
                &mut usage_acc,
            ) {
                Ok(evs) => evs,
                Err(e) => {
                    if let Some(request_hash) = request_hash.clone()
                        && let Some(obs) = observer.as_ref()
                    {
                        let ended_at = time::OffsetDateTime::now_utc();
                        let response_hash = canonical_cbor_bytes(&response_synth)
                            .ok()
                            .and_then(|bytes| obs.put_object(&bytes).ok());
                        let stream_hash = canonical_cbor_bytes(&chunk_hashes)
                            .ok()
                            .and_then(|bytes| obs.put_object(&bytes).ok());
                        obs.record_attempt(AttemptRecord {
                            attempt_number,
                            started_at,
                            ended_at,
                            status: map_model_error_to_attempt_status(&e),
                            request_hash,
                            response_hash,
                            stream_hash,
                            error_message: Some(e.to_string()),
                        });
                    }
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            };
            for ev in events {
                if let Some(obs) = observer.as_ref()
                    && let Ok(bytes) = canonical_cbor_bytes(&stream_event_chunk(&ev))
                    && let Ok(hash) = obs.put_object(&bytes)
                {
                    chunk_hashes.push(hash);
                }
                accumulate_response(&ev, &mut response_synth);
                if matches!(ev, StreamEvent::Done { .. }) {
                    done_sent = true;
                }
                if tx.send(Ok(ev)).await.is_err() {
                    return;
                }
            }
            if done_sent {
                break;
            }
        }

        if !done_sent {
            if let Some(ref snap) = usage_acc {
                let usage_ev = StreamEvent::UsageReport(snap.clone());
                if let Some(obs) = observer.as_ref()
                    && let Ok(bytes) = canonical_cbor_bytes(&stream_event_chunk(&usage_ev))
                    && let Ok(hash) = obs.put_object(&bytes)
                {
                    chunk_hashes.push(hash);
                }
                let _ = tx.send(Ok(usage_ev)).await;
            }
            let done_ev = StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            };
            if let Some(obs) = observer.as_ref()
                && let Ok(bytes) = canonical_cbor_bytes(&stream_event_chunk(&done_ev))
                && let Ok(hash) = obs.put_object(&bytes)
            {
                chunk_hashes.push(hash);
            }
            accumulate_response(&done_ev, &mut response_synth);
            let _ = tx.send(Ok(done_ev)).await;
        }

        if let Some(request_hash) = request_hash
            && let Some(obs) = observer.as_ref()
        {
            let ended_at = time::OffsetDateTime::now_utc();
            let response_hash = if response_synth.text.is_empty()
                && response_synth.tool_calls.is_empty()
                && response_synth.stop_reason.is_none()
            {
                None
            } else if let Ok(bytes) = canonical_cbor_bytes(&response_synth) {
                obs.put_object(&bytes).ok()
            } else {
                None
            };
            let stream_hash = if chunk_hashes.is_empty() {
                None
            } else if let Ok(bytes) = canonical_cbor_bytes(&chunk_hashes) {
                obs.put_object(&bytes).ok()
            } else {
                None
            };
            obs.record_attempt(AttemptRecord {
                attempt_number,
                started_at,
                ended_at,
                status: AttemptStatus::Success,
                request_hash,
                response_hash,
                stream_hash,
                error_message: None,
            });
        }
        return;
    }
}

#[async_trait]
impl LlmProvider for AnthropicBackend {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn context_window_tokens(&self) -> usize {
        self.inner.context_window
    }

    fn completion_model_id(&self) -> &str {
        self.inner.model.as_str()
    }

    fn set_attempt_observer(&self, observer: Arc<dyn AttemptObserver>) {
        *self
            .attempt_observer
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(observer);
    }

    fn estimate_tokens(&self, messages: &[Message]) -> Option<usize> {
        Some(crate::estimate_messages_tokens(messages))
    }

    async fn complete(
        &self,
        messages: &[Message],
        config: &CompletionConfig,
    ) -> Result<CompletionStream, ModelError> {
        let (system_blocks, rest) = anthropic_leading_system_blocks(messages);
        let api_messages = build_anthropic_api_messages(&rest);
        let tools = anthropic_tools_from_config(config);
        let body = build_request_json(
            &self.inner.model,
            &system_blocks,
            api_messages,
            tools,
            config,
        );

        let (tx, rx) = mpsc::channel::<Result<StreamEvent, ModelError>>(64);
        let inner = Arc::clone(&self.inner);
        let cfg = config.clone();
        let observer = self.observer();
        tokio::spawn(run_anthropic_stream(inner, body, cfg, observer, tx));

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CompletionConfig;
    use crate::journaling::JournalingProvider;
    use akmon_journal::{
        EventKind, HashAlgorithm, MemoryObjectStore, MemorySessionGraph, ObjectStore, SessionGraph,
    };
    use futures::StreamExt;
    use mockito::Server;
    use static_assertions::assert_not_impl_any;
    use std::sync::Mutex;

    fn anthropic_stream_ok(text: &str) -> String {
        format!(
            "data: {{\"type\":\"message_start\",\"message\":{{\"usage\":{{\"input_tokens\":1,\"output_tokens\":0}}}}}}\n\n\
             data: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{text}\"}}}}\n\n\
             data: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"input_tokens\":1,\"output_tokens\":1,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0}}}}\n\n"
        )
    }

    #[test]
    fn anthropic_backend_new_default_model_and_base_url() {
        let b = AnthropicBackend::new(Secret::new("k".to_string()), DEFAULT_ANTHROPIC_MODEL);
        assert_eq!(b.model(), DEFAULT_ANTHROPIC_MODEL);
        assert_eq!(b.base_url(), DEFAULT_ANTHROPIC_BASE_URL);
    }

    #[test]
    fn anthropic_backend_with_base_url_trims_slash() {
        let b = AnthropicBackend::with_base_url(
            Secret::new("k".to_string()),
            "m",
            "https://example.com/v1/",
        );
        assert_eq!(b.base_url(), "https://example.com/v1");
    }

    #[test]
    fn anthropic_backend_does_not_impl_debug() {
        assert_not_impl_any!(AnthropicBackend: std::fmt::Debug);
    }

    #[test]
    fn anthropic_system_and_rest_splits_roles() {
        let msgs = vec![
            Message {
                role: MessageRole::System,
                content: "a".into(),
            },
            Message {
                role: MessageRole::System,
                content: "b".into(),
            },
            Message {
                role: MessageRole::User,
                content: "hi".into(),
            },
        ];
        let (sys, rest) = anthropic_system_and_rest(&msgs);
        assert_eq!(sys, "a\n\nb");
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].content, "hi");
        assert_eq!(anthropic_system_block_text(&msgs), "a\n\nb");
    }

    #[test]
    fn message_delta_updates_cache_token_fields_from_usage() {
        let mut tool_builds = BTreeMap::new();
        let mut finished_tools = BTreeMap::new();
        let mut usage_acc = Some(UsageReport {
            input_tokens: 10,
            output_tokens: 0,
            cache_creation_tokens: 9999,
            cache_read_tokens: 0,
        });
        let v = json!({
            "type": "message_delta",
            "usage": {
                "input_tokens": 50,
                "output_tokens": 7,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 8779
            },
            "delta": { "stop_reason": "end_turn" }
        });
        let out =
            apply_anthropic_sse_json(&v, &mut tool_builds, &mut finished_tools, &mut usage_acc)
                .expect("parse");
        assert!(out.iter().any(|e| matches!(e, StreamEvent::Done { .. })));
        let snap = usage_acc.as_ref().expect("usage");
        assert_eq!(snap.input_tokens, 50);
        assert_eq!(snap.output_tokens, 7);
        assert_eq!(snap.cache_creation_tokens, 0);
        assert_eq!(snap.cache_read_tokens, 8779);
    }

    #[test]
    fn request_json_system_is_plain_text_blocks_with_top_level_cache_control() {
        let cfg = CompletionConfig::default();
        let blocks = vec!["system prompt body".into()];
        let body = build_request_json("claude-test", &blocks, vec![], vec![], &cfg);
        assert_eq!(
            body.get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
        let sys = body.get("system").expect("system key");
        let arr = sys.as_array().expect("system must be array");
        assert_eq!(arr.len(), 1);
        let block = &arr[0];
        assert_eq!(block.get("type"), Some(&json!("text")));
        assert_eq!(block.get("text"), Some(&json!("system prompt body")));
        assert!(block.get("cache_control").is_none());
    }

    #[test]
    fn request_json_splits_multiple_system_blocks_without_per_block_cache() {
        let cfg = CompletionConfig::default();
        let blocks = vec!["base".into(), "project".into()];
        let body = build_request_json("claude-test", &blocks, vec![], vec![], &cfg);
        assert_eq!(
            body.get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
        let arr = body["system"].as_array().expect("system array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["text"], json!("base"));
        assert_eq!(arr[1]["text"], json!("project"));
        assert!(arr[0].get("cache_control").is_none());
        assert!(arr[1].get("cache_control").is_none());
    }

    #[test]
    fn request_json_omits_system_when_empty() {
        let cfg = CompletionConfig::default();
        let body = build_request_json("m", &[], vec![], vec![], &cfg);
        assert!(body.get("system").is_none());
        assert_eq!(
            body.get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }

    #[test]
    fn request_json_tools_have_no_tool_level_cache_control() {
        let cfg = CompletionConfig::default();
        let tools = vec![
            json!({
                "name": "list_directory",
                "description": "d0",
                "input_schema": {},
            }),
            json!({
                "name": "read_file",
                "description": "d1",
                "input_schema": {},
            }),
        ];
        let body = build_request_json("m", &[], vec![], tools, &cfg);
        let arr = body
            .get("tools")
            .expect("tools")
            .as_array()
            .expect("tools array");
        assert_eq!(arr.len(), 2);
        assert!(arr[0].get("cache_control").is_none());
        assert!(arr[1].get("cache_control").is_none());
    }

    #[tokio::test]
    async fn t_anthropic_publishes_single_success_attempt() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/v1/messages")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(anthropic_stream_ok("ok"))
            .create_async()
            .await;
        let backend = AnthropicBackend::with_base_url(
            Secret::new("k".to_string()),
            "claude-haiku-4-5-20251001",
            server.url(),
        );
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(Arc::clone(&store), uuid::Uuid::new_v4());
        let cwd_hash = store.put(b"cwd").expect("cwd hash");
        let config_hash = store.put(b"cfg").expect("cfg hash");
        graph
            .append(EventKind::SessionStart {
                cwd_hash,
                config_hash,
            })
            .expect("session start");
        let graph = Arc::new(Mutex::new(graph));
        let wrapped = JournalingProvider::new(
            backend,
            "anthropic".to_owned(),
            Arc::clone(&store),
            Arc::clone(&graph),
        );
        let msgs = vec![Message {
            role: MessageRole::User,
            content: "hello".to_owned(),
        }];
        let cfg = CompletionConfig::default();
        let mut s = wrapped.complete(&msgs, &cfg).await.expect("complete");
        while let Some(item) = s.next().await {
            item.expect("stream item");
        }
        let history = graph
            .lock()
            .expect("graph lock")
            .history()
            .expect("history");
        let (_, attempts, stream_hash) = history
            .iter()
            .find_map(|(_, e)| match &e.kind {
                EventKind::ProviderCall {
                    provider_id: _,
                    attempts,
                    stream_hash,
                } => Some(((), attempts.clone(), stream_hash.clone())),
                _ => None,
            })
            .expect("provider call");
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].attempt_number, 1);
        assert_eq!(attempts[0].status, AttemptStatus::Success);
        assert!(attempts[0].response_hash.is_some());
        assert!(attempts[0].stream_hash.is_some());
        assert_eq!(stream_hash, attempts[0].stream_hash);
    }

    #[tokio::test]
    async fn t_anthropic_publishes_retry_sequence_on_rate_limit() {
        let mut server = Server::new_async().await;
        let _m1 = server
            .mock("POST", "/v1/messages")
            .with_status(429)
            .with_body("{\"error\":\"rate\"}")
            .expect(2)
            .create_async()
            .await;
        let _m2 = server
            .mock("POST", "/v1/messages")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(anthropic_stream_ok("ok"))
            .expect(1)
            .create_async()
            .await;
        let backend = AnthropicBackend::with_base_url(
            Secret::new("k".to_string()),
            "claude-haiku-4-5-20251001",
            server.url(),
        );
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(Arc::clone(&store), uuid::Uuid::new_v4());
        let cwd_hash = store.put(b"cwd").expect("cwd hash");
        let config_hash = store.put(b"cfg").expect("cfg hash");
        graph
            .append(EventKind::SessionStart {
                cwd_hash,
                config_hash,
            })
            .expect("session start");
        let graph = Arc::new(Mutex::new(graph));
        let wrapped = JournalingProvider::new(
            backend,
            "anthropic".to_owned(),
            Arc::clone(&store),
            Arc::clone(&graph),
        );
        let msgs = vec![Message {
            role: MessageRole::User,
            content: "hello".to_owned(),
        }];
        let cfg = CompletionConfig::default();
        let mut s = wrapped.complete(&msgs, &cfg).await.expect("complete");
        while let Some(item) = s.next().await {
            item.expect("stream item");
        }
        let history = graph
            .lock()
            .expect("graph lock")
            .history()
            .expect("history");
        let (_, attempts, stream_hash) = history
            .iter()
            .find_map(|(_, e)| match &e.kind {
                EventKind::ProviderCall {
                    provider_id: _,
                    attempts,
                    stream_hash,
                } => Some(((), attempts.clone(), stream_hash.clone())),
                _ => None,
            })
            .expect("provider call");
        assert_eq!(attempts.len(), 3);
        assert_eq!(attempts[0].attempt_number, 1);
        assert_eq!(attempts[1].attempt_number, 2);
        assert_eq!(attempts[2].attempt_number, 3);
        assert_eq!(attempts[0].status, AttemptStatus::RateLimited);
        assert_eq!(attempts[1].status, AttemptStatus::RateLimited);
        assert_eq!(attempts[2].status, AttemptStatus::Success);
        assert!(attempts[0].response_hash.is_none() && attempts[0].stream_hash.is_none());
        assert!(attempts[1].response_hash.is_none() && attempts[1].stream_hash.is_none());
        assert!(attempts[2].response_hash.is_some() && attempts[2].stream_hash.is_some());
        assert_eq!(stream_hash, attempts[2].stream_hash);
    }

    #[tokio::test]
    async fn t_anthropic_fallback_model_mutates_request_hash() {
        let mut server = Server::new_async().await;
        let _m1 = server
            .mock("POST", "/v1/messages")
            .with_status(429)
            .with_body("{\"error\":\"rate\"}")
            .expect(4)
            .create_async()
            .await;
        let _m2 = server
            .mock("POST", "/v1/messages")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(anthropic_stream_ok("ok"))
            .expect(1)
            .create_async()
            .await;
        let backend = AnthropicBackend::with_base_url(
            Secret::new("k".to_string()),
            "claude-haiku-4-5-20251001",
            server.url(),
        );
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(Arc::clone(&store), uuid::Uuid::new_v4());
        let cwd_hash = store.put(b"cwd").expect("cwd hash");
        let config_hash = store.put(b"cfg").expect("cfg hash");
        graph
            .append(EventKind::SessionStart {
                cwd_hash,
                config_hash,
            })
            .expect("session start");
        let graph = Arc::new(Mutex::new(graph));
        let wrapped = JournalingProvider::new(
            backend,
            "anthropic".to_owned(),
            Arc::clone(&store),
            Arc::clone(&graph),
        );
        let msgs = vec![Message {
            role: MessageRole::User,
            content: "hello".to_owned(),
        }];
        let cfg = CompletionConfig {
            fallback_model: Some("claude-sonnet-4-5-20251001".to_owned()),
            ..CompletionConfig::default()
        };
        let mut s = wrapped.complete(&msgs, &cfg).await.expect("complete");
        while let Some(item) = s.next().await {
            item.expect("stream item");
        }
        let history = graph
            .lock()
            .expect("graph lock")
            .history()
            .expect("history");
        let attempts = history
            .iter()
            .find_map(|(_, e)| match &e.kind {
                EventKind::ProviderCall {
                    provider_id: _,
                    attempts,
                    stream_hash: _,
                } => Some(attempts.clone()),
                _ => None,
            })
            .expect("provider call");
        assert_eq!(attempts.len(), 5);
        assert_eq!(attempts[0].status, AttemptStatus::RateLimited);
        assert_eq!(attempts[1].status, AttemptStatus::RateLimited);
        assert_eq!(attempts[2].status, AttemptStatus::RateLimited);
        assert_eq!(attempts[3].status, AttemptStatus::RateLimited);
        assert_eq!(attempts[4].status, AttemptStatus::Success);
        assert_eq!(attempts[0].request_hash, attempts[1].request_hash);
        assert_eq!(attempts[1].request_hash, attempts[2].request_hash);
        assert_eq!(attempts[3].request_hash, attempts[4].request_hash);
        assert_ne!(attempts[2].request_hash, attempts[3].request_hash);
    }

    #[tokio::test]
    async fn t_anthropic_handles_overloaded_response() {
        let mut server = Server::new_async().await;
        let _m1 = server
            .mock("POST", "/v1/messages")
            .with_status(529)
            .with_body("{\"error\":\"overloaded\"}")
            .expect(2)
            .create_async()
            .await;
        let _m2 = server
            .mock("POST", "/v1/messages")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(anthropic_stream_ok("ok"))
            .expect(1)
            .create_async()
            .await;
        let backend = AnthropicBackend::with_base_url(
            Secret::new("k".to_string()),
            "claude-haiku-4-5-20251001",
            server.url(),
        );
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(Arc::clone(&store), uuid::Uuid::new_v4());
        let cwd_hash = store.put(b"cwd").expect("cwd hash");
        let config_hash = store.put(b"cfg").expect("cfg hash");
        graph
            .append(EventKind::SessionStart {
                cwd_hash,
                config_hash,
            })
            .expect("session start");
        let graph = Arc::new(Mutex::new(graph));
        let wrapped = JournalingProvider::new(
            backend,
            "anthropic".to_owned(),
            Arc::clone(&store),
            Arc::clone(&graph),
        );
        let msgs = vec![Message {
            role: MessageRole::User,
            content: "hello".to_owned(),
        }];
        let cfg = CompletionConfig::default();
        let mut s = wrapped.complete(&msgs, &cfg).await.expect("complete");
        while let Some(item) = s.next().await {
            item.expect("stream item");
        }
        let history = graph
            .lock()
            .expect("graph lock")
            .history()
            .expect("history");
        let attempts = history
            .iter()
            .find_map(|(_, e)| match &e.kind {
                EventKind::ProviderCall {
                    provider_id: _,
                    attempts,
                    stream_hash: _,
                } => Some(attempts.clone()),
                _ => None,
            })
            .expect("provider call");
        assert_eq!(attempts.len(), 3);
        assert_eq!(attempts[0].status, AttemptStatus::ServerError);
        assert_eq!(attempts[1].status, AttemptStatus::ServerError);
        assert_eq!(attempts[2].status, AttemptStatus::Success);
    }
}
