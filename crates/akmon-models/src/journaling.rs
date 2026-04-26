//! Journaling wrapper for [`crate::LlmProvider`] implementations.
//!
//! `JournalingProvider` connects model calls to the AGEF journal substrate by capturing
//! logical request/response evidence and appending provider-call events. It is designed
//! to preserve the per-attempt evidence contract from D-17: one [`AttemptRecord`] per
//! HTTP attempt, including retries that are otherwise hidden inside backend loops.
//!
//! Backends publish attempt-level data through an observer pattern. This module defines
//! [`AttemptObserver`] for that integration and includes an internal collector that
//! buffers attempts during one logical `complete()` call.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use akmon_journal::{
    AttemptRecord, AttemptStatus, EventKind, Hash, JournalError, ObjectStore, SessionGraph,
    digest_bytes,
};
use async_trait::async_trait;
use futures::Stream;
use pin_project_lite::pin_project;
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

use crate::{
    CompletionConfig, CompletionStream, LlmProvider, Message, ModelError, ModelToolCall,
    StopReason, StreamEvent, UsageReport,
};

/// Receives one [`AttemptRecord`] per HTTP attempt within a logical
/// [`crate::LlmProvider::complete`] call.
///
/// Used by [`JournalingProvider`] to capture retry history that would otherwise be
/// invisible to the wrapper.
pub trait AttemptObserver: Send + Sync {
    /// Records one provider HTTP/model attempt.
    fn record_attempt(&self, attempt: AttemptRecord);

    /// Stores the given bytes in the underlying object store and returns the resulting hash.
    ///
    /// The returned [`Hash`] uses the store's configured algorithm (SHA-256 default, BLAKE3
    /// optional per AGEF v0.1).
    ///
    /// Backends MUST use this method to obtain hashes for [`AttemptRecord`] fields
    /// (`request_hash`, `response_hash`, etc.). Backends MUST NOT independently compute hashes
    /// via hashing crates, because the store's algorithm is authoritative and direct computation
    /// may produce hashes that do not resolve in the store.
    fn put_object(&self, bytes: &[u8]) -> Result<Hash, JournalError>;
}

/// Wraps any [`LlmProvider`] to capture per-call evidence into an AGEF-compatible journal
/// substrate.
///
/// Each call to [`LlmProvider::complete`] produces:
/// - one or more [`akmon_journal::AttemptRecord`] entries (one per HTTP attempt, including
///   retries) when the inner provider implements [`LlmProvider::set_attempt_observer`].
/// - exactly one synthesized [`akmon_journal::AttemptRecord`] covering the full call when the
///   inner provider does not support observation.
/// - one [`akmon_journal::EventKind::ProviderCall`] event in the session graph at the end of the
///   call.
///
/// The wrapper enforces synchronous append semantics: when the returned [`CompletionStream`] yields
/// its terminal item, the `ProviderCall` event is already in the graph.
///
/// See the AGEF specification at <https://github.com/radotsvetkov/agef> for the journal format.
///
/// A mutex is required around the graph because [`LlmProvider::complete`] takes `&self` while
/// [`SessionGraph::append`] requires `&mut self`. The graph lock is held briefly per append and
/// never across awaits while consuming the inner provider stream.
#[allow(dead_code)]
pub struct JournalingProvider<P, S, G>
where
    P: LlmProvider,
    S: ObjectStore,
    G: SessionGraph,
{
    inner: P,
    store: Arc<S>,
    graph: Arc<Mutex<G>>,
    provider_id: String,
}

impl<P, S, G> JournalingProvider<P, S, G>
where
    P: LlmProvider,
    S: ObjectStore,
    G: SessionGraph,
{
    /// Creates a new journaling wrapper.
    pub fn new(inner: P, provider_id: String, store: Arc<S>, graph: Arc<Mutex<G>>) -> Self {
        Self {
            inner,
            store,
            graph,
            provider_id,
        }
    }
}

#[async_trait]
impl<P, S, G> LlmProvider for JournalingProvider<P, S, G>
where
    P: LlmProvider,
    S: ObjectStore + 'static,
    G: SessionGraph + 'static,
{
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn context_window_tokens(&self) -> usize {
        self.inner.context_window_tokens()
    }

    fn completion_model_id(&self) -> &str {
        self.inner.completion_model_id()
    }

    fn estimate_tokens(&self, messages: &[Message]) -> Option<usize> {
        self.inner.estimate_tokens(messages)
    }

    async fn complete(
        &self,
        messages: &[Message],
        config: &CompletionConfig,
    ) -> Result<CompletionStream, ModelError> {
        let collector = Arc::new(AttemptCollector::new(self.store.clone()));
        let observer: Arc<dyn AttemptObserver> = collector.clone();
        self.inner.set_attempt_observer(observer);
        let started_at = time::OffsetDateTime::now_utc();
        let request_payload = RequestPayload {
            provider_id: self.provider_id.as_str(),
            messages,
            config,
        };
        let request_bytes = canonical_cbor_bytes(&request_payload)?;
        let request_hash = digest_bytes(self.store.algorithm(), &request_bytes);
        put_bytes(self.store.as_ref(), &request_bytes)?;

        let inner_stream = match self.inner.complete(messages, config).await {
            Ok(stream) => stream,
            Err(err) => {
                let ended_at = time::OffsetDateTime::now_utc();
                let attempt = AttemptRecord {
                    attempt_number: 1,
                    started_at,
                    ended_at,
                    status: map_model_error_to_attempt_status(&err),
                    request_hash,
                    response_hash: None,
                    stream_hash: None,
                    error_message: Some(err.to_string()),
                };
                append_provider_call(
                    self.graph.clone(),
                    self.provider_id.clone(),
                    vec![attempt],
                    None,
                )?;
                return Err(err);
            }
        };

        Ok(Box::pin(JournalingStream::new(
            inner_stream,
            self.store.clone(),
            self.graph.clone(),
            self.provider_id.clone(),
            request_hash,
            started_at,
            collector,
        )))
    }
}

