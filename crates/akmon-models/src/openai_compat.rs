//! OpenAI-compatible HTTPS chat completions (`/chat/completions`) with SSE streaming.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use akmon_core::Secret;
use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::StatusCode;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::config::CompletionConfig;
use crate::error::ModelError;
use crate::message::{Message, MessageRole};
use crate::stream::{
    CompletionStream, ModelToolCall, StopReason, StreamEvent, UsageReport,
};
use crate::tool_def::ToolDefinition;
use crate::LlmProvider;

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
    let base = base_url.trim_start_matches("https://").trim_start_matches("http://");
    base.split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_string()
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
    if let Ok(v) = serde_json::from_str::<Value>(&m.content) {
        if let Some(arr) = v.get("tool_calls").and_then(|a| a.as_array()) {
            if !arr.is_empty() {
                let mut tcalls: Vec<Value> = Vec::new();
                for tc in arr {
                    let id = tc.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let name = tc.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
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
        }
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
        let display_name = format!("{provider_slug}/{}", model);
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
        let deployment = extract_azure_deployment_from_endpoint(&endpoint)
            .unwrap_or_else(|| "gpt-4".into());
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
}

fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn map_http_status(status: StatusCode, body: &str) -> ModelError {
    match status.as_u16() {
        401 => ModelError::AuthError,
        429 => ModelError::RateLimited {
            retry_after_secs: None,
        },
        _ => ModelError::BackendUnavailable {
            message: format!("HTTP {status}: {}", truncate(body)),
        },
    }
}

fn truncate(s: &str) -> String {
    const M: usize = 512;
    if s.len() <= M {
        s.to_string()
    } else {
        format!("{}…", &s[..M])
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
    serde_json::from_str::<Value>(&s).map(Some).map_err(|e| ModelError::StreamInterrupted {
        message: format!("invalid SSE JSON: {e}"),
    })
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
        *usage_acc = Some(UsageReport {
            input_tokens: u32_from(u.get("prompt_tokens")),
            output_tokens: u32_from(u.get("completion_tokens")),
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        });
    }

    let choice = v.get("choices").and_then(|c| c.as_array()).and_then(|a| a.first());
    let delta = choice.and_then(|c| c.get("delta"));
    if let Some(d) = delta {
        if let Some(txt) = d.get("content").and_then(|x| x.as_str()) {
            if !txt.is_empty() {
                out.push(StreamEvent::TextDelta {
                    text: txt.to_string(),
                });
            }
        }
        if let Some(tarr) = d.get("tool_calls").and_then(|x| x.as_array()) {
            for tc in tarr {
                let index = tc.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                let entry = tools.entry(index).or_default();
                if let Some(id) = tc.get("id").and_then(|x| x.as_str()) {
                    if !id.is_empty() {
                        entry.id = id.to_string();
                    }
                }
                if let Some(f) = tc.get("function") {
                    if let Some(n) = f.get("name").and_then(|x| x.as_str()) {
                        if !n.is_empty() {
                            entry.name = n.to_string();
                        }
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
    if let Some(fr) = finish {
        if !fr.is_empty() && fr != "null" {
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
                            format!("call_{}", part.name)
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
    }

    Ok(out)
}

fn u32_from(v: Option<&Value>) -> u32 {
    v.and_then(|x| x.as_u64())
        .map(|n| n.min(u64::from(u32::MAX)) as u32)
        .unwrap_or(0)
}

async fn run_openai_stream(
    inner: Arc<OpenAiCompatInner>,
    body: Value,
    config: CompletionConfig,
    tx: mpsc::Sender<Result<StreamEvent, ModelError>>,
) {
    let mut req = inner
        .client
        .post(inner.post_url.clone())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "text/event-stream");
    match inner.auth {
        AuthStyle::Bearer => {
            let tok = format!("Bearer {}", inner.api_key.expose_secret());
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

    let resp = match req.json(&body).send().await {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(Err(map_send_err(e))).await;
            return;
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let _ = tx.send(Err(map_http_status(status, &body))).await;
        return;
    }

    let mut byte_stream = resp.bytes_stream();
    let mut buf = String::new();
    let deadline = Duration::from_millis(config.first_token_deadline_ms);
    let first = match tokio::time::timeout(deadline, read_next_sse_event(&mut buf, &mut byte_stream))
        .await
    {
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
                        let _ = tx.send(Ok(StreamEvent::UsageReport(snap))).await;
                    }
                    let _ = tx
                        .send(Ok(StreamEvent::Done {
                            stop_reason: StopReason::EndTurn,
                            tool_calls: vec![],
                        }))
                        .await;
                    done_sent = true;
                }
                break;
            }
            Err(e) => {
                let _ = tx.send(Err(e)).await;
                return;
            }
        };

        let events = match apply_openai_sse_line(&v, &mut tools, &mut usage_acc) {
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
        if let Some(snap) = usage_acc {
            let _ = tx.send(Ok(StreamEvent::UsageReport(snap))).await;
        }
        let _ = tx
            .send(Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            }))
            .await;
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

    async fn complete(
        &self,
        messages: &[Message],
        config: &CompletionConfig,
    ) -> Result<CompletionStream, ModelError> {
        let msgv = build_openai_messages(messages)?;
        let tools = openai_tools(config);
        let mut body_map = serde_json::Map::new();
        body_map.insert("model".into(), json!(self.inner.model));
        body_map.insert("messages".into(), Value::Array(msgv));
        body_map.insert("stream".into(), json!(true));
        body_map.insert("temperature".into(), json!(config.temperature));
        body_map.insert("max_tokens".into(), json!(config.max_tokens));
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
        tokio::spawn(run_openai_stream(inner, body, cfg, tx));
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CompletionConfig;
    use static_assertions::assert_not_impl_any;

    #[test]
    fn openrouter_base_url_and_headers() {
        let b = OpenAiCompatBackend::openrouter("k".into(), "anthropic/claude-3".into());
        assert!(b.post_url().contains("openrouter.ai/api/v1/chat/completions"));
        assert!(b
            .extra_headers()
            .iter()
            .any(|(k, v)| k == "HTTP-Referer" && v == "https://akmon.dev"));
        assert!(b
            .extra_headers()
            .iter()
            .any(|(k, v)| k == "X-Title" && v == "Akmon"));
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
        assert!(ev.iter().any(|e| matches!(e, StreamEvent::TextDelta { text } if text == "Hi")));
        let v2 = json!({
            "choices": [{
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2}
        });
        let ev2 = apply_openai_sse_line(&v2, &mut tools, &mut usage).expect("ok");
        assert!(ev2.iter().any(|e| matches!(e, StreamEvent::Done { stop_reason, .. } if *stop_reason == StopReason::EndTurn)));
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
}
