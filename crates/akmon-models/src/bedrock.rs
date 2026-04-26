//! Amazon Bedrock Runtime `InvokeModelWithResponseStream` (Claude Messages API payload) with SigV4.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use akmon_core::Secret;
use async_trait::async_trait;
use base64::Engine;
use bytes::Bytes;
use chrono::Utc;
use futures::{Stream, StreamExt};
use hmac::{Hmac, Mac};
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::anthropic::{ToolAccum, apply_anthropic_sse_json, build_bedrock_anthropic_invoke_json};
use crate::config::CompletionConfig;
use crate::error::ModelError;
use crate::message::Message;
use crate::openai_compat::infer_context_window_tokens;
use crate::stream::{CompletionStream, ModelToolCall, StopReason, StreamEvent, UsageReport};
use crate::{AttemptObserver, LlmProvider};
use akmon_journal::{AttemptRecord, AttemptStatus, Hash, HashAlgorithm};

type HmacSha256 = Hmac<Sha256>;

/// Well-known Bedrock model ids (for docs / UI).
pub const BEDROCK_DISPLAY_MODEL_IDS: &[&str] = &[
    "anthropic.claude-haiku-4-5-v1:0",
    "anthropic.claude-sonnet-4-6-v1:0",
    "anthropic.claude-opus-4-6-v1:0",
    "meta.llama3-8b-instruct-v1:0",
    "meta.llama3-70b-instruct-v1:0",
];

fn sha256_hex(data: &[u8]) -> String {
    let d = Sha256::digest(data);
    hex::encode(d)
}

fn hmac_sha256_bytes(key: &[u8], data: &[u8]) -> Result<Vec<u8>, ModelError> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| ModelError::AuthError)?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn signing_key(
    secret_key: &str,
    date_stamp: &str,
    region: &str,
    service: &str,
) -> Result<Vec<u8>, ModelError> {
    let k_date = hmac_sha256_bytes(
        format!("AWS4{secret_key}").as_bytes(),
        date_stamp.as_bytes(),
    )?;
    let k_region = hmac_sha256_bytes(&k_date, region.as_bytes())?;
    let k_service = hmac_sha256_bytes(&k_region, service.as_bytes())?;
    hmac_sha256_bytes(&k_service, b"aws4_request")
}

/// Builds the SigV4 `Authorization` header and required `x-amz-*` fields.
#[allow(clippy::too_many_arguments)]
fn authorization_header(
    method: &str,
    host: &str,
    canonical_uri: &str,
    region: &str,
    body: &[u8],
    access_key: &str,
    secret_key: &str,
    session_token: Option<&str>,
    amz_date: &str,
    date_stamp: &str,
) -> Result<HeaderMap, ModelError> {
    let payload_hash = sha256_hex(body);
    let mut signed_set = vec!["content-type", "host", "x-amz-content-sha256", "x-amz-date"];
    if session_token.is_some() {
        signed_set.push("x-amz-security-token");
    }
    signed_set.sort();
    signed_set.dedup();

    let canonical_headers = {
        let mut lines = String::new();
        let ct = "application/json";
        lines.push_str(&format!("content-type:{ct}\n"));
        lines.push_str(&format!("host:{host}\n"));
        lines.push_str(&format!("x-amz-content-sha256:{payload_hash}\n"));
        lines.push_str(&format!("x-amz-date:{amz_date}\n"));
        if let Some(st) = session_token {
            lines.push_str(&format!("x-amz-security-token:{st}\n"));
        }
        lines
    };

    let signed_headers = signed_set.join(";");
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );

    let canonical_hash = sha256_hex(canonical_request.as_bytes());
    let credential_scope = format!("{date_stamp}/{region}/bedrock/aws4_request");
    let string_to_sign =
        format!("AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{canonical_hash}");

    let key = signing_key(secret_key, date_stamp, region, "bedrock")?;
    let sig = hex::encode(hmac_sha256_bytes(&key, string_to_sign.as_bytes())?);

    let auth = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{credential_scope}, SignedHeaders={signed_headers}, Signature={sig}"
    );

    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        HeaderName::from_static("host"),
        HeaderValue::from_str(host).map_err(|e| ModelError::BackendUnavailable {
            message: e.to_string(),
        })?,
    );
    headers.insert(
        HeaderName::from_static("x-amz-date"),
        HeaderValue::from_str(amz_date).map_err(|e| ModelError::BackendUnavailable {
            message: e.to_string(),
        })?,
    );
    headers.insert(
        HeaderName::from_static("x-amz-content-sha256"),
        HeaderValue::from_str(&payload_hash).map_err(|e| ModelError::BackendUnavailable {
            message: e.to_string(),
        })?,
    );
    if let Some(st) = session_token {
        headers.insert(
            HeaderName::from_static("x-amz-security-token"),
            HeaderValue::from_str(st).map_err(|e| ModelError::BackendUnavailable {
                message: e.to_string(),
            })?,
        );
    }
    headers.insert(
        reqwest::header::AUTHORIZATION,
        HeaderValue::from_str(&auth).map_err(|e| ModelError::BackendUnavailable {
            message: e.to_string(),
        })?,
    );
    Ok(headers)
}

