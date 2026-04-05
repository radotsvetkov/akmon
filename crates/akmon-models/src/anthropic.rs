//! [Anthropic](https://www.anthropic.com/) Messages API backend (streaming SSE).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use akmon_core::Secret;
use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::config::CompletionConfig;
use crate::error::ModelError;
use crate::message::{Message, MessageRole};
use crate::stream::{CompletionStream, ModelToolCall, StopReason, StreamEvent};
use crate::tool_def::ToolDefinition;
use crate::LlmProvider;

/// Default Anthropic Messages API host (no trailing slash).
pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";

/// Default model id for [`AnthropicBackend::new`] (Anthropic API alias for Claude Haiku 4.5).
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-haiku-4-5";

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
}

fn build_anthropic_http_client() -> reqwest::Client {
    match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .build()
    {
        Ok(c) => c,
        Err(_) => reqwest::Client::new(),
    }
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
            if let Ok(v) = serde_json::from_str::<Value>(&m.content) {
                if let Some(arr) = v.get("tool_calls").and_then(|a| a.as_array()) {
                    if !arr.is_empty() {
                        let mut blocks: Vec<Value> = Vec::new();
                        if let Some(t) = v.get("text").and_then(|x| x.as_str()) {
                            if !t.trim().is_empty() {
                                blocks.push(json!({"type": "text", "text": t}));
                            }
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
                }
            }
            Some(json!({
                "role": "assistant",
                "content": m.content,
            }))
        }
        _ => None,
    }
}

fn build_anthropic_api_messages(msgs: &[&Message]) -> Vec<Value> {
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

fn anthropic_tools_from_config(config: &CompletionConfig) -> Vec<Value> {
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

fn build_request_json(
    model: &str,
    system: &str,
    messages: Vec<Value>,
    tools: Vec<Value>,
    config: &CompletionConfig,
) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("model".to_string(), json!(model));
    map.insert("max_tokens".to_string(), json!(config.max_tokens));
    map.insert("stream".to_string(), json!(true));
    map.insert("temperature".to_string(), json!(config.temperature));
    if !system.is_empty() {
        map.insert("system".to_string(), json!(system));
    }
    map.insert("messages".to_string(), Value::Array(messages));
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
        503 | 529 => ModelError::BackendUnavailable {
            message: format!("HTTP {status}"),
        },
        _ => ModelError::BackendUnavailable {
            message: format!("HTTP {status}: {}", truncate_body(body)),
        },
    }
}

fn truncate_body(s: &str) -> String {
    const MAX: usize = 512;
    if s.len() <= MAX {
        s.to_string()
    } else {
        format!("{}…", &s[..MAX])
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
    serde_json::from_str::<Value>(&s).map(Some).map_err(|e| ModelError::StreamInterrupted {
        message: format!("invalid SSE JSON: {e}"),
    })
}

struct ToolAccum {
    id: String,
    name: String,
    partial_json: String,
}

async fn read_next_sse_event<S>(buf: &mut String, stream: &mut S) -> Result<Option<String>, ModelError>
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

fn apply_anthropic_sse_json(
    v: &Value,
    tool_builds: &mut BTreeMap<usize, ToolAccum>,
    finished_tools: &mut BTreeMap<usize, ModelToolCall>,
) -> Result<Vec<StreamEvent>, ModelError> {
    let mut out: Vec<StreamEvent> = Vec::new();
    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    match ty {
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
                    if let Some(text) = delta.get("text").and_then(|x| x.as_str()) {
                        if !text.is_empty() {
                            out.push(StreamEvent::TextDelta {
                                text: text.to_string(),
                            });
                        }
                    }
                } else if dt == "input_json_delta" {
                    if let Some(p) = delta.get("partial_json").and_then(|x| x.as_str()) {
                        if let Some(acc) = tool_builds.get_mut(&index) {
                            acc.partial_json.push_str(p);
                        }
                    }
                }
            }
        }
        "content_block_start" => {
            let index = v.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
            if let Some(cb) = v.get("content_block") {
                let cbt = cb.get("type").and_then(|x| x.as_str()).unwrap_or("");
                if cbt == "tool_use" {
                    let id = cb.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let name = cb.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
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
            if let Some(delta) = v.get("delta") {
                if let Some(sr) = delta.get("stop_reason").and_then(|x| x.as_str()) {
                    let stop = match sr {
                        "end_turn" => StopReason::EndTurn,
                        "tool_use" => StopReason::ToolUse,
                        "max_tokens" => StopReason::MaxTokens,
                        _ => StopReason::EndTurn,
                    };
                    let tool_calls: Vec<ModelToolCall> = finished_tools.values().cloned().collect();
                    out.push(StreamEvent::Done {
                        stop_reason: stop,
                        tool_calls,
                    });
                }
            }
        }
        _ => {}
    }
    Ok(out)
}

