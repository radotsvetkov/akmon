//! OpenAI-compatible HTTPS chat completions (`/chat/completions`) with SSE streaming.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use akmon_core::Secret;
use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use reqwest::StatusCode;
use reqwest::header::{HeaderName, HeaderValue};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::config::CompletionConfig;
use crate::error::ModelError;
use crate::max_tokens::max_tokens_for_openai_style_model;
use crate::message::{Message, MessageRole};
use crate::stream::{CompletionStream, ModelToolCall, StopReason, StreamEvent, UsageReport};
use crate::tool_def::ToolDefinition;
use crate::{AttemptObserver, LlmProvider};
use akmon_journal::{AttemptRecord, AttemptStatus, Hash, HashAlgorithm};

/// Infers a conservative context window from model id substrings.
pub fn infer_context_window_tokens(model_id: &str) -> usize {
    let m = model_id.to_lowercase();
    if m.contains("gpt-4") {
        return 128_000;
    }
    if m.contains("gpt-3.5") {
        return 16_384;
    }
    if m.contains("claude") {
        return 200_000;
    }
    if m.contains("llama-3") || m.contains("llama3") {
        return 128_000;
    }
    if m.contains("llama-2") || m.contains("llama2") {
        return 4096;
    }
    if m.contains("deepseek") {
        return 64_000;
    }
    if m.contains("mistral") {
        return 32_000;
    }
    if m.contains("gemma") {
        return 128_000;
    }
    if m.contains("qwen") {
        return 128_000;
    }
    32_000
}

#[derive(Clone, Copy, Debug)]
enum AuthStyle {
    Bearer,
    ApiKey(&'static str),
}

fn trim_slash(mut s: String) -> String {
    while s.ends_with('/') {
        s.pop();
    }
    s
}

fn host_only(base_url: &str) -> String {
    let base = base_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    base.split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_string()
}

fn openai_stream_footer_label(display_name: &str) -> String {
    let slug = display_name.split('/').next().unwrap_or(display_name);
    match slug {
        "openrouter" => "OpenRouter".into(),
        "openai" => "OpenAI".into(),
        "groq" => "Groq".into(),
        "azure" => "Azure OpenAI".into(),
        _ if !slug.is_empty() => {
            let mut c = slug.chars();
            c.next().map_or_else(
                || "OpenAI-compatible".into(),
                |f| f.to_uppercase().to_string() + c.as_str(),
            )
        }
        _ => "OpenAI-compatible".into(),
    }
}

fn provider_slug_for_host(host: &str) -> String {
    match host {
        "openrouter.ai" | "www.openrouter.ai" => "openrouter".into(),
        "api.openai.com" => "openai".into(),
        "api.groq.com" => "groq".into(),
        "api.together.xyz" => "together".into(),
        "api.deepseek.com" => "deepseek".into(),
        _ if !host.is_empty() => host
            .strip_prefix("api.")
            .unwrap_or(host)
            .split('.')
            .next()
            .unwrap_or(host)
            .to_string(),
        _ => "openai-compat".into(),
    }
}

fn extract_azure_deployment_from_endpoint(endpoint: &str) -> Option<String> {
    let key = "/deployments/";
    let pos = endpoint.find(key)?;
    let rest = &endpoint[pos + key.len()..];
    let seg = rest.split('/').next()?;
    if seg.is_empty() {
        None
    } else {
        Some(seg.to_string())
    }
}

fn tool_message_to_openai(m: &Message) -> Option<Value> {
    let v: Value = serde_json::from_str(&m.content).ok()?;
    let id = v.get("tool_call_id").and_then(|x| x.as_str())?;
    let output = v.get("output").cloned().unwrap_or(json!({}));
    let content_str = serde_json::to_string(&output).ok()?;
    Some(json!({
        "role": "tool",
        "tool_call_id": id,
        "content": content_str,
    }))
}

fn assistant_to_openai(m: &Message) -> Value {
    if let Ok(v) = serde_json::from_str::<Value>(&m.content)
        && let Some(arr) = v.get("tool_calls").and_then(|a| a.as_array())
        && !arr.is_empty()
    {
        let mut tcalls: Vec<Value> = Vec::new();
        for tc in arr {
            let id = tc
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let name = tc
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let args = tc.get("arguments").cloned().unwrap_or(json!({}));
            let arg_str = match args {
                Value::String(ref s) => s.clone(),
                ref o => serde_json::to_string(o).unwrap_or_else(|_| "{}".into()),
            };
            tcalls.push(json!({
                "id": id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": arg_str,
                }
            }));
        }
        let text = v
            .get("text")
            .and_then(|x| x.as_str())
            .filter(|t| !t.trim().is_empty());
        return json!({
            "role": "assistant",
            "content": text.unwrap_or(""),
            "tool_calls": tcalls,
        });
    }
    json!({
        "role": "assistant",
        "content": m.content,
    })
}

