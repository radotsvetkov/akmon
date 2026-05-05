use std::sync::{Arc, Mutex};

use akmon_journal::{AttemptRecord, AttemptStatus, Event, EventKind, Hash, ObjectStore};
use akmon_models::{
    AttemptObserver, CompletionConfig, CompletionStream, LlmProvider, Message, ModelError,
    ModelToolCall, StopReason, StreamEvent,
};
use async_trait::async_trait;
use futures::stream;
#[cfg(test)]
use serde::Serializer;
#[cfg(test)]
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

use crate::{
    ReplayDivergence, ReplayDivergenceCollector, ReplayDivergenceKind, ReplayError, ReplayMode,
};

/// Playback-provider construction options.
#[derive(Debug, Clone)]
pub struct PlaybackProviderConfig {
    /// Replay behavior mode.
    ///
    /// The mode field MUST match the corresponding mode of any [`crate::PlaybackTool`] used in
    /// the same replay run. The ReplayEngine constructs both with the same mode value;
    /// constructing them with mismatched modes (for example, for testing) results in undefined
    /// replay semantics.
    pub mode: ReplayMode,
    /// Provider identifier to extract from source `ProviderCall` events.
    pub provider_id: String,
    /// Display name returned by [`LlmProvider::name`].
    pub provider_name: String,
    /// Model id returned by [`LlmProvider::completion_model_id`].
    pub model_id: String,
    /// Context window returned by [`LlmProvider::context_window_tokens`].
    pub context_window_tokens: usize,
}

#[derive(Debug, Clone)]
struct ProviderState {
    cursor: usize,
}

#[derive(Debug, Clone)]
struct RecordedProviderCall {
    event_seq: u64,
    attempts: Vec<AttemptRecord>,
}

/// Replays recorded provider calls through the [`LlmProvider`] trait.
///
/// Strict-mode attempt replay is currently informational: one [`LlmProvider::complete`] call emits
/// status hints for failed recorded attempts before emitting the recorded successful payload.
/// This does not force the agent loop to invoke `complete()` once per recorded attempt.
///
/// TODO(Item 5.2 follow-up): If needed, add a strict-mode strategy that replays one recorded
/// attempt per `complete()` invocation so retry-path behavior is exercised through call-count parity.
pub struct PlaybackProvider<S: ObjectStore> {
    calls: Vec<RecordedProviderCall>,
    store: Arc<S>,
    divergences: ReplayDivergenceCollector,
    state: Mutex<ProviderState>,
    config: PlaybackProviderConfig,
}

/// Canonical request payload used by journaling provider hashing.
#[cfg(test)]
#[derive(Debug)]
struct RequestPayload<'a> {
    provider_id: &'a str,
    messages: &'a [Message],
    config: &'a CompletionConfig,
}

#[cfg(test)]
impl Serialize for RequestPayload<'_> {
    fn serialize<T: Serializer>(&self, serializer: T) -> Result<T::Ok, T::Error> {
        let mut s = serializer.serialize_struct("RequestPayload", 3)?;
        s.serialize_field("provider_id", &self.provider_id)?;
        s.serialize_field("messages", &self.messages)?;
        s.serialize_field("config", &ConfigPayload(self.config))?;
        s.end()
    }
}

#[cfg(test)]
struct ConfigPayload<'a>(&'a CompletionConfig);

