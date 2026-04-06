//! [Ollama](https://ollama.com/) `/api/chat` backend.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt, ready};
use pin_project_lite::pin_project;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::LlmProvider;
use crate::config::CompletionConfig;
use crate::error::ModelError;
use crate::message::{Message, MessageRole};
use crate::stream::{CompletionStream, ModelToolCall, StopReason, StreamEvent};
use crate::tool_def::ToolDefinition;

/// JSON line from Ollama's NDJSON chat stream (or the single JSON body when `stream: false`).
#[derive(Debug, serde::Deserialize)]
struct OllamaChatLine {
    #[serde(default)]
    message: Option<OllamaMessageBody>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaMessageBody {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Option<Vec<serde_json::Value>>,
}

/// One tool entry in the Ollama `/api/chat` `tools` array (OpenAI-style).
#[derive(Debug, Serialize)]
struct OllamaToolWire<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: OllamaFunctionWire<'a>,
}

#[derive(Debug, Serialize)]
struct OllamaFunctionWire<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

fn ollama_tools_from_config(config: &CompletionConfig) -> Vec<OllamaToolWire<'_>> {
    config
        .tools
        .iter()
        .map(|t: &ToolDefinition| OllamaToolWire {
            kind: "function",
            function: OllamaFunctionWire {
                name: t.name.as_str(),
                description: t.description.as_str(),
                parameters: &t.parameters,
            },
        })
        .collect()
}

/// Outgoing chat request body for `/api/chat`.
#[derive(Debug, Serialize)]
struct OllamaChatRequest<'a> {
    model: &'a str,
    messages: Vec<OllamaApiMessage<'a>>,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OllamaToolWire<'a>>,
    options: OllamaOptions,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    temperature: f32,
    num_predict: i32,
}

#[derive(Debug, Serialize)]
struct OllamaApiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

pin_project! {
    /// Incrementally splits a byte stream into newline-terminated UTF-8 lines.
    ///
    /// `S` is typically `reqwest`'s response body byte stream. The projection keeps
    /// `poll_next` correct when `S` is pinned.
    pub(crate) struct OllamaLineDemux<S> {
        #[pin]
        source: S,
        buffer: String,
    }
}

impl<S> OllamaLineDemux<S> {
    /// Wraps `source` with an empty line buffer.
    pub(crate) fn new(source: S) -> Self {
        Self {
            source,
            buffer: String::new(),
        }
    }

    /// Same as [`OllamaLineDemux::new`] but seeds `buffer` (e.g. after a partial read).
    pub(crate) fn with_buffer(source: S, buffer: String) -> Self {
        Self { source, buffer }
    }
}

impl<S> Stream for OllamaLineDemux<S>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Send,
{
    type Item = Result<String, ModelError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        loop {
            if let Some(pos) = this.buffer.find('\n') {
                let line = this.buffer[..pos].to_string();
                this.buffer.drain(..=pos);
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                return Poll::Ready(Some(Ok(trimmed.to_string())));
            }
            match ready!(this.source.as_mut().poll_next(cx)) {
                None => {
                    let rest = std::mem::take(this.buffer);
                    let trimmed = rest.trim();
                    if trimmed.is_empty() {
                        return Poll::Ready(None);
                    }
                    return Poll::Ready(Some(Ok(trimmed.to_string())));
                }
                Some(Err(e)) => {
                    return Poll::Ready(Some(Err(map_reqwest_stream_error(e))));
                }
                Some(Ok(bytes)) => {
                    this.buffer.push_str(&String::from_utf8_lossy(&bytes));
                }
            }
        }
    }
}

/// Talks to a single Ollama server and model id.
#[derive(Debug, Clone)]
pub struct OllamaBackend {
    base_url: String,
    model: String,
    client: reqwest::Client,
    /// Advertised context size (approximate; Ollama does not always expose this per pull).
    context_window_tokens: usize,
}