fn build_openai_messages(msgs: &[Message]) -> Result<Vec<Value>, ModelError> {
    let mut out: Vec<Value> = Vec::new();
    let mut system_parts: Vec<&str> = Vec::new();
    for m in msgs {
        if m.role == MessageRole::System {
            system_parts.push(m.content.as_str());
        }
    }
    if !system_parts.is_empty() {
        out.push(json!({
            "role": "system",
            "content": system_parts.join("\n\n"),
        }));
    }

    let mut i: usize = 0;
    while i < msgs.len() {
        let m = &msgs[i];
        if m.role == MessageRole::System {
            i += 1;
            continue;
        }
        if m.role == MessageRole::Tool {
            if let Some(tm) = tool_message_to_openai(m) {
                out.push(tm);
            }
            i += 1;
            continue;
        }
        if m.role == MessageRole::Assistant {
            out.push(assistant_to_openai(m));
            i += 1;
            continue;
        }
        if m.role == MessageRole::User {
            out.push(json!({
                "role": "user",
                "content": m.content,
            }));
            i += 1;
            continue;
        }
        i += 1;
    }
    Ok(out)
}

#[derive(Debug, Serialize)]
struct OpenAiToolWire<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: OpenAiFnWire<'a>,
}

#[derive(Debug, Serialize)]
struct OpenAiFnWire<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a Value,
}

fn openai_tools(config: &CompletionConfig) -> Vec<OpenAiToolWire<'_>> {
    config
        .tools
        .iter()
        .map(|t: &ToolDefinition| OpenAiToolWire {
            kind: "function",
            function: OpenAiFnWire {
                name: t.name.as_str(),
                description: t.description.as_str(),
                parameters: &t.parameters,
            },
        })
        .collect()
}

struct OpenAiCompatInner {
    display_name: String,
    post_url: String,
    api_key: Secret<String>,
    model: String,
    client: reqwest::Client,
    auth: AuthStyle,
    extra_headers: Vec<(String, String)>,
    context_window: usize,
}

/// HTTP client for `/v1/chat/completions`-style APIs (OpenAI, OpenRouter, Groq, Azure, local LM Studio, …).
///
/// API keys are held in [`Secret`]; this type does **not** implement [`std::fmt::Debug`].
pub struct OpenAiCompatBackend {
    inner: Arc<OpenAiCompatInner>,
    attempt_observer: Arc<RwLock<Option<Arc<dyn AttemptObserver>>>>,
}

impl OpenAiCompatBackend {
    fn new_with_parts(
        provider_slug: String,
        post_url: String,
        api_key: Secret<String>,
        model: String,
        auth: AuthStyle,
        extra_headers: Vec<(String, String)>,
    ) -> Self {
        let client = build_client();
        let context_window = infer_context_window_tokens(&model);
        let display_name = format!("{provider_slug}/{model}");
        Self {
            inner: Arc::new(OpenAiCompatInner {
                display_name,
                post_url,
                api_key,
                model,
                client,
                auth,
                extra_headers,
                context_window,
            }),
            attempt_observer: Arc::new(RwLock::new(None)),
        }
    }

    /// Custom base URL (no `/chat/completions` suffix); trailing slashes stripped.
    pub fn custom(base_url: String, api_key: String, model: String) -> Self {
        let base = trim_slash(base_url);
        let host = host_only(&base);
        let slug = provider_slug_for_host(&host);
        let url = format!("{base}/chat/completions");
        Self::new_with_parts(
            slug,
            url,
            Secret::new(api_key),
            model,
            AuthStyle::Bearer,
            Vec::new(),
        )
    }