#[cfg(test)]
impl Serialize for ConfigPayload<'_> {
    fn serialize<T: Serializer>(&self, serializer: T) -> Result<T::Ok, T::Error> {
        let config = self.0;
        let mut s = serializer.serialize_struct("CompletionConfig", 7)?;
        s.serialize_field("max_tokens", &config.max_tokens)?;
        s.serialize_field("session_id", &config.session_id)?;
        s.serialize_field("temperature", &config.temperature)?;
        s.serialize_field("first_token_deadline_ms", &config.first_token_deadline_ms)?;
        s.serialize_field("stream", &config.stream)?;
        s.serialize_field("tools", &config.tools)?;
        s.serialize_field("fallback_model", &config.fallback_model)?;
        s.end()
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct RecordedResponse {
    text: String,
    tool_calls: Vec<ModelToolCall>,
    stop_reason: Option<String>,
}

impl<S: ObjectStore> PlaybackProvider<S> {
    /// Builds provider playback state from source history.
    pub fn from_history(
        events: &[(Hash, Event)],
        store: Arc<S>,
        config: PlaybackProviderConfig,
        divergences: ReplayDivergenceCollector,
    ) -> Result<Self, ReplayError> {
        if events.is_empty() {
            return Err(ReplayError::EmptySource);
        }
        let mut calls = Vec::new();
        for (_, event) in events {
            if let EventKind::ProviderCall {
                provider_id,
                attempts,
                ..
            } = &event.kind
            {
                if provider_id != &config.provider_id {
                    continue;
                }
                if attempts.is_empty() {
                    return Err(ReplayError::MalformedSourceEvent {
                        event_seq: event.sequence,
                        reason: "ProviderCall has empty attempts".to_owned(),
                    });
                }
                for attempt in attempts {
                    ensure_present(store.as_ref(), &attempt.request_hash)?;
                    if let Some(h) = attempt.response_hash.as_ref() {
                        ensure_present(store.as_ref(), h)?;
                    }
                    if let Some(h) = attempt.stream_hash.as_ref() {
                        ensure_present(store.as_ref(), h)?;
                    }
                }
                calls.push(RecordedProviderCall {
                    event_seq: event.sequence,
                    attempts: attempts.clone(),
                });
            }
        }
        if calls.is_empty() {
            return Err(ReplayError::NoMatchingCalls(config.provider_id.clone()));
        }
        Ok(Self {
            calls,
            store,
            divergences,
            state: Mutex::new(ProviderState { cursor: 0 }),
            config,
        })
    }

    /// Current provider-call cursor index.
    pub fn cursor(&self) -> usize {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).cursor
    }

    /// Remaining recorded provider calls.
    pub fn remaining(&self) -> usize {
        let cursor = self.cursor();
        self.calls.len().saturating_sub(cursor)
    }
}

#[async_trait]
impl<S: ObjectStore + 'static> LlmProvider for PlaybackProvider<S> {
    fn name(&self) -> &str {
        self.config.provider_name.as_str()
    }

    fn context_window_tokens(&self) -> usize {
        self.config.context_window_tokens
    }

    fn completion_model_id(&self) -> &str {
        self.config.model_id.as_str()
    }

    fn set_attempt_observer(&self, _observer: Arc<dyn AttemptObserver>) {}

    async fn complete(
        &self,
        _messages: &[Message],
        _config: &CompletionConfig,
    ) -> Result<CompletionStream, ModelError> {
        let call = {
            let mut guard = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let Some(call) = self.calls.get(guard.cursor).cloned() else {
                record_divergence(
                    &self.divergences,
                    ReplayDivergence {
                        event_seq: None,
                        kind: ReplayDivergenceKind::ProviderCallUnexpected,
                        expected: "recorded provider call available".to_owned(),
                        actual: "provider called after recorded sequence exhausted".to_owned(),
                    },
                );
                return Ok(Box::pin(stream::iter([Ok(StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                })])));
            };
            guard.cursor = guard.cursor.saturating_add(1);
            call
        };
        // Per P11 (replay comparison scope), request_hash is not compared at any layer.
        // Request payloads contain runtime-variable content (session_id, environment paths)
        // that cannot be faithfully reconstructed during replay. PlaybackProvider returns
        // recorded responses without verifying request hash equivalence. Engine-level
        // comparison also excludes request_hash per the same contract.
        if matches!(self.config.mode, ReplayMode::Strict) {
            let expected_len = call.attempts.len();
            let actual_len = call.attempts.len();
            if expected_len != actual_len {
                record_divergence(
                    &self.divergences,
                    ReplayDivergence {
                        event_seq: Some(call.event_seq),
                        kind: ReplayDivergenceKind::AttemptCountDivergence,
                        expected: expected_len.to_string(),
                        actual: actual_len.to_string(),
                    },
                );
            }
        }
        let success_attempt = call
            .attempts
            .iter()
            .find(|a| matches!(a.status, AttemptStatus::Success))
            .cloned();
        let Some(success) = success_attempt else {
            record_divergence(
                &self.divergences,
                ReplayDivergence {
                    event_seq: Some(call.event_seq),
                    kind: ReplayDivergenceKind::AttemptStatusDivergence,
                    expected: "at least one Success attempt".to_owned(),
                    actual: "no Success attempt found".to_owned(),
                },
            );
            return Err(ModelError::BackendUnavailable {
                message: "recorded provider call has no successful attempt".to_owned(),
            });
        };
        let response = read_response(self.store.as_ref(), &success)?;
        let mut events = Vec::new();
        events.push(Ok(StreamEvent::ProviderReady {
            provider: self.config.provider_name.clone(),
            model: self.config.model_id.clone(),
        }));
        if matches!(self.config.mode, ReplayMode::Strict) {
            // Strict mode currently exposes retry history within a single stream as status hints.
            // It is intentionally informational for v2.0.0; it does not yet require multiple
            // complete() calls to reach the final successful payload.
            for attempt in &call.attempts {
                if !matches!(attempt.status, AttemptStatus::Success) {
                    events.push(Ok(StreamEvent::StatusHint {
                        message: format!(
                            "strict replay attempt {}: {:?}",
                            attempt.attempt_number, attempt.status
                        ),
                    }));
                }
            }
        }
        if !response.text.is_empty() {
            events.push(Ok(StreamEvent::TextDelta {
                text: response.text.clone(),
            }));
        }
        events.push(Ok(StreamEvent::Done {
            stop_reason: parse_stop_reason(response.stop_reason.as_deref()),
            tool_calls: response.tool_calls,
        }));
        Ok(Box::pin(stream::iter(events)))
    }
}