impl OllamaBackend {
    /// Builds a backend for `base_url` (trailing `/` stripped) and `model`.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        let mut base_url = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        let client = build_http_client();
        Self {
            base_url,
            model: model.into(),
            client,
            context_window_tokens: 8192,
        }
    }

    /// Overrides the value returned by [`LlmProvider::context_window_tokens`].
    pub fn with_context_window_tokens(mut self, n: usize) -> Self {
        self.context_window_tokens = n;
        self
    }

    /// Exposes the configured API base (without trailing slash).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Exposes the Ollama model tag.
    pub fn model(&self) -> &str {
        &self.model
    }

    fn chat_url(&self) -> String {
        format!("{base}/api/chat", base = self.base_url)
    }

    fn map_messages(messages: &[Message]) -> Vec<OllamaApiMessage<'_>> {
        messages
            .iter()
            .map(|m| OllamaApiMessage {
                role: match m.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                },
                content: m.content.as_str(),
            })
            .collect()
    }
}

fn build_http_client() -> reqwest::Client {
    match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(120))
        .build()
    {
        Ok(c) => c,
        Err(_) => reqwest::Client::new(),
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

fn map_status(status: StatusCode, body: &str, headers: &HeaderMap) -> ModelError {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ModelError::AuthError,
        StatusCode::TOO_MANY_REQUESTS => ModelError::RateLimited {
            retry_after_secs: headers
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok()),
        },
        StatusCode::BAD_REQUEST if body.to_lowercase().contains("context") => {
            ModelError::ContextWindowExceeded
        }
        _ => ModelError::BackendUnavailable {
            message: format!("HTTP {status}: {body}"),
        },
    }
}

/// Final stop reason given the merged tool-call list for this completion (streaming may put
/// `tool_calls` only on an earlier chunk while `done: true` arrives on a later line).
fn finalize_stop_reason(line: &OllamaChatLine, tool_calls: &[ModelToolCall]) -> StopReason {
    if line
        .done_reason
        .as_deref()
        .is_some_and(|r| r.eq_ignore_ascii_case("length"))
    {
        return StopReason::MaxTokens;
    }
    if !tool_calls.is_empty() {
        return StopReason::ToolUse;
    }
    StopReason::EndTurn
}

fn stop_reason_from_line(line: &OllamaChatLine) -> StopReason {
    let tool_calls = extract_tool_calls_from_line(line);
    finalize_stop_reason(line, &tool_calls)
}

fn parse_arguments_field(v: &serde_json::Value) -> serde_json::Value {
    if let Some(s) = v.as_str() {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(s) {
            parsed
        } else {
            serde_json::Value::String(s.to_string())
        }
    } else {
        v.clone()
    }
}

fn parse_one_tool_call(v: &serde_json::Value) -> Option<ModelToolCall> {
    let id = v.get("id").and_then(|x| x.as_str())?.to_string();
    let func = v.get("function")?;
    let name = func.get("name").and_then(|x| x.as_str())?.to_string();
    let arguments = func
        .get("arguments")
        .map(parse_arguments_field)
        .unwrap_or_else(|| serde_json::json!({}));
    Some(ModelToolCall {
        id,
        name,
        arguments,
    })
}

fn extract_tool_calls_from_line(line: &OllamaChatLine) -> Vec<ModelToolCall> {
    let mut out = Vec::new();
    let mut push_list = |list: &[serde_json::Value]| {
        for v in list {
            if let Some(c) = parse_one_tool_call(v) {
                out.push(c);
            }
        }
    };
    if let Some(list) = line.tool_calls.as_deref() {
        push_list(list);
    }
    if let Some(msg) = &line.message
        && let Some(list) = msg.tool_calls.as_deref()
    {
        push_list(list);
    }
    out
}

