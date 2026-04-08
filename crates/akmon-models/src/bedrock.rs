//! Amazon Bedrock Runtime `InvokeModelWithResponseStream` (Claude Messages API payload) with SigV4.

use std::collections::BTreeMap;
use std::sync::Arc;
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

use crate::LlmProvider;
use crate::anthropic::{ToolAccum, apply_anthropic_sse_json, build_bedrock_anthropic_invoke_json};
use crate::config::CompletionConfig;
use crate::error::ModelError;
use crate::message::Message;
use crate::openai_compat::infer_context_window_tokens;
use crate::stream::{CompletionStream, ModelToolCall, StreamEvent, UsageReport};

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
}

/// Amazon Bedrock Runtime client (SigV4, Claude Messages JSON, Anthropic-style stream events).
///
/// Credentials are stored with [`Secret`] and this type does **not** implement [`std::fmt::Debug`].
pub struct BedrockBackend {
    inner: Arc<BedrockInner>,
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
            }),
        }
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
    let url = format!("https://{host}{canonical_uri}");

    let now = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();

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
            let _ = tx
                .send(Err(ModelError::BackendUnavailable {
                    message: e.to_string(),
                }))
                .await;
            return;
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let _ = tx
            .send(Err(match status {
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ModelError::AuthError,
                _ => ModelError::BackendUnavailable {
                    message: format!(
                        "HTTP {status}: {snippet}",
                        snippet = &text[..text.len().min(512)]
                    ),
                },
            }))
            .await;
        return;
    }

    let _ = tx
        .send(Ok(StreamEvent::ProviderReady {
            provider: "AWS Bedrock".into(),
            model: inner.model_id.clone(),
        }))
        .await;

    let mut bytes_stream = resp.bytes_stream();
    let mut acc: Vec<u8> = Vec::new();
    let mut tool_builds: BTreeMap<usize, ToolAccum> = BTreeMap::new();
    let mut finished_tools: BTreeMap<usize, ModelToolCall> = BTreeMap::new();
    let mut usage_acc: Option<UsageReport> = None;
    let mut done_sent = false;

    let deadline = Duration::from_millis(config.first_token_deadline_ms);
    let first =
        match tokio::time::timeout(deadline, read_next_frame(&mut acc, &mut bytes_stream)).await {
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
                        message: "empty Bedrock stream".into(),
                    }))
                    .await;
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
            let _ = tx
                .send(Err(ModelError::BackendUnavailable {
                    message: format!("Bedrock: {outer}"),
                }))
                .await;
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
        if let Some(ref snap) = usage_acc {
            let _ = tx.send(Ok(StreamEvent::UsageReport(snap.clone()))).await;
        }
        let _ = tx
            .send(Ok(StreamEvent::Done {
                stop_reason: crate::stream::StopReason::EndTurn,
                tool_calls: vec![],
            }))
            .await;
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

    async fn complete(
        &self,
        messages: &[Message],
        config: &CompletionConfig,
    ) -> Result<CompletionStream, ModelError> {
        let body_json = build_bedrock_anthropic_invoke_json(messages, config);
        let body = serde_json::to_vec(&body_json).map_err(|e| ModelError::BackendUnavailable {
            message: e.to_string(),
        })?;
        let (tx, rx) = mpsc::channel(64);
        let inner = Arc::clone(&self.inner);
        let cfg = config.clone();
        tokio::spawn(run_bedrock_response_stream(inner, body, cfg, tx));
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