fn ensure_present<S: ObjectStore>(store: &S, hash: &Hash) -> Result<(), ReplayError> {
    match store.contains(hash) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ReplayError::MissingSourceObject(hash.clone())),
        Err(err) => Err(ReplayError::StoreReadFailed {
            hash: hash.clone(),
            reason: err.to_string(),
        }),
    }
}

#[cfg(test)]
fn request_hash<S: ObjectStore>(
    store: &S,
    provider_id: &str,
    messages: &[Message],
    config: &CompletionConfig,
) -> Result<Hash, ModelError> {
    let payload = RequestPayload {
        provider_id,
        messages,
        config,
    };
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&payload, &mut bytes).map_err(|err| {
        ModelError::BackendUnavailable {
            message: format!("replay request hash encode failed: {err}"),
        }
    })?;
    Ok(akmon_journal::digest_bytes(store.algorithm(), &bytes))
}

fn read_response<S: ObjectStore>(
    store: &S,
    attempt: &AttemptRecord,
) -> Result<RecordedResponse, ModelError> {
    let Some(hash) = attempt.response_hash.as_ref() else {
        return Ok(RecordedResponse {
            text: String::new(),
            tool_calls: Vec::new(),
            stop_reason: Some("end_turn".to_owned()),
        });
    };
    let Some(bytes) = store
        .get(hash)
        .map_err(|err| ModelError::BackendUnavailable {
            message: format!("replay response read failed: {err}"),
        })?
    else {
        return Err(ModelError::BackendUnavailable {
            message: format!("replay response object missing: {}", hash.to_hex()),
        });
    };
    ciborium::de::from_reader(bytes.as_ref()).map_err(|err| ModelError::BackendUnavailable {
        message: format!("replay response decode failed: {err}"),
    })
}

fn parse_stop_reason(value: Option<&str>) -> StopReason {
    match value {
        Some("max_tokens") => StopReason::MaxTokens,
        Some("tool_use") => StopReason::ToolUse,
        _ => StopReason::EndTurn,
    }
}