    /// OpenRouter (`https://openrouter.ai/api/v1`).
    pub fn openrouter(api_key: String, model: String) -> Self {
        Self::new_with_parts(
            "openrouter".into(),
            "https://openrouter.ai/api/v1/chat/completions".into(),
            Secret::new(api_key),
            model,
            AuthStyle::Bearer,
            vec![
                ("HTTP-Referer".into(), "https://akmon.dev".into()),
                ("X-Title".into(), "Akmon".into()),
            ],
        )
    }

    /// OpenAI (`https://api.openai.com/v1`).
    pub fn openai(api_key: String, model: String) -> Self {
        Self::custom("https://api.openai.com/v1".into(), api_key, model)
    }

    /// Groq OpenAI-compatible endpoint.
    pub fn groq(api_key: String, model: String) -> Self {
        Self::new_with_parts(
            "groq".into(),
            "https://api.groq.com/openai/v1/chat/completions".into(),
            Secret::new(api_key),
            model,
            AuthStyle::Bearer,
            Vec::new(),
        )
    }

    /// Together.ai.
    pub fn together(api_key: String, model: String) -> Self {
        Self::new_with_parts(
            "together".into(),
            "https://api.together.xyz/v1/chat/completions".into(),
            Secret::new(api_key),
            model,
            AuthStyle::Bearer,
            Vec::new(),
        )
    }

    /// DeepSeek.
    pub fn deepseek(api_key: String, model: String) -> Self {
        Self::new_with_parts(
            "deepseek".into(),
            "https://api.deepseek.com/v1/chat/completions".into(),
            Secret::new(api_key),
            model,
            AuthStyle::Bearer,
            Vec::new(),
        )
    }

    /// Azure OpenAI: `endpoint` should include `/deployments/{deployment}/chat/completions` before the query string.
    /// Sends `api-key` header (not `Authorization: Bearer`).
    pub fn azure(endpoint: String, api_key: String, api_version: String) -> Self {
        let endpoint = trim_slash(endpoint);
        let deployment =
            extract_azure_deployment_from_endpoint(&endpoint).unwrap_or_else(|| "gpt-4".into());
        let post_url = if endpoint.contains("api-version=") {
            endpoint
        } else {
            format!("{endpoint}?api-version={api_version}")
        };
        Self::new_with_parts(
            "azure".into(),
            post_url,
            Secret::new(api_key),
            deployment,
            AuthStyle::ApiKey("api-key"),
            Vec::new(),
        )
    }

    /// Full `POST` URL (for tests and Azure).
    pub fn post_url(&self) -> &str {
        &self.inner.post_url
    }

    /// Extra headers configured for this backend (e.g. OpenRouter).
    pub fn extra_headers(&self) -> &[(String, String)] {
        &self.inner.extra_headers
    }

    /// Authentication style (Bearer vs Azure `api-key`).
    pub fn auth_style_is_azure_api_key(&self) -> bool {
        matches!(self.inner.auth, AuthStyle::ApiKey(_))
    }