/// Extracts JSON payload bytes from one AWS event-stream frame.
fn payload_from_event_frame(frame: &[u8]) -> Option<&[u8]> {
    if frame.len() < 4 {
        return None;
    }
    let total = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if frame.len() < 4 + total {
        return None;
    }
    let body = &frame[4..4 + total];
    if body.len() < 12 {
        return None;
    }
    let hl = u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize;
    let payload_start = 8 + hl;
    if body.len() < payload_start + 4 {
        return None;
    }
    let payload_end = body.len() - 4;
    if payload_start > payload_end {
        return None;
    }
    Some(&body[payload_start..payload_end])
}

fn bedrock_chunk_b64(outer: &Value) -> Option<&str> {
    outer
        .pointer("/chunk/bytes")
        .or_else(|| outer.get("bytes"))
        .and_then(|v| v.as_str())
}

struct BedrockInner {
    region: String,
    model_id: String,
    access_key_id: Secret<String>,
    secret_access_key: Secret<String>,
    session_token: Option<Secret<String>>,
    client: reqwest::Client,
    name_buf: String,
    context_window: usize,
    endpoint_base: Option<String>,
}

/// Amazon Bedrock Runtime client (SigV4, Claude Messages JSON, Anthropic-style stream events).
///
/// Credentials are stored with [`Secret`] and this type does **not** implement [`std::fmt::Debug`].
pub struct BedrockBackend {
    inner: Arc<BedrockInner>,
    attempt_observer: Arc<RwLock<Option<Arc<dyn AttemptObserver>>>>,
}

impl BedrockBackend {
    /// Builds a backend with explicit long-lived or temporary AWS keys.
    pub fn new(
        region: String,
        model_id: String,
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let name_buf = format!("bedrock/{model_id}");
        let context_window = infer_context_window_tokens(&model_id);
        Self {
            inner: Arc::new(BedrockInner {
                region,
                model_id,
                access_key_id: Secret::new(access_key_id),
                secret_access_key: Secret::new(secret_access_key),
                session_token: session_token.map(Secret::new),
                client,
                name_buf,
                context_window,
                endpoint_base: None,
            }),
            attempt_observer: Arc::new(RwLock::new(None)),
        }
    }

    #[cfg(test)]
    fn with_endpoint_base(mut self, endpoint_base: String) -> Self {
        let mut inner = BedrockInner {
            region: self.inner.region.clone(),
            model_id: self.inner.model_id.clone(),
            access_key_id: Secret::new(self.inner.access_key_id.expose_secret().clone()),
            secret_access_key: Secret::new(self.inner.secret_access_key.expose_secret().clone()),
            session_token: self
                .inner
                .session_token
                .as_ref()
                .map(|s| Secret::new(s.expose_secret().clone())),
            client: self.inner.client.clone(),
            name_buf: self.inner.name_buf.clone(),
            context_window: self.inner.context_window,
            endpoint_base: Some(endpoint_base),
        };
        if inner
            .endpoint_base
            .as_ref()
            .is_some_and(|s| s.ends_with('/'))
        {
            inner.endpoint_base = inner
                .endpoint_base
                .as_ref()
                .map(|s| s.trim_end_matches('/').to_owned());
        }
        self.inner = Arc::new(inner);
        self
    }

    /// Reads `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and optional `AWS_SESSION_TOKEN`.
    pub fn from_env(region: String, model_id: String) -> Option<Self> {
        let ak = std::env::var("AWS_ACCESS_KEY_ID").ok()?;
        let sk = std::env::var("AWS_SECRET_ACCESS_KEY").ok()?;
        let st = std::env::var("AWS_SESSION_TOKEN").ok();
        Some(Self::new(region, model_id, ak, sk, st))
    }

    /// Configured region (e.g. `us-east-1`).
    pub fn region(&self) -> &str {
        &self.inner.region
    }