fn record_divergence(collector: &ReplayDivergenceCollector, divergence: ReplayDivergence) {
    let mut guard = collector.lock().unwrap_or_else(|p| p.into_inner());
    guard.push(divergence);
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_journal::{HashAlgorithm, MemoryObjectStore, ObjectStore};
    use akmon_models::{MessageRole, StreamEvent};
    use futures::StreamExt;
    use std::sync::{Arc, Mutex};
    use time::OffsetDateTime;

    fn collector() -> ReplayDivergenceCollector {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn provider_config(mode: ReplayMode) -> PlaybackProviderConfig {
        PlaybackProviderConfig {
            mode,
            provider_id: "mock-provider".to_owned(),
            provider_name: "Mock".to_owned(),
            model_id: "mock-model".to_owned(),
            context_window_tokens: 8192,
        }
    }

    fn sample_messages() -> Vec<Message> {
        vec![Message {
            role: MessageRole::User,
            content: "hello".to_owned(),
        }]
    }

    fn sample_event(attempts: Vec<AttemptRecord>) -> Event {
        Event {
            parents: vec![],
            kind: EventKind::ProviderCall {
                provider_id: "mock-provider".to_owned(),
                attempts,
                stream_hash: None,
            },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 1,
        }
    }

    fn response_hash<S: ObjectStore>(store: &S, text: &str) -> Hash {
        let response = RecordedResponse {
            text: text.to_owned(),
            tool_calls: Vec::new(),
            stop_reason: Some("end_turn".to_owned()),
        };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&response, &mut bytes).expect("encode");
        store.put(&bytes).expect("put response")
    }

    fn request_hash_for<S: ObjectStore>(
        store: &S,
        messages: &[Message],
        config: &CompletionConfig,
    ) -> Hash {
        let payload = RequestPayload {
            provider_id: "mock-provider",
            messages,
            config,
        };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&payload, &mut bytes).expect("encode");
        let stored = store.put(&bytes).expect("store payload");
        let computed =
            request_hash(store, "mock-provider", messages, config).expect("request hash");
        assert_eq!(stored, computed);
        computed
    }

    #[tokio::test]
    async fn playback_provider_default_returns_recorded_success_response() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let messages = sample_messages();
        let completion = CompletionConfig::default();
        let req = request_hash_for(store.as_ref(), &messages, &completion);
        let rsp = response_hash(store.as_ref(), "hello replay");
        let attempts = vec![AttemptRecord {
            attempt_number: 1,
            started_at: OffsetDateTime::UNIX_EPOCH,
            ended_at: OffsetDateTime::UNIX_EPOCH,
            status: AttemptStatus::Success,
            request_hash: req,
            response_hash: Some(rsp),
            stream_hash: None,
            error_message: None,
        }];
        let event = sample_event(attempts);
        let playback = PlaybackProvider::from_history(
            &[(Hash::from_bytes(HashAlgorithm::Sha256, [1_u8; 32]), event)],
            store,
            provider_config(ReplayMode::Default),
            collector(),
        )
        .expect("construct");
        let mut stream = playback
            .complete(&messages, &completion)
            .await
            .expect("complete");
        let mut got_text = String::new();
        while let Some(item) = stream.next().await {
            if let Ok(StreamEvent::TextDelta { text }) = item {
                got_text.push_str(&text);
            }
        }
        assert_eq!(got_text, "hello replay");
    }

    #[tokio::test]
    async fn playback_provider_strict_replays_attempt_sequence() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let messages = sample_messages();
        let completion = CompletionConfig::default();
        let req = request_hash_for(store.as_ref(), &messages, &completion);
        let rsp = response_hash(store.as_ref(), "ok");
        let attempts = vec![
            AttemptRecord {
                attempt_number: 1,
                started_at: OffsetDateTime::UNIX_EPOCH,
                ended_at: OffsetDateTime::UNIX_EPOCH,
                status: AttemptStatus::RateLimited,
                request_hash: req.clone(),
                response_hash: None,
                stream_hash: None,
                error_message: Some("429".to_owned()),
            },
            AttemptRecord {
                attempt_number: 2,
                started_at: OffsetDateTime::UNIX_EPOCH,
                ended_at: OffsetDateTime::UNIX_EPOCH,
                status: AttemptStatus::Success,
                request_hash: req,
                response_hash: Some(rsp),
                stream_hash: None,
                error_message: None,
            },
        ];
        let event = sample_event(attempts);
        let playback = PlaybackProvider::from_history(
            &[(Hash::from_bytes(HashAlgorithm::Sha256, [2_u8; 32]), event)],
            store,
            provider_config(ReplayMode::Strict),
            collector(),
        )
        .expect("construct");
        let mut stream = playback
            .complete(&messages, &completion)
            .await
            .expect("complete");
        let mut status_hints = 0usize;
        while let Some(item) = stream.next().await {
            if matches!(item, Ok(StreamEvent::StatusHint { .. })) {
                status_hints = status_hints.saturating_add(1);
            }
        }
        assert_eq!(status_hints, 1);
    }

    #[tokio::test]
    async fn playback_provider_reports_divergence_when_called_past_sequence() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let messages = sample_messages();
        let completion = CompletionConfig::default();
        let req = request_hash_for(store.as_ref(), &messages, &completion);
        let rsp = response_hash(store.as_ref(), "ok");
        let attempts = vec![AttemptRecord {
            attempt_number: 1,
            started_at: OffsetDateTime::UNIX_EPOCH,
            ended_at: OffsetDateTime::UNIX_EPOCH,
            status: AttemptStatus::Success,
            request_hash: req,
            response_hash: Some(rsp),
            stream_hash: None,
            error_message: None,
        }];
        let event = sample_event(attempts);
        let divergences = collector();
        let playback = PlaybackProvider::from_history(
            &[(Hash::from_bytes(HashAlgorithm::Sha256, [3_u8; 32]), event)],
            store,
            provider_config(ReplayMode::Default),
            Arc::clone(&divergences),
        )
        .expect("construct");
        let _ = playback
            .complete(&messages, &completion)
            .await
            .expect("first");
        let _ = playback
            .complete(&messages, &completion)
            .await
            .expect("second");
        let guard = divergences.lock().unwrap_or_else(|p| p.into_inner());
        assert!(
            guard
                .iter()
                .any(|d| matches!(d.kind, ReplayDivergenceKind::ProviderCallUnexpected))
        );
    }

    #[tokio::test]
    async fn playback_provider_cursors_advance_after_each_call_async() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let messages = sample_messages();
        let completion = CompletionConfig::default();
        let req = request_hash_for(store.as_ref(), &messages, &completion);
        let rsp_a = response_hash(store.as_ref(), "a");
        let rsp_b = response_hash(store.as_ref(), "b");
        let attempts_a = vec![AttemptRecord {
            attempt_number: 1,
            started_at: OffsetDateTime::UNIX_EPOCH,
            ended_at: OffsetDateTime::UNIX_EPOCH,
            status: AttemptStatus::Success,
            request_hash: req.clone(),
            response_hash: Some(rsp_a),
            stream_hash: None,
            error_message: None,
        }];
        let attempts_b = vec![AttemptRecord {
            attempt_number: 1,
            started_at: OffsetDateTime::UNIX_EPOCH,
            ended_at: OffsetDateTime::UNIX_EPOCH,
            status: AttemptStatus::Success,
            request_hash: req,
            response_hash: Some(rsp_b),
            stream_hash: None,
            error_message: None,
        }];
        let playback = PlaybackProvider::from_history(
            &[
                (
                    Hash::from_bytes(HashAlgorithm::Sha256, [4_u8; 32]),
                    sample_event(attempts_a),
                ),
                (
                    Hash::from_bytes(HashAlgorithm::Sha256, [5_u8; 32]),
                    sample_event(attempts_b),
                ),
            ],
            store,
            provider_config(ReplayMode::Default),
            collector(),
        )
        .expect("construct");
        assert_eq!(playback.cursor(), 0);
        let _ = playback
            .complete(&messages, &completion)
            .await
            .expect("first");
        assert_eq!(playback.cursor(), 1);
        let _ = playback
            .complete(&messages, &completion)
            .await
            .expect("second");
        assert_eq!(playback.cursor(), 2);
    }

    #[tokio::test]
    async fn t_playback_provider_does_not_emit_request_mismatch_per_p11() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let messages = sample_messages();
        let completion = CompletionConfig::default();
        let req = request_hash_for(store.as_ref(), &messages, &completion);
        let rsp = response_hash(store.as_ref(), "ok");
        let attempts = vec![AttemptRecord {
            attempt_number: 1,
            started_at: OffsetDateTime::UNIX_EPOCH,
            ended_at: OffsetDateTime::UNIX_EPOCH,
            status: AttemptStatus::Success,
            request_hash: req,
            response_hash: Some(rsp),
            stream_hash: None,
            error_message: None,
        }];
        let event = sample_event(attempts);
        let divergences = collector();
        let playback = PlaybackProvider::from_history(
            &[(Hash::from_bytes(HashAlgorithm::Sha256, [6_u8; 32]), event)],
            Arc::clone(&store),
            provider_config(ReplayMode::Default),
            Arc::clone(&divergences),
        )
        .expect("construct");
        let mismatch_completion = CompletionConfig {
            session_id: Some(uuid::Uuid::new_v4().to_string()),
            ..CompletionConfig::default()
        };
        let mut stream = playback
            .complete(&messages, &mismatch_completion)
            .await
            .expect("complete");
        let mut saw_done = false;
        while let Some(item) = stream.next().await {
            if matches!(item, Ok(StreamEvent::Done { .. })) {
                saw_done = true;
            }
        }
        assert!(saw_done);
        let guard = divergences.lock().unwrap_or_else(|p| p.into_inner());
        assert!(
            !guard
                .iter()
                .any(|d| matches!(d.kind, ReplayDivergenceKind::ProviderRequestMismatch))
        );
    }
}