    fn observer(&self) -> Option<Arc<dyn AttemptObserver>> {
        self.attempt_observer
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

fn build_client() -> reqwest::Client {
    crate::http_client::build_http_client(10, 300).unwrap_or_else(|e| {
        panic!("openai-compatible HTTP client: {e}");
    })
}

fn map_http_status(status: StatusCode, body: &str) -> ModelError {
    match status.as_u16() {
        401 => ModelError::AuthError,
        429 => ModelError::RateLimited {
            retry_after_secs: None,
        },
        _ => ModelError::BackendUnavailable {
            message: format!("HTTP {status}: {msg}", msg = truncate(body)),
        },
    }
}

fn truncate(s: &str) -> String {
    const M: usize = 512;
    if s.len() <= M {
        s.to_string()
    } else {
        format!("{prefix}…", prefix = &s[..M])
    }
}

fn map_send_err(e: reqwest::Error) -> ModelError {
    if e.is_timeout() {
        return ModelError::FirstTokenTimeout;
    }
    ModelError::BackendUnavailable {
        message: e.to_string(),
    }
}

fn map_stream_err(e: reqwest::Error) -> ModelError {
    ModelError::StreamInterrupted {
        message: e.to_string(),
    }
}

#[derive(Debug, Default, Serialize)]
struct OpenAiSynthesizedResponse {
    text: String,
    tool_calls: Vec<ModelToolCall>,
    stop_reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct OpenAiStreamEventChunk {
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

fn canonical_cbor_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, ModelError> {
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

fn stream_event_chunk(event: &StreamEvent) -> OpenAiStreamEventChunk {
    match event {
        StreamEvent::ProviderReady { provider, model } => OpenAiStreamEventChunk {
            kind: "provider_ready",
            provider: Some(provider.clone()),
            model: Some(model.clone()),
            message: None,
            text: None,
            stop_reason: None,
            tool_calls: None,
        },
        StreamEvent::StatusHint { message } => OpenAiStreamEventChunk {
            kind: "status_hint",
            provider: None,
            model: None,
            message: Some(message.clone()),
            text: None,
            stop_reason: None,
            tool_calls: None,
        },
        StreamEvent::TextDelta { text } => OpenAiStreamEventChunk {
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
        } => OpenAiStreamEventChunk {
            kind: "done",
            provider: None,
            model: None,
            message: None,
            text: None,
            stop_reason: Some(stop_reason_label(stop_reason).to_owned()),
            tool_calls: Some(tool_calls.clone()),
        },
        StreamEvent::UsageReport(_) => OpenAiStreamEventChunk {
            kind: "usage_report",
            provider: None,
            model: None,
            message: None,
            text: None,
            stop_reason: None,
            tool_calls: None,
        },
        StreamEvent::Error { error } => OpenAiStreamEventChunk {
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

fn accumulate_response(event: &StreamEvent, response: &mut OpenAiSynthesizedResponse) {
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

/// Parses one SSE `data:` payload line block (reuse Anthropic-style SSE framing).
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
            Some(Err(e)) => return Err(map_stream_err(e)),
            Some(Ok(bytes)) => buf.push_str(&String::from_utf8_lossy(&bytes)),
        }
    }
}

#[derive(Default)]
struct ToolPart {
    id: String,
    name: String,
    args: String,
}

fn apply_openai_sse_line(
    v: &Value,
    tools: &mut BTreeMap<u32, ToolPart>,
    usage_acc: &mut Option<UsageReport>,
) -> Result<Vec<StreamEvent>, ModelError> {
    let mut out: Vec<StreamEvent> = Vec::new();

    if let Some(u) = v.get("usage") {
        *usage_acc = Some(usage_report_from_openai_json(u));
    }

    let choice = v
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first());
    let delta = choice.and_then(|c| c.get("delta"));
    if let Some(d) = delta {
        if let Some(txt) = d.get("content").and_then(|x| x.as_str())
            && !txt.is_empty()
        {
            out.push(StreamEvent::TextDelta {
                text: txt.to_string(),
            });
        }
        if let Some(tarr) = d.get("tool_calls").and_then(|x| x.as_array()) {
            for tc in tarr {
                let index = tc.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                let entry = tools.entry(index).or_default();
                if let Some(id) = tc.get("id").and_then(|x| x.as_str())
                    && !id.is_empty()
                {
                    entry.id = id.to_string();
                }
                if let Some(f) = tc.get("function") {
                    if let Some(n) = f.get("name").and_then(|x| x.as_str())
                        && !n.is_empty()
                    {
                        entry.name = n.to_string();
                    }
                    if let Some(a) = f.get("arguments").and_then(|x| x.as_str()) {
                        entry.args.push_str(a);
                    }
                }
            }
        }
    }

    let finish = choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(|x| x.as_str());
    if let Some(fr) = finish
        && !fr.is_empty()
        && fr != "null"
    {
        let stop = match fr {
            "tool_calls" => StopReason::ToolUse,
            "length" => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };
        let mut model_calls: Vec<ModelToolCall> = Vec::new();
        if stop == StopReason::ToolUse {
            let drained = std::mem::take(tools);
            for (_i, part) in drained {
                if part.name.is_empty() {
                    continue;
                }
                let arguments = if part.args.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&part.args).unwrap_or(json!({}))
                };
                model_calls.push(ModelToolCall {
                    id: if part.id.is_empty() {
                        format!("call_{name}", name = part.name)
                    } else {
                        part.id
                    },
                    name: part.name,
                    arguments,
                });
            }
        }
        if let Some(snap) = usage_acc.clone() {
            out.push(StreamEvent::UsageReport(snap));
        }
        out.push(StreamEvent::Done {
            stop_reason: stop,
            tool_calls: model_calls,
        });
    }

    Ok(out)
}

fn u32_from(v: Option<&Value>) -> u32 {
    v.and_then(|x| x.as_u64())
        .map(|n| n.min(u64::from(u32::MAX)) as u32)
        .unwrap_or(0)
}

/// Maps OpenAI `usage` object to [`UsageReport`], including `prompt_tokens_details.cached_tokens`.
fn usage_report_from_openai_json(u: &Value) -> UsageReport {
    let cached = u
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    UsageReport {
        input_tokens: u32_from(u.get("prompt_tokens")),
        output_tokens: u32_from(u.get("completion_tokens")),
        cache_creation_tokens: 0,
        cache_read_tokens: cached.min(u64::from(u32::MAX)) as u32,
    }
}

/// Warn in debug builds if the combined system string looks dynamically generated (hurts OpenAI prefix caching).
fn validate_openai_system_prefix_stability(system: &str) {
    if !cfg!(debug_assertions) {
        return;
    }
    let low = system.to_lowercase();
    const INDICATORS: &[&str] = &[
        "current time:",
        "today is",
        "session id:",
        "turn:",
        "timestamp:",
        "date:",
    ];
    for ind in INDICATORS {
        if low.contains(ind) {
            tracing::warn!(
                indicator = ind,
                "OpenAI system prompt may contain per-request dynamic text; automatic prompt caching needs a byte-stable system prefix — move volatile context into the user message"
            );
        }
    }
}

#[must_use]
fn jittered_wait_secs(base: u64) -> u64 {
    let nano = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let jitter = ((nano % 2001) as f64 / 1000.0 - 1.0) * 0.2 * (base as f64);
    (base as f64 + jitter).round().max(1.0) as u64
}

fn openai_post_request(inner: &OpenAiCompatInner, body: &Value) -> reqwest::RequestBuilder {
    let mut req = inner
        .client
        .post(inner.post_url.clone())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "text/event-stream");
    match inner.auth {
        AuthStyle::Bearer => {
            let tok = format!("Bearer {key}", key = inner.api_key.expose_secret());
            if let Ok(h) = HeaderValue::from_str(&tok) {
                req = req.header(reqwest::header::AUTHORIZATION, h);
            }
        }
        AuthStyle::ApiKey(hname) => {
            if let (Ok(name), Ok(val)) = (
                HeaderName::try_from(hname),
                HeaderValue::from_str(inner.api_key.expose_secret()),
            ) {
                req = req.header(name, val);
            }
        }
    }
    for (k, v) in &inner.extra_headers {
        if let (Ok(name), Ok(val)) = (HeaderName::try_from(k.as_str()), HeaderValue::from_str(v)) {
            req = req.header(name, val);
        }
    }
    req.json(body)
}

async fn run_openai_stream(
    inner: Arc<OpenAiCompatInner>,
    mut body: Value,
    config: CompletionConfig,
    stream_footer_label: String,
    observer: Option<Arc<dyn AttemptObserver>>,
    tx: mpsc::Sender<Result<StreamEvent, ModelError>>,
) {
    const RETRY_MAX: u32 = 5;
    const RETRY_BASE_SECS: u64 = 1;
    const RETRY_MULT: u64 = 2;
    const RETRY_CAP_SECS: u64 = 60;

    let mut rate_attempts: u32 = 0;
    let mut fallback_applied = false;
    loop {
        if !fallback_applied
            && rate_attempts == 3
            && let Some(ref fb) = config.fallback_model
            && let Some(obj) = body.as_object_mut()
        {
            obj.insert("model".to_string(), json!(fb));
            fallback_applied = true;
            let _ = tx
                .send(Ok(StreamEvent::StatusHint {
                    message: format!(
                        "⟳ rate limited — retrying with fallback model `{fb}` ({}/{RETRY_MAX})",
                        rate_attempts + 1
                    ),
                }))
                .await;
        }

        let attempt_number = rate_attempts.saturating_add(1);
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

        let resp = match openai_post_request(&inner, &body).send().await {
            Ok(r) => r,
            Err(e) => {
                let err = map_send_err(e);
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

        if !resp.status().is_success() {
            let status = resp.status();
            let overloaded = status.as_u16() == 529;
            let rate_limited = status == StatusCode::TOO_MANY_REQUESTS || overloaded;
            let status_kind = map_http_status_to_attempt_status(status);

            if rate_limited {
                rate_attempts = rate_attempts.saturating_add(1);
                let headers = resp.headers().clone();
                let body_txt = resp.text().await.unwrap_or_default();
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
                        error_message: Some(format!("HTTP {status}: {}", truncate(&body_txt))),
                    });
                }
                if rate_attempts > RETRY_MAX {
                    let _ = tx.send(Err(map_http_status(status, &body_txt))).await;
                    return;
                }

                let header_wait = headers
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());
                let exp =
                    RETRY_BASE_SECS.saturating_mul(RETRY_MULT.pow(rate_attempts.saturating_sub(1)));
                let base_wait = header_wait.unwrap_or(exp).max(1);
                let wait_secs = jittered_wait_secs(base_wait).min(RETRY_CAP_SECS);

                for remaining in (1..=wait_secs).rev() {
                    let _ = tx
                        .send(Ok(StreamEvent::StatusHint {
                            message: format!(
                                "⟳ rate limited — retrying in {remaining}s ({rate_attempts}/{RETRY_MAX})"
                            ),
                        }))
                        .await;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                continue;
            }

            let body_txt = match resp.text().await {
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
            let err = map_http_status(status, &body_txt);
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

        let provider_ready = StreamEvent::ProviderReady {
            provider: stream_footer_label.clone(),
            model: inner.model.clone(),
        };
        if let Some(obs) = observer.as_ref()
            && let Ok(bytes) = canonical_cbor_bytes(&stream_event_chunk(&provider_ready))
        {
            let _ = obs.put_object(&bytes);
        }
        let _ = tx.send(Ok(provider_ready)).await;

        let mut byte_stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut chunk_hashes: Vec<Hash> = Vec::new();
        let mut response_synth = OpenAiSynthesizedResponse::default();
        let deadline = Duration::from_millis(config.first_token_deadline_ms);
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
                Ok(Ok(Some(b))) => b,
            };

        let mut tools: BTreeMap<u32, ToolPart> = BTreeMap::new();
        let mut usage_acc: Option<UsageReport> = None;
        let mut done_sent = false;
        let mut pending = Some(first);
        loop {
            let block = match pending.take() {
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
                Ok(None) => {
                    if !done_sent {
                        if let Some(snap) = usage_acc.clone() {
                            let usage_ev = StreamEvent::UsageReport(snap);
                            if let Some(obs) = observer.as_ref()
                                && let Ok(bytes) =
                                    canonical_cbor_bytes(&stream_event_chunk(&usage_ev))
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
                        done_sent = true;
                    }
                    break;
                }
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

            let events = match apply_openai_sse_line(&v, &mut tools, &mut usage_acc) {
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
            if let Some(snap) = usage_acc {
                let usage_ev = StreamEvent::UsageReport(snap);
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
impl LlmProvider for OpenAiCompatBackend {
    fn name(&self) -> &str {
        self.inner.display_name.as_str()
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

    async fn complete(
        &self,
        messages: &[Message],
        config: &CompletionConfig,
    ) -> Result<CompletionStream, ModelError> {
        let msgv = build_openai_messages(messages)?;
        if let Some(first) = msgv.first()
            && first.get("role") == Some(&json!("system"))
            && let Some(Value::String(s)) = first.get("content")
        {
            validate_openai_system_prefix_stability(s);
        }
        let tools = openai_tools(config);
        let mut body_map = serde_json::Map::new();
        body_map.insert("model".into(), json!(self.inner.model));
        body_map.insert("messages".into(), Value::Array(msgv));
        body_map.insert("stream".into(), json!(true));
        body_map.insert("temperature".into(), json!(config.temperature));
        let max_out = if config.max_tokens == 0 {
            max_tokens_for_openai_style_model(&self.inner.model)
        } else {
            config.max_tokens
        };
        body_map.insert("max_tokens".into(), json!(max_out));
        body_map.insert("stream_options".into(), json!({ "include_usage": true }));
        if self.inner.post_url.contains("api.openai.com") {
            body_map.insert("store".into(), json!(false));
        }
        if self.inner.post_url.contains("openrouter.ai")
            && let Some(ref sid) = config.session_id
            && !sid.is_empty()
        {
            let key: String = sid.chars().take(16).collect();
            body_map.insert("prompt_cache_key".into(), json!(key));
        }
        if !tools.is_empty() {
            body_map.insert(
                "tools".into(),
                serde_json::to_value(&tools).map_err(|e| ModelError::StreamInterrupted {
                    message: e.to_string(),
                })?,
            );
        }
        let body = Value::Object(body_map);
        let (tx, rx) = mpsc::channel(64);
        let inner = Arc::clone(&self.inner);
        let cfg = config.clone();
        let footer = openai_stream_footer_label(self.inner.display_name.as_str());
        let observer = self.observer();
        tokio::spawn(run_openai_stream(inner, body, cfg, footer, observer, tx));
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

    fn stream_body_ok(text: &str) -> String {
        format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}}}}]}}\n\n\
             data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":1,\"completion_tokens\":1,\"prompt_tokens_details\":{{\"cached_tokens\":0}}}}}}\n\n\
             data: [DONE]\n\n"
        )
    }

    #[test]
    fn openrouter_base_url_and_headers() {
        let b = OpenAiCompatBackend::openrouter("k".into(), "anthropic/claude-3".into());
        assert!(
            b.post_url()
                .contains("openrouter.ai/api/v1/chat/completions")
        );
        assert!(
            b.extra_headers()
                .iter()
                .any(|(k, v)| k == "HTTP-Referer" && v == "https://akmon.dev")
        );
        assert!(
            b.extra_headers()
                .iter()
                .any(|(k, v)| k == "X-Title" && v == "Akmon")
        );
    }

    #[test]
    fn azure_appends_api_version() {
        let b = OpenAiCompatBackend::azure(
            "https://x.openai.azure.com/openai/deployments/gpt-4o/chat/completions".into(),
            "key".into(),
            "2024-02-01".into(),
        );
        assert!(b.post_url().contains("api-version=2024-02-01"));
        assert!(b.auth_style_is_azure_api_key());
    }

    #[test]
    fn tool_wire_wraps_function_object() {
        let cfg = CompletionConfig {
            tools: vec![ToolDefinition {
                name: "read_file".into(),
                description: "d".into(),
                parameters: json!({"type": "object"}),
            }],
            ..CompletionConfig::default()
        };
        let t = openai_tools(&cfg);
        let v = serde_json::to_value(&t).expect("json");
        assert_eq!(v[0]["type"], "function");
        assert_eq!(v[0]["function"]["name"], "read_file");
    }

    #[test]
    fn sse_openai_text_delta_and_done_stop() {
        let mut tools = BTreeMap::new();
        let mut usage = None;
        let v = json!({
            "choices": [{"delta": {"content": "Hi"}}]
        });
        let ev = apply_openai_sse_line(&v, &mut tools, &mut usage).expect("ok");
        assert!(
            ev.iter()
                .any(|e| matches!(e, StreamEvent::TextDelta { text } if text == "Hi"))
        );
        let v2 = json!({
            "choices": [{
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 2,
                "prompt_tokens_details": {"cached_tokens": 40}
            }
        });
        let ev2 = apply_openai_sse_line(&v2, &mut tools, &mut usage).expect("ok");
        assert!(ev2.iter().any(|e| matches!(e, StreamEvent::Done { stop_reason, .. } if *stop_reason == StopReason::EndTurn)));
        let snap = usage.expect("usage");
        assert_eq!(snap.cache_read_tokens, 40);
    }

    #[test]
    fn sse_openai_tool_calls_finish() {
        let mut tools = BTreeMap::new();
        let mut usage = None;
        let v = json!({
            "choices": [{
                "delta": {"tool_calls": [{
                    "index": 0,
                    "id": "call_1",
                    "function": {"name": "read_file", "arguments": "{\"path\":\""}
                }]}
            }]
        });
        let _ = apply_openai_sse_line(&v, &mut tools, &mut usage).expect("ok");
        let v2 = json!({
            "choices": [{
                "delta": {"tool_calls": [{
                    "index": 0,
                    "function": {"arguments": "src/main.rs\"}"}
                }]},
                "finish_reason": "tool_calls"
            }]
        });
        let ev2 = apply_openai_sse_line(&v2, &mut tools, &mut usage).expect("ok");
        assert!(ev2.iter().any(|e| matches!(e, StreamEvent::Done { stop_reason, .. } if *stop_reason == StopReason::ToolUse)));
    }

    #[test]
    fn openai_backend_no_debug() {
        assert_not_impl_any!(OpenAiCompatBackend: std::fmt::Debug);
    }

    #[tokio::test]
    async fn t_openai_compat_publishes_single_success_attempt() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(stream_body_ok("ok"))
            .create_async()
            .await;
        let backend = OpenAiCompatBackend::custom(server.url(), "key".into(), "gpt-4o-mini".into());
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
            "openai-compat".to_owned(),
            Arc::clone(&store),
            Arc::clone(&graph),
        );
        let messages = vec![Message {
            role: MessageRole::User,
            content: "hello".to_owned(),
        }];
        let cfg = CompletionConfig::default();
        let mut s = wrapped.complete(&messages, &cfg).await.expect("complete");
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
        let a = &attempts[0];
        assert_eq!(a.attempt_number, 1);
        assert_eq!(a.status, AttemptStatus::Success);
        assert!(a.response_hash.is_some());
        assert!(a.stream_hash.is_some());
        assert_eq!(stream_hash, a.stream_hash);
    }

    #[tokio::test]
    async fn t_openai_compat_publishes_retry_sequence_on_rate_limit() {
        let mut server = Server::new_async().await;
        let _m1 = server
            .mock("POST", "/chat/completions")
            .with_status(429)
            .with_body("{\"error\":\"rate\"}")
            .expect(2)
            .create_async()
            .await;
        let _m2 = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(stream_body_ok("ok"))
            .expect(1)
            .create_async()
            .await;
        let backend = OpenAiCompatBackend::custom(server.url(), "key".into(), "gpt-4o-mini".into());
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
            "openai-compat".to_owned(),
            Arc::clone(&store),
            Arc::clone(&graph),
        );
        let messages = vec![Message {
            role: MessageRole::User,
            content: "hello".to_owned(),
        }];
        let cfg = CompletionConfig::default();
        let mut s = wrapped.complete(&messages, &cfg).await.expect("complete");
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
    async fn t_openai_compat_fallback_model_mutates_request_hash() {
        let mut server = Server::new_async().await;
        let _m1 = server
            .mock("POST", "/chat/completions")
            .with_status(429)
            .with_body("{\"error\":\"rate\"}")
            .expect(4)
            .create_async()
            .await;
        let _m2 = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(stream_body_ok("ok"))
            .expect(1)
            .create_async()
            .await;
        let backend = OpenAiCompatBackend::custom(server.url(), "key".into(), "gpt-4o-mini".into());
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
            "openai-compat".to_owned(),
            Arc::clone(&store),
            Arc::clone(&graph),
        );
        let messages = vec![Message {
            role: MessageRole::User,
            content: "hello".to_owned(),
        }];
        let cfg = CompletionConfig {
            fallback_model: Some("gpt-4.1-mini".to_owned()),
            ..CompletionConfig::default()
        };
        let mut s = wrapped.complete(&messages, &cfg).await.expect("complete");
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
}
