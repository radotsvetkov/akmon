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

use std::sync::{Arc, Mutex};

use akmon_journal::{AttemptRecord, ObjectStore, SessionGraph};
use async_trait::async_trait;

use crate::{CompletionConfig, CompletionStream, LlmProvider, Message, ModelError};

/// Receives one [`AttemptRecord`] per HTTP attempt within a logical
/// [`crate::LlmProvider::complete`] call.
///
/// Used by [`JournalingProvider`] to capture retry history that would otherwise be
/// invisible to the wrapper.
pub trait AttemptObserver: Send + Sync {
    /// Records one provider HTTP/model attempt.
    fn record_attempt(&self, attempt: AttemptRecord);
}

/// Journaling wrapper around an [`LlmProvider`] backend.
///
/// The wrapper stores evidence objects and appends provider-call events to a session graph.
/// A mutex is required around the graph because [`LlmProvider::complete`] takes `&self` while
/// [`SessionGraph::append`] requires `&mut self`.
///
/// The graph lock is intended to be held briefly per append operation and never across
/// awaits while consuming the inner provider stream.
#[allow(dead_code)]
pub struct JournalingProvider<P, S, G>
where
    P: LlmProvider,
    S: ObjectStore,
    G: SessionGraph,
{
    inner: P,
    store: Arc<S>,
    graph: Arc<tokio::sync::Mutex<G>>,
    provider_id: String,
}

impl<P, S, G> JournalingProvider<P, S, G>
where
    P: LlmProvider,
    S: ObjectStore,
    G: SessionGraph,
{
    /// Creates a new journaling wrapper.
    pub fn new(
        inner: P,
        provider_id: String,
        store: Arc<S>,
        graph: Arc<tokio::sync::Mutex<G>>,
    ) -> Self {
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
    S: ObjectStore,
    G: SessionGraph,
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
        self.inner.complete(messages, config).await
    }
}

/// Collects [`AttemptRecord`] values during one `complete()` call.
///
/// Implements [`AttemptObserver`], buffering attempts in an internal mutex-protected
/// vector. The wrapper drains this buffer after the inner stream terminates.
#[allow(dead_code)]
struct AttemptCollector {
    attempts: Mutex<Vec<AttemptRecord>>,
}

#[allow(dead_code)]
impl AttemptCollector {
    fn new() -> Self {
        Self {
            attempts: Mutex::new(Vec::new()),
        }
    }
}

impl AttemptObserver for AttemptCollector {
    fn record_attempt(&self, attempt: AttemptRecord) {
        let _ = attempt;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use akmon_journal::{HashAlgorithm, MemoryObjectStore, MemorySessionGraph};
    use async_trait::async_trait;
    use futures::StreamExt;

    use super::JournalingProvider;
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

    fn wrapped_with_mock(
        mock: MockProvider,
    ) -> JournalingProvider<MockProvider, MemoryObjectStore, MemorySessionGraph> {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let graph = MemorySessionGraph::open_new(Arc::clone(&store), uuid::Uuid::new_v4());
        JournalingProvider::new(
            mock,
            "mock-provider".to_owned(),
            store,
            Arc::new(tokio::sync::Mutex::new(graph)),
        )
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
}