async fn run_anthropic_stream(
    inner: Arc<AnthropicInner>,
    body: Value,
    config: CompletionConfig,
    tx: mpsc::Sender<Result<StreamEvent, ModelError>>,
) {
    let url = format!("{}/v1/messages", inner.base_url);
    let resp = match inner
        .client
        .post(&url)
        .header(
            "x-api-key",
            inner.api_key.expose_secret().as_str(),
        )
        .header("anthropic-version", "2023-06-01")
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/json",
        )
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(Err(map_reqwest_send_error(e))).await;
            return;
        }
    };

    let status = resp.status();
    let headers = resp.headers().clone();
    if !status.is_success() {
        let body_text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                let _ = tx
                    .send(Err(ModelError::StreamInterrupted {
                        message: e.to_string(),
                    }))
                    .await;
                return;
            }
        };
        let _ = tx
            .send(Err(map_anthropic_http_status(status, &body_text, &headers)))
            .await;
        return;
    }

    let deadline = Duration::from_millis(config.first_token_deadline_ms);
    let mut byte_stream = resp.bytes_stream();
    let mut buf = String::new();

    let first = match tokio::time::timeout(deadline, read_next_sse_event(&mut buf, &mut byte_stream)).await {
        Err(_) => {
            let _ = tx.send(Err(ModelError::FirstTokenTimeout)).await;
            return;
        }
        Ok(Err(e)) => {
            let _ = tx.send(Err(e)).await;
            return;
        }
        Ok(Ok(None)) => {
            let _ = tx
                .send(Err(ModelError::StreamInterrupted {
                    message: "empty SSE stream".into(),
                }))
                .await;
            return;
        }
        Ok(Ok(Some(o))) => o,
    };

    let mut tool_builds: BTreeMap<usize, ToolAccum> = BTreeMap::new();
    let mut finished_tools: BTreeMap<usize, ModelToolCall> = BTreeMap::new();
    let mut done_sent = false;

    let mut pending_block: Option<String> = Some(first);

    loop {
        let block = match pending_block.take() {
            Some(b) => b,
            None => match read_next_sse_event(&mut buf, &mut byte_stream).await {
                Ok(Some(b)) => b,
                Ok(None) => break,
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            },
        };

        let v = match sse_block_to_json(&block) {
            Ok(Some(x)) => x,
            Ok(None) => continue,
            Err(e) => {
                let _ = tx.send(Err(e)).await;
                return;
            }
        };

        let events = match apply_anthropic_sse_json(&v, &mut tool_builds, &mut finished_tools) {
            Ok(evs) => evs,
            Err(e) => {
                let _ = tx.send(Err(e)).await;
                return;
            }
        };

        for ev in events {
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
        let _ = tx
            .send(Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            }))
            .await;
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

    async fn complete(
        &self,
        messages: &[Message],
        config: &CompletionConfig,
    ) -> Result<CompletionStream, ModelError> {
        let (system, rest) = anthropic_system_and_rest(messages);
        let api_messages = build_anthropic_api_messages(&rest);
        let tools = anthropic_tools_from_config(config);
        let body = build_request_json(
            &self.inner.model,
            &system,
            api_messages,
            tools,
            config,
        );

        let (tx, rx) = mpsc::channel::<Result<StreamEvent, ModelError>>(64);
        let inner = Arc::clone(&self.inner);
        let cfg = config.clone();
        tokio::spawn(run_anthropic_stream(inner, body, cfg, tx));

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::assert_not_impl_any;

    #[test]
    fn anthropic_backend_new_default_model_and_base_url() {
        let b = AnthropicBackend::new(Secret::new("k".to_string()), "claude-haiku-4-5");
        assert_eq!(b.model(), "claude-haiku-4-5");
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
    }
}