/// Reads until the first complete `\n`-terminated line or EOF, enforcing `deadline` for the
/// first body chunk(s). Returns the line and a demuxer for any remaining bytes.
async fn read_first_line_with_timeout<S>(
    mut stream: S,
    deadline: Duration,
) -> Result<(String, OllamaLineDemux<S>), ModelError>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
    let mut buf = String::new();
    let outcome = tokio::time::timeout(deadline, async {
        loop {
            if let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string();
                buf.drain(..=pos);
                if line.is_empty() {
                    continue;
                }
                return Ok::<_, ModelError>((line, buf, stream));
            }
            match stream.next().await {
                None => {
                    return if buf.trim().is_empty() {
                        Err(ModelError::StreamInterrupted {
                            message: "empty response stream".into(),
                        })
                    } else {
                        Ok((buf.trim().to_string(), String::new(), stream))
                    };
                }
                Some(Err(e)) => return Err(map_reqwest_stream_error(e)),
                Some(Ok(bytes)) => {
                    buf.push_str(&String::from_utf8_lossy(&bytes));
                }
            }
        }
    })
    .await;

    match outcome {
        Err(_) => Err(ModelError::FirstTokenTimeout),
        Ok(Err(e)) => Err(e),
        Ok(Ok((line, remainder, stream))) => {
            let demux = if remainder.is_empty() {
                OllamaLineDemux::new(stream)
            } else {
                OllamaLineDemux::with_buffer(stream, remainder)
            };
            Ok((line, demux))
        }
    }
}

async fn process_json_line(
    line: &str,
    tx: &mpsc::Sender<Result<StreamEvent, ModelError>>,
    done_sent: &mut bool,
    pending_tool_calls: &mut Vec<ModelToolCall>,
) -> Result<(), ModelError> {
    let parsed: OllamaChatLine =
        serde_json::from_str(line).map_err(|e| ModelError::StreamInterrupted {
            message: format!("invalid JSON line: {e}"),
        })?;

    if let Some(msg) = &parsed.message
        && !msg.content.is_empty()
    {
        let _ = tx
            .send(Ok(StreamEvent::TextDelta {
                text: msg.content.clone(),
            }))
            .await;
    }

    let from_line = extract_tool_calls_from_line(&parsed);
    if !from_line.is_empty() {
        *pending_tool_calls = from_line;
    }

    if parsed.done {
        *done_sent = true;
        let mut tool_calls = extract_tool_calls_from_line(&parsed);
        if tool_calls.is_empty() {
            tool_calls = pending_tool_calls.clone();
        }
        let reason = finalize_stop_reason(&parsed, &tool_calls);
        let _ = tx
            .send(Ok(StreamEvent::Done {
                stop_reason: reason,
                tool_calls,
            }))
            .await;
    }

    Ok(())
}