pin_project! {
    struct JournalingStream<S, G>
    where
        S: ObjectStore,
        G: SessionGraph,
    {
        #[pin]
        inner: CompletionStream,
        store: Arc<S>,
        graph: Arc<Mutex<G>>,
        provider_id: String,
        request_hash: Hash,
        started_at: time::OffsetDateTime,
        collector: Arc<AttemptCollector<S>>,
        chunk_hashes: Vec<Hash>,
        response: SynthesizedResponse,
        last_error: Option<ModelError>,
        done_seen: bool,
        finalized: bool,
    }
}

impl<S, G> JournalingStream<S, G>
where
    S: ObjectStore,
    G: SessionGraph,
{
    fn new(
        inner: CompletionStream,
        store: Arc<S>,
        graph: Arc<Mutex<G>>,
        provider_id: String,
        request_hash: Hash,
        started_at: time::OffsetDateTime,
        collector: Arc<AttemptCollector<S>>,
    ) -> Self {
        Self {
            inner,
            store,
            graph,
            provider_id,
            request_hash,
            started_at,
            collector,
            chunk_hashes: Vec::new(),
            response: SynthesizedResponse::default(),
            last_error: None,
            done_seen: false,
            finalized: false,
        }
    }

    fn finalize(&mut self) -> Result<(), ModelError> {
        if self.finalized {
            return Ok(());
        }
        self.finalized = true;
        let ended_at = time::OffsetDateTime::now_utc();
        let drained = self.collector.drain();
        let (attempts, stream_hash) = if drained.is_empty() {
            let response_hash = response_hash_for_final(&self.response, self.store.as_ref())?;
            let stream_hash =
                stream_hash_for_chunks(self.chunk_hashes.as_slice(), self.store.as_ref())?;
            (
                synthesized_attempts_if_empty(
                    drained,
                    self.request_hash.clone(),
                    response_hash,
                    stream_hash.clone(),
                    self.started_at,
                    ended_at,
                    self.last_error.as_ref(),
                ),
                stream_hash,
            )
        } else {
            let stream_hash = drained
                .iter()
                .rev()
                .find(|a| matches!(a.status, AttemptStatus::Success))
                .and_then(|a| a.stream_hash.clone());
            (drained, stream_hash)
        };
        append_provider_call(
            Arc::clone(&self.graph),
            self.provider_id.clone(),
            attempts,
            stream_hash,
        )?;
        Ok(())
    }
}

impl<S, G> Stream for JournalingStream<S, G>
where
    S: ObjectStore,
    G: SessionGraph,
{
    type Item = Result<StreamEvent, ModelError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done_seen {
            if !self.finalized
                && let Err(err) = self.as_mut().get_mut().finalize()
            {
                self.finalized = true;
                return Poll::Ready(Some(Err(err)));
            }
            return Poll::Ready(None);
        }

        let polled = {
            let mut this = self.as_mut().project();
            this.inner.as_mut().poll_next(cx)
        };

        match polled {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => {
                self.done_seen = true;
                if !self.finalized
                    && let Err(err) = self.as_mut().get_mut().finalize()
                {
                    self.finalized = true;
                    return Poll::Ready(Some(Err(err)));
                }
                Poll::Ready(None)
            }
            Poll::Ready(Some(item)) => {
                let this = self.as_mut().get_mut();
                match &item {
                    Ok(event) => {
                        match store_stream_event_chunk(this.store.as_ref(), event) {
                            Ok(hash) => this.chunk_hashes.push(hash),
                            Err(err) => {
                                this.last_error = Some(err.clone());
                                this.done_seen = true;
                                return Poll::Ready(Some(Err(err)));
                            }
                        }
                        accumulate_response(event, &mut this.response);
                        if matches!(event, StreamEvent::Done { .. }) {
                            this.done_seen = true;
                        }
                    }
                    Err(err) => {
                        this.last_error = Some(err.clone());
                        this.done_seen = true;
                    }
                }
                Poll::Ready(Some(item))
            }
        }
    }
}