    /// Bedrock `modelId` path segment (may contain `:`).
    pub fn model_id(&self) -> &str {
        &self.inner.model_id
    }

    /// Canonical URI path used in SigV4 (encoded model id).
    pub fn canonical_uri_path(&self) -> String {
        let enc = urlencoding::encode(&self.inner.model_id);
        format!("/model/{enc}/invoke-with-response-stream")
    }

    fn observer(&self) -> Option<Arc<dyn AttemptObserver>> {
        self.attempt_observer
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[derive(Debug, Default, serde::Serialize)]
struct BedrockSynthesizedResponse {
    text: String,
    tool_calls: Vec<ModelToolCall>,
    stop_reason: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct BedrockStreamEventChunk {
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

fn stream_event_chunk(event: &StreamEvent) -> BedrockStreamEventChunk {
    match event {
        StreamEvent::ProviderReady { provider, model } => BedrockStreamEventChunk {
            kind: "provider_ready",
            provider: Some(provider.clone()),
            model: Some(model.clone()),
            message: None,
            text: None,
            stop_reason: None,
            tool_calls: None,
        },
        StreamEvent::StatusHint { message } => BedrockStreamEventChunk {
            kind: "status_hint",
            provider: None,
            model: None,
            message: Some(message.clone()),
            text: None,
            stop_reason: None,
            tool_calls: None,
        },
        StreamEvent::TextDelta { text } => BedrockStreamEventChunk {
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
        } => BedrockStreamEventChunk {
            kind: "done",
            provider: None,
            model: None,
            message: None,
            text: None,
            stop_reason: Some(stop_reason_label(stop_reason).to_owned()),
            tool_calls: Some(tool_calls.clone()),
        },
        StreamEvent::UsageReport(_) => BedrockStreamEventChunk {
            kind: "usage_report",
            provider: None,
            model: None,
            message: None,
            text: None,
            stop_reason: None,
            tool_calls: None,
        },
        StreamEvent::Error { error } => BedrockStreamEventChunk {
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

fn accumulate_response(event: &StreamEvent, response: &mut BedrockSynthesizedResponse) {
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

async fn read_next_frame<S>(
    acc: &mut Vec<u8>,
    stream: &mut S,
) -> Result<Option<Vec<u8>>, ModelError>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
    loop {
        if acc.len() >= 4 {
            let total = u32::from_be_bytes([acc[0], acc[1], acc[2], acc[3]]) as usize;
            let need = 4 + total;
            if acc.len() >= need {
                let frame = acc[..need].to_vec();
                acc.drain(..need);
                return Ok(Some(frame));
            }
        }
        match stream.next().await {
            None => {
                if acc.is_empty() {
                    return Ok(None);
                }
                return Err(ModelError::StreamInterrupted {
                    message: "truncated Bedrock event stream".into(),
                });
            }
            Some(Err(e)) => {
                return Err(ModelError::StreamInterrupted {
                    message: e.to_string(),
                });
            }
            Some(Ok(b)) => acc.extend_from_slice(&b),
        }
    }
}

async fn run_bedrock_response_stream(
    inner: Arc<BedrockInner>,
    observer: Option<Arc<dyn AttemptObserver>>,
    request_body_for_hash: Value,
    body: Vec<u8>,
    config: CompletionConfig,
    tx: mpsc::Sender<Result<StreamEvent, ModelError>>,
) {
    let host = format!(
        "bedrock-runtime.{region}.amazonaws.com",
        region = inner.region
    );
    let canonical_uri = {
        let enc = urlencoding::encode(&inner.model_id);
        format!("/model/{enc}/invoke-with-response-stream")
    };
    let url = if let Some(base) = inner.endpoint_base.as_ref() {
        format!("{base}{canonical_uri}")
    } else {
        format!("https://{host}{canonical_uri}")
    };

    let now = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();

    let started_at = time::OffsetDateTime::now_utc();
    let request_hash = if let Some(obs) = observer.as_ref() {
        // Hash determinism here depends on build_bedrock_anthropic_invoke_json
        // producing consistent serde_json::Value structure across invocations
        // for identical inputs. Verified by t_bedrock_request_canonical_cbor_is_deterministic.
        let request_bytes = match canonical_cbor_bytes(&request_body_for_hash) {
            Ok(bytes) => bytes,
            Err(err) => {
                let ended_at = time::OffsetDateTime::now_utc();
                obs.record_attempt(AttemptRecord {
                    attempt_number: 1,
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
                    attempt_number: 1,
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

    let headers = match authorization_header(
        "POST",
        &host,
        &canonical_uri,
        &inner.region,
        &body,
        inner.access_key_id.expose_secret(),
        inner.secret_access_key.expose_secret(),
        inner
            .session_token
            .as_ref()
            .map(|s| s.expose_secret().as_str()),
        &amz_date,
        &date_stamp,
    ) {
        Ok(h) => h,
        Err(e) => {
            if let Some(request_hash) = request_hash
                && let Some(obs) = observer.as_ref()
            {
                let ended_at = time::OffsetDateTime::now_utc();
                obs.record_attempt(AttemptRecord {
                    attempt_number: 1,
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
    };

    let resp = match inner
        .client
        .post(&url)
        .headers(headers)
        .body(body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let err = ModelError::BackendUnavailable {
                message: e.to_string(),
            };
            if let Some(request_hash) = request_hash.clone()
                && let Some(obs) = observer.as_ref()
            {
                let ended_at = time::OffsetDateTime::now_utc();
                obs.record_attempt(AttemptRecord {
                    attempt_number: 1,
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
        let text = resp.text().await.unwrap_or_default();
        let err = match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ModelError::AuthError,
            StatusCode::TOO_MANY_REQUESTS => ModelError::RateLimited {
                retry_after_secs: None,
            },
            _ => ModelError::BackendUnavailable {
                message: format!(
                    "HTTP {status}: {snippet}",
                    snippet = &text[..text.len().min(512)]
                ),
            },
        };
        if let Some(request_hash) = request_hash.clone()
            && let Some(obs) = observer.as_ref()
        {
            let ended_at = time::OffsetDateTime::now_utc();
            obs.record_attempt(AttemptRecord {
                attempt_number: 1,
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

    let provider_ready = StreamEvent::ProviderReady {
        provider: "AWS Bedrock".into(),
        model: inner.model_id.clone(),
    };
    if let Some(obs) = observer.as_ref()
        && let Ok(bytes) = canonical_cbor_bytes(&stream_event_chunk(&provider_ready))
    {
        let _ = obs.put_object(&bytes);
    }
    let _ = tx.send(Ok(provider_ready)).await;

    let mut bytes_stream = resp.bytes_stream();
    let mut acc: Vec<u8> = Vec::new();
    let mut tool_builds: BTreeMap<usize, ToolAccum> = BTreeMap::new();
    let mut finished_tools: BTreeMap<usize, ModelToolCall> = BTreeMap::new();
    let mut usage_acc: Option<UsageReport> = None;
    let mut done_sent = false;
    let mut chunk_hashes: Vec<Hash> = Vec::new();
    let mut response_synth = BedrockSynthesizedResponse::default();

    let deadline = Duration::from_millis(config.first_token_deadline_ms);
    let first =
        match tokio::time::timeout(deadline, read_next_frame(&mut acc, &mut bytes_stream)).await {
            Err(_) => {
                let err = ModelError::FirstTokenTimeout;
                if let Some(request_hash) = request_hash.clone()
                    && let Some(obs) = observer.as_ref()
                {
                    let ended_at = time::OffsetDateTime::now_utc();
                    obs.record_attempt(AttemptRecord {
                        attempt_number: 1,
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
                        attempt_number: 1,
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
                    message: "empty Bedrock stream".into(),
                };
                if let Some(request_hash) = request_hash.clone()
                    && let Some(obs) = observer.as_ref()
                {
                    let ended_at = time::OffsetDateTime::now_utc();
                    obs.record_attempt(AttemptRecord {
                        attempt_number: 1,
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
            Ok(Ok(Some(f))) => f,
        };

    let mut pending_frame = Some(first);
    loop {
        let frame = match pending_frame.take() {
            Some(f) => f,
            None => match read_next_frame(&mut acc, &mut bytes_stream).await {
                Ok(Some(f)) => f,
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
                            attempt_number: 1,
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

        let Some(payload) = payload_from_event_frame(&frame) else {
            continue;
        };
        let payload_str = match std::str::from_utf8(payload) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let outer: Value = match serde_json::from_str(payload_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if outer
            .get("internalServerException")
            .or_else(|| outer.get("throttlingException"))
            .or_else(|| outer.get("validationException"))
            .is_some()
        {
            let err = ModelError::BackendUnavailable {
                message: format!("Bedrock: {outer}"),
            };
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
                    attempt_number: 1,
                    started_at,
                    ended_at,
                    status: map_model_error_to_attempt_status(&err),
                    request_hash,
                    response_hash,
                    stream_hash,
                    error_message: Some(err.to_string()),
                });
            }
            let _ = tx.send(Err(err)).await;
            return;
        }

        let Some(inner_chunk) = bedrock_chunk_b64(&outer) else {
            continue;
        };
        let decoded = match base64::engine::general_purpose::STANDARD.decode(inner_chunk.as_bytes())
        {
            Ok(b) => b,
            Err(_) => continue,
        };
        let inner_text = match String::from_utf8(decoded) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let v: Value = match serde_json::from_str(&inner_text) {
            Ok(x) => x,
            Err(_) => continue,
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
                        attempt_number: 1,
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
            attempt_number: 1,
            started_at,
            ended_at,
            status: AttemptStatus::Success,
            request_hash,
            response_hash,
            stream_hash,
            error_message: None,
        });
    }
}

#[async_trait]
impl LlmProvider for BedrockBackend {
    fn name(&self) -> &str {
        self.inner.name_buf.as_str()
    }

    fn context_window_tokens(&self) -> usize {
        self.inner.context_window
    }

    fn completion_model_id(&self) -> &str {
        self.inner.model_id.as_str()
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
        let body_json = build_bedrock_anthropic_invoke_json(messages, config);
        let request_body_for_hash = body_json.clone();
        let body = serde_json::to_vec(&body_json).map_err(|e| ModelError::BackendUnavailable {
            message: e.to_string(),
        })?;
        let (tx, rx) = mpsc::channel(64);
        let inner = Arc::clone(&self.inner);
        let cfg = config.clone();
        let observer = self.observer();
        tokio::spawn(run_bedrock_response_stream(
            inner,
            observer,
            request_body_for_hash,
            body,
            cfg,
            tx,
        ));
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journaling::JournalingProvider;
    use akmon_journal::{
        EventKind, HashAlgorithm, MemoryObjectStore, MemorySessionGraph, ObjectStore, SessionGraph,
        digest_bytes,
    };
    use futures::StreamExt;
    use mockito::Server;
    use std::sync::Mutex;

    fn bedrock_event_frame(payload: &[u8]) -> Vec<u8> {
        let headers_len: u32 = 0;
        let body_len = 8 + headers_len as usize + payload.len() + 4;
        let mut body = Vec::new();
        body.extend_from_slice(&headers_len.to_be_bytes());
        body.extend_from_slice(&[0_u8; 4]);
        body.extend_from_slice(payload);
        body.extend_from_slice(&[0_u8; 4]);
        let mut frame = Vec::new();
        frame.extend_from_slice(&(body_len as u32).to_be_bytes());
        frame.extend_from_slice(&body);
        frame
    }

    #[test]
    fn from_env_none_without_vars() {
        if std::env::var("AWS_ACCESS_KEY_ID").is_ok()
            || std::env::var("AWS_SECRET_ACCESS_KEY").is_ok()
        {
            return;
        }
        assert!(BedrockBackend::from_env("us-east-1".into(), "m".into()).is_none());
    }

    #[test]
    fn canonical_path_encodes_colon() {
        let b = BedrockBackend::new(
            "us-east-1".into(),
            "anthropic.claude-haiku-4-5-v1:0".into(),
            "AKIA".into(),
            "secret".into(),
            None,
        );
        let p = b.canonical_uri_path();
        assert!(p.contains("%3A"), "colon in model id must be encoded: {p}");
        assert!(p.contains("invoke-with-response-stream"));
    }

    #[test]
    fn sigv4_authorization_contains_credential_scope() {
        let headers = authorization_header(
            "POST",
            "bedrock-runtime.us-east-1.amazonaws.com",
            "/model/x%3Ay/invoke-with-response-stream",
            "us-east-1",
            b"{\"a\":1}",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            None,
            "20260106T120000Z",
            "20260106",
        )
        .expect("headers");
        let auth = headers
            .get(reqwest::header::AUTHORIZATION)
            .expect("Authorization")
            .to_str()
            .expect("str");
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential="));
        assert!(auth.contains("20260106/us-east-1/bedrock/aws4_request"));
        assert!(auth.contains("SignedHeaders="));
        assert!(auth.contains("Signature="));
    }

    #[test]
    fn event_frame_extracts_payload() {
        let hl: u32 = 0;
        let payload = b"{}";
        let body_len = 8 + hl as usize + payload.len() + 4;
        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(&hl.to_be_bytes());
        body.extend_from_slice(&[0u8; 4]);
        body.extend_from_slice(payload);
        body.extend_from_slice(&[0u8; 4]);
        let mut frame: Vec<u8> = Vec::new();
        frame.extend_from_slice(&(body_len as u32).to_be_bytes());
        frame.extend_from_slice(&body);
        let p = payload_from_event_frame(&frame).expect("payload");
        assert_eq!(p, payload);
    }

    #[test]
    fn t_bedrock_request_canonical_cbor_is_deterministic() {
        let messages = vec![Message {
            role: crate::message::MessageRole::User,
            content: "deterministic".to_owned(),
        }];
        let cfg = CompletionConfig::default();
        let req_a = build_bedrock_anthropic_invoke_json(&messages, &cfg);
        let req_b = build_bedrock_anthropic_invoke_json(&messages, &cfg);

        let cbor_a = canonical_cbor_bytes(&req_a).expect("cbor a");
        let cbor_b = canonical_cbor_bytes(&req_b).expect("cbor b");
        assert_eq!(cbor_a, cbor_b);

        let hash_a = digest_bytes(HashAlgorithm::Sha256, &cbor_a);
        let hash_b = digest_bytes(HashAlgorithm::Sha256, &cbor_b);
        assert_eq!(hash_a, hash_b);
    }

    #[tokio::test]
    async fn t_bedrock_publishes_attempt_record_with_full_hashes() {
        let mut server = Server::new_async().await;
        let model_id = "anthropic.claude-haiku-4-5-v1:0";
        let enc = urlencoding::encode(model_id);
        let path = format!("/model/{enc}/invoke-with-response-stream");

        let text1 = serde_json::json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": "one" }
        });
        let text2 = serde_json::json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": "two" }
        });
        let done = serde_json::json!({
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn" }
        });
        let outer1 = serde_json::json!({ "bytes": base64::engine::general_purpose::STANDARD.encode(text1.to_string().as_bytes()) });
        let outer2 = serde_json::json!({ "bytes": base64::engine::general_purpose::STANDARD.encode(text2.to_string().as_bytes()) });
        let outer3 = serde_json::json!({ "bytes": base64::engine::general_purpose::STANDARD.encode(done.to_string().as_bytes()) });
        let mut body = Vec::new();
        body.extend_from_slice(&bedrock_event_frame(outer1.to_string().as_bytes()));
        body.extend_from_slice(&bedrock_event_frame(outer2.to_string().as_bytes()));
        body.extend_from_slice(&bedrock_event_frame(outer3.to_string().as_bytes()));

        let _mock = server
            .mock("POST", path.as_str())
            .with_status(200)
            .with_header("content-type", "application/vnd.amazon.eventstream")
            .with_body(body)
            .create_async()
            .await;

        let backend = BedrockBackend::new(
            "us-east-1".into(),
            model_id.into(),
            "AKIA".into(),
            "secret".into(),
            None,
        )
        .with_endpoint_base(server.url());
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
            "bedrock".to_owned(),
            Arc::clone(&store),
            Arc::clone(&graph),
        );
        let messages = vec![Message {
            role: crate::message::MessageRole::User,
            content: "hello".to_owned(),
        }];
        let cfg = CompletionConfig::default();
        let mut stream = wrapped.complete(&messages, &cfg).await.expect("complete");
        while let Some(item) = stream.next().await {
            item.expect("stream item");
        }

        let history = graph
            .lock()
            .expect("graph lock")
            .history()
            .expect("history");
        let provider_call = history
            .iter()
            .find_map(|(_, event)| match &event.kind {
                EventKind::ProviderCall {
                    provider_id,
                    attempts,
                    stream_hash,
                } => Some((provider_id.clone(), attempts.clone(), stream_hash.clone())),
                _ => None,
            })
            .expect("provider call");
        assert_eq!(provider_call.1.len(), 1);
        let attempt = &provider_call.1[0];
        assert_eq!(attempt.attempt_number, 1);
        assert_eq!(attempt.status, AttemptStatus::Success);
        assert!(attempt.response_hash.is_some());
        assert!(attempt.stream_hash.is_some());
        assert!(
            store
                .contains(&attempt.request_hash)
                .expect("request in store")
        );
        assert!(
            store
                .contains(attempt.response_hash.as_ref().expect("response hash"))
                .expect("response in store")
        );
        assert!(
            store
                .contains(attempt.stream_hash.as_ref().expect("stream hash"))
                .expect("stream in store")
        );
        assert_eq!(provider_call.2, attempt.stream_hash);
    }
}