async fn run_streaming(
    backend: OllamaBackend,
    messages: Vec<Message>,
    config: CompletionConfig,
    tx: mpsc::Sender<Result<StreamEvent, ModelError>>,
) {
    let url = backend.chat_url();
    let tools = ollama_tools_from_config(&config);
    let req_body = OllamaChatRequest {
        model: backend.model.as_str(),
        messages: OllamaBackend::map_messages(&messages),
        stream: true,
        tools,
        options: OllamaOptions {
            temperature: config.temperature,
            num_predict: config.max_tokens.min(i32::MAX as u32) as i32,
        },
    };

    let resp = match backend.client.post(&url).json(&req_body).send().await {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(Err(map_reqwest_send_error(e))).await;
            return;
        }
    };

    let status = resp.status();
    let headers = resp.headers().clone();
    if !status.is_success() {
        let body = match resp.text().await {
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
        let _ = tx.send(Err(map_status(status, &body, &headers))).await;
        return;
    }

    let byte_stream = resp.bytes_stream();
    let deadline = Duration::from_millis(config.first_token_deadline_ms);

    let (first_line, mut lines) = match read_first_line_with_timeout(byte_stream, deadline).await {
        Ok(x) => x,
        Err(e) => {
            let _ = tx.send(Err(e)).await;
            return;
        }
    };

    let mut done_sent = false;
    let mut pending_tool_calls: Vec<ModelToolCall> = Vec::new();
    if let Err(e) =
        process_json_line(&first_line, &tx, &mut done_sent, &mut pending_tool_calls).await
    {
        let _ = tx.send(Err(e)).await;
        return;
    }

    while let Some(item) = lines.next().await {
        match item {
            Err(e) => {
                let _ = tx.send(Err(e)).await;
                return;
            }
            Ok(line) => {
                if let Err(e) =
                    process_json_line(&line, &tx, &mut done_sent, &mut pending_tool_calls).await
                {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            }
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

async fn run_buffered(
    backend: OllamaBackend,
    messages: Vec<Message>,
    config: CompletionConfig,
    tx: mpsc::Sender<Result<StreamEvent, ModelError>>,
) {
    let url = backend.chat_url();
    let tools = ollama_tools_from_config(&config);
    let req_body = OllamaChatRequest {
        model: backend.model.as_str(),
        messages: OllamaBackend::map_messages(&messages),
        stream: false,
        tools,
        options: OllamaOptions {
            temperature: config.temperature,
            num_predict: config.max_tokens.min(i32::MAX as u32) as i32,
        },
    };

    let deadline = Duration::from_millis(config.first_token_deadline_ms);
    let collect = async {
        let resp = backend
            .client
            .post(&url)
            .json(&req_body)
            .send()
            .await
            .map_err(map_reqwest_send_error)?;

        let status = resp.status();
        let headers = resp.headers().clone();
        if !status.is_success() {
            let body = resp
                .text()
                .await
                .map_err(|e| ModelError::StreamInterrupted {
                    message: e.to_string(),
                })?;
            return Err(map_status(status, &body, &headers));
        }

        resp.text()
            .await
            .map_err(|e| ModelError::StreamInterrupted {
                message: e.to_string(),
            })
    };

    let text = match tokio::time::timeout(deadline, collect).await {
        Err(_) => {
            let _ = tx.send(Err(ModelError::FirstTokenTimeout)).await;
            return;
        }
        Ok(Err(e)) => {
            let _ = tx.send(Err(e)).await;
            return;
        }
        Ok(Ok(t)) => t,
    };

    let parsed: OllamaChatLine = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            let _ = tx
                .send(Err(ModelError::StreamInterrupted {
                    message: format!("invalid JSON body: {e}"),
                }))
                .await;
            return;
        }
    };

    if let Some(ref msg) = parsed.message
        && !msg.content.is_empty()
    {
        let _ = tx
            .send(Ok(StreamEvent::TextDelta {
                text: msg.content.clone(),
            }))
            .await;
    }

    let reason = stop_reason_from_line(&parsed);
    let tool_calls = extract_tool_calls_from_line(&parsed);
    let _ = tx
        .send(Ok(StreamEvent::Done {
            stop_reason: reason,
            tool_calls,
        }))
        .await;
}

#[async_trait]
impl LlmProvider for OllamaBackend {
    fn name(&self) -> &str {
        "ollama"
    }

    fn context_window_tokens(&self) -> usize {
        self.context_window_tokens
    }

    fn completion_model_id(&self) -> &str {
        self.model.as_str()
    }

    async fn complete(
        &self,
        messages: &[Message],
        config: &CompletionConfig,
    ) -> Result<CompletionStream, ModelError> {
        let (tx, rx) = mpsc::channel::<Result<StreamEvent, ModelError>>(64);
        let backend = self.clone();
        let messages_vec: Vec<Message> = messages.to_vec();
        let cfg = config.clone();

        if config.stream {
            tokio::spawn(run_streaming(backend, messages_vec, cfg, tx));
        } else {
            tokio::spawn(run_buffered(backend, messages_vec, cfg, tx));
        }

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_trims_slash_and_stores_fields() {
        let b = OllamaBackend::new("http://127.0.0.1:11434///", "llama3.2");
        assert_eq!(b.base_url(), "http://127.0.0.1:11434");
        assert_eq!(b.model(), "llama3.2");
    }
}