#[derive(Debug)]
struct RequestPayload<'a> {
    provider_id: &'a str,
    messages: &'a [Message],
    config: &'a CompletionConfig,
}

impl Serialize for RequestPayload<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("RequestPayload", 3)?;
        s.serialize_field("provider_id", &self.provider_id)?;
        s.serialize_field("messages", &self.messages)?;
        s.serialize_field("config", &ConfigPayload(self.config))?;
        s.end()
    }
}

struct ConfigPayload<'a>(&'a CompletionConfig);

impl Serialize for ConfigPayload<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
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

#[derive(Debug, Default, Serialize)]
struct SynthesizedResponse {
    text: String,
    tool_calls: Vec<ModelToolCall>,
    stop_reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct StreamEventChunk {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<UsageReport>,
}

fn canonical_cbor_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, ModelError> {
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(value, &mut bytes).map_err(|err| ModelError::StreamInterrupted {
        message: format!("canonical cbor encode failed: {err}"),
    })?;
    Ok(bytes)
}

fn put_bytes<S: ObjectStore>(store: &S, bytes: &[u8]) -> Result<Hash, ModelError> {
    store
        .put(bytes)
        .map_err(|err| ModelError::BackendUnavailable {
            message: format!("journal object store put failed: {err}"),
        })
}

fn store_stream_event_chunk<S: ObjectStore>(
    store: &S,
    event: &StreamEvent,
) -> Result<Hash, ModelError> {
    let chunk = stream_event_chunk(event);
    let bytes = canonical_cbor_bytes(&chunk)?;
    put_bytes(store, &bytes)
}

fn stream_event_chunk(event: &StreamEvent) -> StreamEventChunk {
    match event {
        StreamEvent::ProviderReady { provider, model } => StreamEventChunk {
            kind: "provider_ready",
            provider: Some(provider.clone()),
            model: Some(model.clone()),
            message: None,
            text: None,
            stop_reason: None,
            tool_calls: None,
            usage: None,
        },
        StreamEvent::StatusHint { message } => StreamEventChunk {
            kind: "status_hint",
            provider: None,
            model: None,
            message: Some(message.clone()),
            text: None,
            stop_reason: None,
            tool_calls: None,
            usage: None,
        },
        StreamEvent::TextDelta { text } => StreamEventChunk {
            kind: "text_delta",
            provider: None,
            model: None,
            message: None,
            text: Some(text.clone()),
            stop_reason: None,
            tool_calls: None,
            usage: None,
        },
        StreamEvent::Done {
            stop_reason,
            tool_calls,
        } => StreamEventChunk {
            kind: "done",
            provider: None,
            model: None,
            message: None,
            text: None,
            stop_reason: Some(stop_reason_label(stop_reason).to_owned()),
            tool_calls: Some(tool_calls.clone()),
            usage: None,
        },
        StreamEvent::UsageReport(usage) => StreamEventChunk {
            kind: "usage_report",
            provider: None,
            model: None,
            message: None,
            text: None,
            stop_reason: None,
            tool_calls: None,
            usage: Some(usage.clone()),
        },
        StreamEvent::Error { error } => StreamEventChunk {
            kind: "error",
            provider: None,
            model: None,
            message: Some(error.to_string()),
            text: None,
            stop_reason: None,
            tool_calls: None,
            usage: None,
        },
    }
}

fn stop_reason_label(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::ToolUse => "tool_use",
    }
}

fn accumulate_response(event: &StreamEvent, response: &mut SynthesizedResponse) {
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

fn response_hash_for_final<S: ObjectStore>(
    response: &SynthesizedResponse,
    store: &S,
) -> Result<Option<Hash>, ModelError> {
    let has_response = !response.text.is_empty()
        || !response.tool_calls.is_empty()
        || response.stop_reason.is_some();
    if !has_response {
        return Ok(None);
    }
    let bytes = canonical_cbor_bytes(response)?;
    Ok(Some(put_bytes(store, &bytes)?))
}

fn stream_hash_for_chunks<S: ObjectStore>(
    chunk_hashes: &[Hash],
    store: &S,
) -> Result<Option<Hash>, ModelError> {
    if chunk_hashes.is_empty() {
        return Ok(None);
    }
    let bytes = canonical_cbor_bytes(chunk_hashes)?;
    Ok(Some(put_bytes(store, &bytes)?))
}

fn synthesized_attempts_if_empty(
    attempts: Vec<AttemptRecord>,
    request_hash: Hash,
    response_hash: Option<Hash>,
    stream_hash: Option<Hash>,
    started_at: time::OffsetDateTime,
    ended_at: time::OffsetDateTime,
    last_error: Option<&ModelError>,
) -> Vec<AttemptRecord> {
    if !attempts.is_empty() {
        return attempts;
    }
    // TODO(Item 2.1 Layer 7): emit tracing::warn when this fallback is used for
    // providers that should publish attempt observer records.
    let status = match last_error {
        Some(err) => map_model_error_to_attempt_status(err),
        None => AttemptStatus::Success,
    };
    let error_message = last_error.map(std::string::ToString::to_string);
    vec![AttemptRecord {
        attempt_number: 1,
        started_at,
        ended_at,
        status,
        request_hash,
        response_hash,
        stream_hash,
        error_message,
    }]
}

fn map_model_error_to_attempt_status(err: &ModelError) -> AttemptStatus {
    match err {
        ModelError::RateLimited { .. } => AttemptStatus::RateLimited,
        // First-token timeout is transport/network path failure, not caller cancellation.
        ModelError::FirstTokenTimeout => AttemptStatus::NetworkError,
        ModelError::BackendUnavailable { .. } => AttemptStatus::ServerError,
        ModelError::AuthError
        | ModelError::ContextWindowExceeded
        | ModelError::ModelNotFound { .. } => AttemptStatus::ClientError,
        ModelError::StreamInterrupted { .. } => AttemptStatus::NetworkError,
    }
}

fn append_provider_call<G: SessionGraph>(
    graph: Arc<Mutex<G>>,
    provider_id: String,
    attempts: Vec<AttemptRecord>,
    stream_hash: Option<Hash>,
) -> Result<(), ModelError> {
    let mut guard = graph
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .append(EventKind::ProviderCall {
            provider_id,
            attempts,
            stream_hash,
        })
        .map(|_| ())
        .map_err(|err| ModelError::BackendUnavailable {
            message: format!("journal graph append failed: {err}"),
        })
}

/// Collects [`AttemptRecord`] values during one `complete()` call.
///
/// Implements [`AttemptObserver`], buffering attempts in an internal mutex-protected
/// vector. The wrapper drains this buffer after the inner stream terminates.
#[allow(dead_code)]
struct AttemptCollector<S: ObjectStore> {
    store: Arc<S>,
    attempts: Mutex<Vec<AttemptRecord>>,
}

#[allow(dead_code)]
impl<S: ObjectStore> AttemptCollector<S> {
    fn new(store: Arc<S>) -> Self {
        Self {
            store,
            attempts: Mutex::new(Vec::new()),
        }
    }

    /// Drains buffered attempts and leaves the collector empty.
    fn drain(&self) -> Vec<AttemptRecord> {
        let mut guard = self.attempts.lock().unwrap_or_else(|poisoned| {
            // Recover from poisoning so subsequent attempts are not dropped.
            poisoned.into_inner()
        });
        std::mem::take(&mut *guard)
    }

    /// Returns the current buffered attempt count.
    fn len(&self) -> usize {
        let guard = self.attempts.lock().unwrap_or_else(|poisoned| {
            // Recover from poisoning so callers can still inspect state.
            poisoned.into_inner()
        });
        guard.len()
    }
}

impl<S: ObjectStore> AttemptObserver for AttemptCollector<S> {
    fn record_attempt(&self, attempt: AttemptRecord) {
        let mut guard = self.attempts.lock().unwrap_or_else(|poisoned| {
            // Mutex poisoning is recovered to avoid losing attempts; poisoning indicates
            // a logic bug elsewhere in this type.
            poisoned.into_inner()
        });
        guard.push(attempt);
    }

    fn put_object(&self, bytes: &[u8]) -> Result<Hash, JournalError> {
        self.store.put(bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, RwLock};
    use std::thread;

    use akmon_journal::{
        EventKind, Hash, HashAlgorithm, MemoryObjectStore, MemorySessionGraph, ObjectStore,
        SessionGraph, digest_bytes,
    };
    use async_trait::async_trait;
    use futures::StreamExt;

    use super::{AttemptCollector, AttemptObserver, JournalingProvider};
    use crate::{
        CompletionConfig, CompletionStream, LlmProvider, Message, ModelError, StreamEvent,
    };

    struct MockProvider {
        provider_name: &'static str,
        context_window: usize,
        model_id: &'static str,
        estimate: Option<usize>,
        events: Vec<Result<StreamEvent, ModelError>>,
    }

    struct InstrumentedMockProvider {
        observer: RwLock<Option<Arc<dyn AttemptObserver>>>,
        events: Vec<Result<StreamEvent, ModelError>>,
    }

    type WrappedHandles = (
        JournalingProvider<MockProvider, MemoryObjectStore, MemorySessionGraph>,
        Arc<MemoryObjectStore>,
        Arc<Mutex<MemorySessionGraph>>,
    );
    type InstrumentedWrappedHandles = (
        JournalingProvider<InstrumentedMockProvider, MemoryObjectStore, MemorySessionGraph>,
        Arc<MemoryObjectStore>,
        Arc<Mutex<MemorySessionGraph>>,
    );

    fn sample_attempt(attempt_number: u32) -> akmon_journal::AttemptRecord {
        akmon_journal::AttemptRecord {
            attempt_number,
            started_at: time::OffsetDateTime::UNIX_EPOCH,
            ended_at: time::OffsetDateTime::UNIX_EPOCH,
            status: akmon_journal::AttemptStatus::Success,
            request_hash: akmon_journal::Hash::from_bytes(HashAlgorithm::Sha256, [0_u8; 32]),
            response_hash: None,
            stream_hash: None,
            error_message: None,
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &str {
            self.provider_name
        }

        fn context_window_tokens(&self) -> usize {
            self.context_window
        }

        fn completion_model_id(&self) -> &str {
            self.model_id
        }

        fn estimate_tokens(&self, _messages: &[Message]) -> Option<usize> {
            self.estimate
        }

        async fn complete(
            &self,
            _messages: &[Message],
            _config: &CompletionConfig,
        ) -> Result<CompletionStream, ModelError> {
            Ok(Box::pin(futures::stream::iter(self.events.clone())))
        }
    }

    #[async_trait]
    impl LlmProvider for InstrumentedMockProvider {
        fn name(&self) -> &str {
            "instrumented-mock"
        }

        fn context_window_tokens(&self) -> usize {
            8192
        }

        fn completion_model_id(&self) -> &str {
            "instrumented-model"
        }

        fn set_attempt_observer(&self, observer: Arc<dyn AttemptObserver>) {
            let mut slot = self
                .observer
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *slot = Some(observer);
        }

        async fn complete(
            &self,
            _messages: &[Message],
            _config: &CompletionConfig,
        ) -> Result<CompletionStream, ModelError> {
            let observer = self
                .observer
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if let Some(obs) = observer {
                obs.record_attempt(sample_attempt(1));
                obs.record_attempt(sample_attempt(2));
            }
            Ok(Box::pin(futures::stream::iter(self.events.clone())))
        }
    }

    fn wrapped_with_mock(
        mock: MockProvider,
    ) -> JournalingProvider<MockProvider, MemoryObjectStore, MemorySessionGraph> {
        let (wrapped, _, _) = wrapped_with_mock_handles(mock);
        wrapped
    }

    fn wrapped_with_mock_handles(mock: MockProvider) -> WrappedHandles {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(Arc::clone(&store), uuid::Uuid::new_v4());
        let cwd_hash = store.put(b"cwd").expect("cwd hash");
        let config_hash = store.put(b"config").expect("config hash");
        graph
            .append(EventKind::SessionStart {
                cwd_hash,
                config_hash,
            })
            .expect("session start");
        let graph = Arc::new(Mutex::new(graph));
        let wrapped = JournalingProvider::new(
            mock,
            "mock-provider".to_owned(),
            Arc::clone(&store),
            Arc::clone(&graph),
        );
        (wrapped, store, graph)
    }

    fn wrapped_with_instrumented_handles(
        provider: InstrumentedMockProvider,
    ) -> InstrumentedWrappedHandles {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(Arc::clone(&store), uuid::Uuid::new_v4());
        let cwd_hash = store.put(b"cwd").expect("cwd hash");
        let config_hash = store.put(b"config").expect("config hash");
        graph
            .append(EventKind::SessionStart {
                cwd_hash,
                config_hash,
            })
            .expect("session start");
        let graph = Arc::new(Mutex::new(graph));
        let wrapped = JournalingProvider::new(
            provider,
            "mock-provider".to_owned(),
            Arc::clone(&store),
            Arc::clone(&graph),
        );
        (wrapped, store, graph)
    }

    #[test]
    fn t_passthrough_name() {
        let wrapped = wrapped_with_mock(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: None,
            events: vec![],
        });
        assert_eq!(wrapped.name(), "mock");
    }

    #[test]
    fn t_passthrough_context_window() {
        let wrapped = wrapped_with_mock(MockProvider {
            provider_name: "mock",
            context_window: 4096,
            model_id: "m",
            estimate: None,
            events: vec![],
        });
        assert_eq!(wrapped.context_window_tokens(), 4096);
    }

    #[test]
    fn t_passthrough_completion_model_id() {
        let wrapped = wrapped_with_mock(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "mock-model-v1",
            estimate: None,
            events: vec![],
        });
        assert_eq!(wrapped.completion_model_id(), "mock-model-v1");
    }

    #[test]
    fn t_passthrough_estimate_tokens() {
        let msg = vec![Message {
            role: crate::MessageRole::User,
            content: "hello".to_owned(),
        }];
        let wrapped_none = wrapped_with_mock(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: None,
            events: vec![],
        });
        assert_eq!(wrapped_none.estimate_tokens(&msg), None);

        let wrapped_some = wrapped_with_mock(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: Some(123),
            events: vec![],
        });
        assert_eq!(wrapped_some.estimate_tokens(&msg), Some(123));
    }

    #[tokio::test]
    async fn t_complete_passthrough_stub() {
        let expected = vec![
            Ok(StreamEvent::ProviderReady {
                provider: "Mock".to_owned(),
                model: "m".to_owned(),
            }),
            Ok(StreamEvent::TextDelta {
                text: "hello".to_owned(),
            }),
            Ok(StreamEvent::Done {
                stop_reason: crate::StopReason::EndTurn,
                tool_calls: vec![],
            }),
        ];
        let wrapped = wrapped_with_mock(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: None,
            events: expected.clone(),
        });

        let mut stream = wrapped
            .complete(&[], &CompletionConfig::default())
            .await
            .expect("complete");
        let mut got = Vec::new();
        while let Some(item) = stream.next().await {
            got.push(item);
        }
        assert_eq!(got, expected);
    }

    #[test]
    fn t_collector_starts_empty() {
        let c = AttemptCollector::new(Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256)));
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn t_collector_records_one_attempt() {
        let c = AttemptCollector::new(Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256)));
        c.record_attempt(sample_attempt(1));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn t_collector_records_multiple_in_order() {
        let c = AttemptCollector::new(Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256)));
        c.record_attempt(sample_attempt(1));
        c.record_attempt(sample_attempt(2));
        c.record_attempt(sample_attempt(3));
        let drained = c.drain();
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0].attempt_number, 1);
        assert_eq!(drained[1].attempt_number, 2);
        assert_eq!(drained[2].attempt_number, 3);
    }

    #[test]
    fn t_collector_drain_resets() {
        let c = AttemptCollector::new(Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256)));
        c.record_attempt(sample_attempt(1));
        let _ = c.drain();
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn t_collector_concurrent_access_safe() {
        const THREADS: usize = 8;
        const WRITES_PER_THREAD: usize = 10;
        let c = Arc::new(AttemptCollector::new(Arc::new(MemoryObjectStore::new(
            HashAlgorithm::Sha256,
        ))));
        let mut joins = Vec::new();
        for thread_idx in 0..THREADS {
            let collector = Arc::clone(&c);
            joins.push(thread::spawn(move || {
                for i in 0..WRITES_PER_THREAD {
                    let n = (thread_idx * WRITES_PER_THREAD + i + 1) as u32;
                    collector.record_attempt(sample_attempt(n));
                }
            }));
        }
        for j in joins {
            j.join().expect("thread join");
        }
        assert_eq!(c.len(), THREADS * WRITES_PER_THREAD);
    }

    #[test]
    fn t_collector_via_trait_object() {
        let c = Arc::new(AttemptCollector::new(Arc::new(MemoryObjectStore::new(
            HashAlgorithm::Sha256,
        ))));
        let observer: Arc<dyn AttemptObserver> = c.clone();
        observer.record_attempt(sample_attempt(1));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn t_request_payload_canonical_deterministic_for_tool_parameter_key_order() {
        let messages = vec![Message {
            role: crate::MessageRole::User,
            content: "hi".to_owned(),
        }];
        let cfg_one = CompletionConfig {
            tools: vec![crate::ToolDefinition {
                name: "tool".to_owned(),
                description: "d".to_owned(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "a": {"type": "string"},
                        "b": {"type": "number"}
                    }
                }),
            }],
            ..CompletionConfig::default()
        };
        let cfg_two = CompletionConfig {
            tools: vec![crate::ToolDefinition {
                name: "tool".to_owned(),
                description: "d".to_owned(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "b": {"type": "number"},
                        "a": {"type": "string"}
                    }
                }),
            }],
            ..CompletionConfig::default()
        };
        let p1 = super::RequestPayload {
            provider_id: "mock-provider",
            messages: &messages,
            config: &cfg_one,
        };
        let p2 = super::RequestPayload {
            provider_id: "mock-provider",
            messages: &messages,
            config: &cfg_two,
        };
        let b1 = super::canonical_cbor_bytes(&p1).expect("cbor1");
        let b2 = super::canonical_cbor_bytes(&p2).expect("cbor2");
        assert_eq!(b1, b2);
        let h1 = digest_bytes(HashAlgorithm::Sha256, &b1);
        let h2 = digest_bytes(HashAlgorithm::Sha256, &b2);
        assert_eq!(h1, h2);
    }

    #[tokio::test]
    async fn t_complete_records_request_hash() {
        let (wrapped, store, graph) = wrapped_with_mock_handles(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: None,
            events: vec![
                Ok(StreamEvent::ProviderReady {
                    provider: "Mock".to_owned(),
                    model: "m".to_owned(),
                }),
                Ok(StreamEvent::Done {
                    stop_reason: crate::StopReason::EndTurn,
                    tool_calls: vec![],
                }),
            ],
        });
        let messages = vec![Message {
            role: crate::MessageRole::User,
            content: "hello".to_owned(),
        }];
        let config = CompletionConfig::default();
        let mut stream = wrapped
            .complete(&messages, &config)
            .await
            .expect("complete");
        while stream.next().await.is_some() {}
        let guard = graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let history = guard.history().expect("history");
        let (_, event) = history.last().expect("last");
        let request_hash = match &event.kind {
            EventKind::ProviderCall { attempts, .. } => attempts[0].request_hash.clone(),
            _ => panic!("expected ProviderCall"),
        };
        let got = store
            .get(&request_hash)
            .expect("store get")
            .expect("request bytes");
        let payload = super::RequestPayload {
            provider_id: "mock-provider",
            messages: &messages,
            config: &config,
        };
        let expected = super::canonical_cbor_bytes(&payload).expect("payload bytes");
        assert_eq!(got.as_ref(), expected.as_slice());
    }

    #[tokio::test]
    async fn t_complete_records_stream_chunks() {
        let events = vec![
            Ok(StreamEvent::ProviderReady {
                provider: "Mock".to_owned(),
                model: "m".to_owned(),
            }),
            Ok(StreamEvent::TextDelta {
                text: "a".to_owned(),
            }),
            Ok(StreamEvent::TextDelta {
                text: "b".to_owned(),
            }),
            Ok(StreamEvent::TextDelta {
                text: "c".to_owned(),
            }),
            Ok(StreamEvent::Done {
                stop_reason: crate::StopReason::EndTurn,
                tool_calls: vec![],
            }),
        ];
        let (wrapped, store, graph) = wrapped_with_mock_handles(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: None,
            events: events.clone(),
        });
        let mut stream = wrapped
            .complete(&[], &CompletionConfig::default())
            .await
            .expect("complete");
        while stream.next().await.is_some() {}
        let expected_chunk_hashes: Vec<Hash> = events
            .iter()
            .map(|evt| match evt {
                Ok(ev) => {
                    let bytes = super::canonical_cbor_bytes(&super::stream_event_chunk(ev))
                        .expect("chunk bytes");
                    digest_bytes(store.algorithm(), &bytes)
                }
                Err(_) => panic!("expected ok event"),
            })
            .collect();
        for h in &expected_chunk_hashes {
            assert!(store.contains(h).expect("contains"));
        }
        let guard = graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let history = guard.history().expect("history");
        let (_, event) = history.last().expect("last");
        let stream_hash = match &event.kind {
            EventKind::ProviderCall { stream_hash, .. } => {
                stream_hash.clone().expect("stream hash")
            }
            _ => panic!("expected ProviderCall"),
        };
        let stored_vec_bytes = store
            .get(&stream_hash)
            .expect("store get")
            .expect("stream hash bytes");
        let expected_vec_bytes =
            super::canonical_cbor_bytes(&expected_chunk_hashes).expect("chunk vec");
        assert_eq!(stored_vec_bytes.as_ref(), expected_vec_bytes.as_slice());
    }

    #[tokio::test]
    async fn t_complete_emits_provider_call_event() {
        let (wrapped, _store, graph) = wrapped_with_mock_handles(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: None,
            events: vec![
                Ok(StreamEvent::ProviderReady {
                    provider: "Mock".to_owned(),
                    model: "m".to_owned(),
                }),
                Ok(StreamEvent::Done {
                    stop_reason: crate::StopReason::EndTurn,
                    tool_calls: vec![],
                }),
            ],
        });
        let mut stream = wrapped
            .complete(&[], &CompletionConfig::default())
            .await
            .expect("complete");
        while stream.next().await.is_some() {}
        let guard = graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let history = guard.history().expect("history");
        let provider_calls: Vec<&EventKind> = history
            .iter()
            .map(|(_, e)| &e.kind)
            .filter(|k| matches!(k, EventKind::ProviderCall { .. }))
            .collect();
        assert_eq!(provider_calls.len(), 1);
        match provider_calls[0] {
            EventKind::ProviderCall {
                provider_id,
                attempts,
                stream_hash,
            } => {
                assert_eq!(provider_id, "mock-provider");
                assert_eq!(attempts.len(), 1);
                assert!(stream_hash.is_some());
            }
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn t_complete_synthesized_attempt_marks_success_on_clean_stream() {
        let (wrapped, _store, graph) = wrapped_with_mock_handles(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: None,
            events: vec![Ok(StreamEvent::Done {
                stop_reason: crate::StopReason::EndTurn,
                tool_calls: vec![],
            })],
        });
        let mut stream = wrapped
            .complete(&[], &CompletionConfig::default())
            .await
            .expect("complete");
        while stream.next().await.is_some() {}
        let guard = graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let history = guard.history().expect("history");
        let (_, event) = history.last().expect("last");
        let attempt = match &event.kind {
            EventKind::ProviderCall { attempts, .. } => attempts[0].clone(),
            _ => panic!("expected ProviderCall"),
        };
        assert_eq!(attempt.status, akmon_journal::AttemptStatus::Success);
        assert!(attempt.error_message.is_none());
    }

    #[tokio::test]
    async fn t_complete_synthesized_attempt_marks_other_on_error_stream() {
        let err = ModelError::ModelNotFound {
            model: "m".to_owned(),
            hint: "missing".to_owned(),
        };
        let (wrapped, _store, graph) = wrapped_with_mock_handles(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: None,
            events: vec![Err(err.clone())],
        });
        let mut stream = wrapped
            .complete(&[], &CompletionConfig::default())
            .await
            .expect("complete");
        while stream.next().await.is_some() {}
        let guard = graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let history = guard.history().expect("history");
        let (_, event) = history.last().expect("last");
        let attempt = match &event.kind {
            EventKind::ProviderCall { attempts, .. } => attempts[0].clone(),
            _ => panic!("expected ProviderCall"),
        };
        assert_eq!(
            attempt.status,
            super::map_model_error_to_attempt_status(&err)
        );
        assert!(attempt.error_message.is_some());
    }

    #[tokio::test]
    async fn t_complete_request_hash_deterministic() {
        let (wrapped, _store, graph) = wrapped_with_mock_handles(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: None,
            events: vec![Ok(StreamEvent::Done {
                stop_reason: crate::StopReason::EndTurn,
                tool_calls: vec![],
            })],
        });
        let messages = vec![Message {
            role: crate::MessageRole::User,
            content: "same".to_owned(),
        }];
        let config = CompletionConfig::default();
        for _ in 0..2 {
            let mut s = wrapped
                .complete(&messages, &config)
                .await
                .expect("complete");
            while s.next().await.is_some() {}
        }
        let guard = graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let history = guard.history().expect("history");
        let hashes: Vec<Hash> = history
            .iter()
            .filter_map(|(_, e)| match &e.kind {
                EventKind::ProviderCall { attempts, .. } => Some(attempts[0].request_hash.clone()),
                _ => None,
            })
            .collect();
        assert!(hashes.len() >= 2);
        assert_eq!(hashes[hashes.len() - 1], hashes[hashes.len() - 2]);
    }

    #[tokio::test]
    async fn t_complete_yields_unchanged_to_caller() {
        let events = vec![
            Ok(StreamEvent::ProviderReady {
                provider: "Mock".to_owned(),
                model: "m".to_owned(),
            }),
            Ok(StreamEvent::TextDelta {
                text: "hello".to_owned(),
            }),
            Ok(StreamEvent::Done {
                stop_reason: crate::StopReason::EndTurn,
                tool_calls: vec![],
            }),
        ];
        let (wrapped, _, _) = wrapped_with_mock_handles(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: None,
            events: events.clone(),
        });
        let mut stream = wrapped
            .complete(&[], &CompletionConfig::default())
            .await
            .expect("complete");
        let mut got = Vec::new();
        while let Some(item) = stream.next().await {
            got.push(item);
        }
        assert_eq!(got, events);
    }

    #[tokio::test]
    async fn t_provider_call_visible_immediately_after_stream_drains() {
        let (wrapped, _store, graph) = wrapped_with_mock_handles(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: None,
            events: vec![
                Ok(StreamEvent::ProviderReady {
                    provider: "Mock".to_owned(),
                    model: "m".to_owned(),
                }),
                Ok(StreamEvent::Done {
                    stop_reason: crate::StopReason::EndTurn,
                    tool_calls: vec![],
                }),
            ],
        });
        let mut stream = wrapped
            .complete(&[], &CompletionConfig::default())
            .await
            .expect("complete");
        while stream.next().await.is_some() {}
        let guard = graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let history = guard.history().expect("history");
        assert!(
            history
                .iter()
                .any(|(_, e)| matches!(e.kind, EventKind::ProviderCall { .. }))
        );
    }

    #[tokio::test]
    async fn t_set_attempt_observer_default_is_noop() {
        let (wrapped, _store, graph) = wrapped_with_mock_handles(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: None,
            events: vec![Ok(StreamEvent::Done {
                stop_reason: crate::StopReason::EndTurn,
                tool_calls: vec![],
            })],
        });
        let mut stream = wrapped
            .complete(&[], &CompletionConfig::default())
            .await
            .expect("complete");
        while stream.next().await.is_some() {}
        let guard = graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let history = guard.history().expect("history");
        let (_, event) = history.last().expect("last");
        match &event.kind {
            EventKind::ProviderCall { attempts, .. } => assert_eq!(attempts.len(), 1),
            _ => panic!("expected ProviderCall"),
        }
    }

    #[tokio::test]
    async fn t_set_attempt_observer_with_instrumented_backend() {
        let provider = InstrumentedMockProvider {
            observer: RwLock::new(None),
            events: vec![Ok(StreamEvent::Done {
                stop_reason: crate::StopReason::EndTurn,
                tool_calls: vec![],
            })],
        };
        let (wrapped, _store, graph) = wrapped_with_instrumented_handles(provider);
        let mut stream = wrapped
            .complete(&[], &CompletionConfig::default())
            .await
            .expect("complete");
        while stream.next().await.is_some() {}
        let guard = graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let history = guard.history().expect("history");
        let (_, event) = history.last().expect("last");
        match &event.kind {
            EventKind::ProviderCall { attempts, .. } => assert_eq!(attempts.len(), 2),
            _ => panic!("expected ProviderCall"),
        }
    }

    #[test]
    fn t_trait_object_compatibility() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            provider_name: "mock",
            context_window: 1,
            model_id: "m",
            estimate: None,
            events: vec![],
        });
        let observer: Arc<dyn AttemptObserver> = Arc::new(AttemptCollector::new(Arc::new(
            MemoryObjectStore::new(HashAlgorithm::Sha256),
        )));
        provider.set_attempt_observer(observer);
    }
}
