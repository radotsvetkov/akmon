//! Journaling wrapper for [`crate::Tool`] implementations.
//!
//! `JournalingTool` captures tool input, output, and optional
//! side-effects manifest into the AGEF journal substrate. Each
//! `execute()` call produces exactly one
//! [`akmon_journal::EventKind::ToolCall`] event in the session
//! graph.
//!
//! Unlike `JournalingProvider`, this wrapper has no retry
//! semantics, no streaming, and no per-attempt complexity.
//! Permission decisions are NOT captured by this wrapper —
//! they are session-level concerns emitted via PermissionGate
//! events at the agent loop level (Item 3.1).

use std::sync::{Arc, Mutex};

use akmon_journal::{EventKind, Hash, JournalError, ObjectStore, SessionGraph};
use async_trait::async_trait;
use serde::Serialize;

use crate::{McpPolicyContext, Tool, ToolContext, ToolOutput};

/// Wraps a [`Tool`] and records one journaled tool-call event per execution.
pub struct JournalingTool<T, S, G>
where
    T: Tool,
    S: ObjectStore,
    G: SessionGraph,
{
    inner: T,
    store: Arc<S>,
    graph: Arc<Mutex<G>>,
    tool_id: String,
}

impl<T, S, G> JournalingTool<T, S, G>
where
    T: Tool,
    S: ObjectStore,
    G: SessionGraph,
{
    /// Creates a new journaling wrapper over an inner tool.
    ///
    /// `tool_id` is explicit so callers may choose a logical identifier that differs
    /// from `inner.name()` (for example, namespaced MCP tool ids).
    pub fn new(inner: T, tool_id: String, store: Arc<S>, graph: Arc<Mutex<G>>) -> Self {
        Self {
            inner,
            store,
            graph,
            tool_id,
        }
    }
}

fn canonical_cbor_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, JournalError> {
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(value, &mut bytes)
        .map_err(|err| JournalError::Cbor(err.to_string()))?;
    Ok(bytes)
}

fn zero_hash<S: ObjectStore>(store: &S) -> Hash {
    Hash::from_bytes(store.algorithm(), [0_u8; 32])
}

fn store_canonical_or_zero<S: ObjectStore, T: Serialize + ?Sized>(
    store: &S,
    value: &T,
    tool_id: &str,
    field_name: &str,
) -> Hash {
    match canonical_cbor_bytes(value).and_then(|bytes| store.put(&bytes)) {
        Ok(hash) => hash,
        Err(err) => {
            tracing::warn!(
                tool_id,
                field = field_name,
                error = %err,
                "journaling degraded: failed to persist canonical object"
            );
            zero_hash(store)
        }
    }
}

#[async_trait]
impl<T, S, G> Tool for JournalingTool<T, S, G>
where
    T: Tool,
    S: ObjectStore,
    G: SessionGraph,
{
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn required_permissions(&self) -> &[akmon_core::Permission] {
        self.inner.required_permissions()
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.inner.parameters_schema()
    }

    fn mcp_policy_context(&self) -> Option<McpPolicyContext> {
        self.inner.mcp_policy_context()
    }

    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        // Clone once so we can both journal deterministic input bytes and pass owned args
        // to the inner tool execute call.
        let args_for_journal = args.clone();
        let input_hash = store_canonical_or_zero(
            self.store.as_ref(),
            &args_for_journal,
            &self.tool_id,
            "input",
        );
        let output = self.inner.execute(args, ctx).await;
        let output_hash =
            store_canonical_or_zero(self.store.as_ref(), &output, &self.tool_id, "output");
        let side_effects_hash = match self.inner.side_effects_manifest(&args_for_journal, &output) {
            Some(bytes) => match self.store.put(&bytes) {
                Ok(hash) => Some(hash),
                Err(err) => {
                    tracing::warn!(
                        tool_id = %self.tool_id,
                        field = "side_effects",
                        error = %err,
                        "journaling degraded: failed to persist side effects object"
                    );
                    Some(zero_hash(self.store.as_ref()))
                }
            },
            None => None,
        };
        let mut guard = self
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Err(err) = guard.append(EventKind::ToolCall {
            tool_id: self.tool_id.clone(),
            input_hash,
            output_hash,
            side_effects_hash,
        }) {
            tracing::warn!(
                tool_id = %self.tool_id,
                error = %err,
                "journaling degraded: failed to append ToolCall event"
            );
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use akmon_core::{Permission, PolicyEngine, PolicyEngineMode, Sandbox};
    use akmon_journal::{
        EventKind, Hash, HashAlgorithm, JournalError, MemoryObjectStore, MemorySessionGraph,
        ObjectStore, SessionGraph,
    };
    use async_trait::async_trait;

    use super::JournalingTool;
    use crate::{McpPolicyContext, Tool, ToolContext, ToolErrorCode, ToolOutput};

    #[derive(Clone)]
    struct MockTool {
        name: &'static str,
        description: &'static str,
        parameters_schema: serde_json::Value,
        mcp_policy_context: Option<McpPolicyContext>,
        output: ToolOutput,
    }

    #[derive(Clone)]
    struct SideEffectsMockTool {
        inner: MockTool,
        manifest: Vec<u8>,
    }

    #[async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            self.description
        }

        fn required_permissions(&self) -> &[Permission] {
            &[]
        }

        fn parameters_schema(&self) -> serde_json::Value {
            self.parameters_schema.clone()
        }

        fn mcp_policy_context(&self) -> Option<McpPolicyContext> {
            self.mcp_policy_context.clone()
        }

        async fn execute(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
            self.output.clone()
        }
    }

    #[async_trait]
    impl Tool for SideEffectsMockTool {
        fn name(&self) -> &str {
            self.inner.name()
        }

        fn description(&self) -> &str {
            self.inner.description()
        }

        fn required_permissions(&self) -> &[Permission] {
            self.inner.required_permissions()
        }

        fn parameters_schema(&self) -> serde_json::Value {
            self.inner.parameters_schema()
        }

        fn mcp_policy_context(&self) -> Option<McpPolicyContext> {
            self.inner.mcp_policy_context()
        }

        async fn execute(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
            self.inner.output.clone()
        }

        fn side_effects_manifest(
            &self,
            _input: &serde_json::Value,
            _output: &ToolOutput,
        ) -> Option<Vec<u8>> {
            Some(self.manifest.clone())
        }
    }

    struct AlwaysFailPutStore {
        algorithm: HashAlgorithm,
    }

    type MockWrappedHandles = (
        JournalingTool<MockTool, MemoryObjectStore, MemorySessionGraph>,
        Arc<MemoryObjectStore>,
        Arc<Mutex<MemorySessionGraph>>,
    );
    type SideEffectsWrappedHandles = (
        JournalingTool<SideEffectsMockTool, MemoryObjectStore, MemorySessionGraph>,
        Arc<MemoryObjectStore>,
        Arc<Mutex<MemorySessionGraph>>,
    );

    impl ObjectStore for AlwaysFailPutStore {
        fn algorithm(&self) -> HashAlgorithm {
            self.algorithm
        }

        fn put(&self, _bytes: &[u8]) -> Result<Hash, JournalError> {
            Err(JournalError::Verification(
                "simulated object-store put failure".to_owned(),
            ))
        }

        fn get(&self, _hash: &Hash) -> Result<Option<bytes::Bytes>, JournalError> {
            Ok(None)
        }

        fn contains(&self, _hash: &Hash) -> Result<bool, JournalError> {
            Ok(false)
        }

        fn iter_hashes(&self) -> Result<Box<dyn Iterator<Item = Hash> + '_>, JournalError> {
            Ok(Box::new(std::iter::empty()))
        }
    }

    fn test_context() -> ToolContext {
        ToolContext::new(
            Sandbox::new(std::env::temp_dir()),
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
        )
    }

    fn seeded_graph(store: &Arc<MemoryObjectStore>) -> Arc<Mutex<MemorySessionGraph>> {
        let mut graph = MemorySessionGraph::open_new(Arc::clone(store), uuid::Uuid::new_v4());
        let cwd_hash = store.put(b"cwd").expect("cwd hash");
        let config_hash = store.put(b"config").expect("config hash");
        graph
            .append(EventKind::SessionStart {
                cwd_hash,
                config_hash,
            })
            .expect("session start");
        Arc::new(Mutex::new(graph))
    }

    fn wrap_with_handles(mock: MockTool) -> MockWrappedHandles {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let graph = seeded_graph(&store);
        let wrapped = JournalingTool::new(
            mock,
            "tool-id".to_owned(),
            Arc::clone(&store),
            Arc::clone(&graph),
        );
        (wrapped, store, graph)
    }

    fn wrap(mock: MockTool) -> JournalingTool<MockTool, MemoryObjectStore, MemorySessionGraph> {
        let (wrapped, _, _) = wrap_with_handles(mock);
        wrapped
    }

    fn wrap_side_effects_with_handles(mock: SideEffectsMockTool) -> SideEffectsWrappedHandles {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let graph = seeded_graph(&store);
        let wrapped = JournalingTool::new(
            mock,
            "tool-id".to_owned(),
            Arc::clone(&store),
            Arc::clone(&graph),
        );
        (wrapped, store, graph)
    }

    fn latest_tool_call(
        graph: &Arc<Mutex<MemorySessionGraph>>,
    ) -> (String, Hash, Hash, Option<Hash>) {
        let guard = graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let history = guard.history().expect("history");
        let (_, event) = history.last().expect("latest event");
        match &event.kind {
            EventKind::ToolCall {
                tool_id,
                input_hash,
                output_hash,
                side_effects_hash,
            } => (
                tool_id.clone(),
                input_hash.clone(),
                output_hash.clone(),
                side_effects_hash.clone(),
            ),
            _ => panic!("expected ToolCall event"),
        }
    }

    fn all_tool_calls(
        graph: &Arc<Mutex<MemorySessionGraph>>,
    ) -> Vec<(String, Hash, Hash, Option<Hash>)> {
        let guard = graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let history = guard.history().expect("history");
        history
            .iter()
            .filter_map(|(_, event)| match &event.kind {
                EventKind::ToolCall {
                    tool_id,
                    input_hash,
                    output_hash,
                    side_effects_hash,
                } => Some((
                    tool_id.clone(),
                    input_hash.clone(),
                    output_hash.clone(),
                    side_effects_hash.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    fn base_mock() -> MockTool {
        MockTool {
            name: "mock-tool",
            description: "mock description",
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
            mcp_policy_context: None,
            output: ToolOutput::Success {
                content: "ok".to_owned(),
            },
        }
    }

    #[test]
    fn t_passthrough_name() {
        let wrapped = wrap(base_mock());
        assert_eq!(wrapped.name(), "mock-tool");
    }

    #[test]
    fn t_passthrough_description() {
        let wrapped = wrap(base_mock());
        assert_eq!(wrapped.description(), "mock description");
    }

    #[test]
    fn t_passthrough_parameters_schema() {
        let wrapped = wrap(base_mock());
        assert_eq!(
            wrapped.parameters_schema(),
            serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            })
        );
    }

    #[test]
    fn t_passthrough_mcp_policy_context() {
        let wrapped_none = wrap(base_mock());
        assert_eq!(wrapped_none.mcp_policy_context(), None);

        let mut with_context = base_mock();
        with_context.mcp_policy_context = Some(McpPolicyContext {
            server: "server-a".to_owned(),
            tool: "tool-a".to_owned(),
        });
        let wrapped_some = wrap(with_context);
        assert_eq!(
            wrapped_some.mcp_policy_context(),
            Some(McpPolicyContext {
                server: "server-a".to_owned(),
                tool: "tool-a".to_owned(),
            })
        );
    }

    #[tokio::test]
    async fn t_passthrough_execute_returns_inner_output() {
        let mut mock = base_mock();
        mock.output = ToolOutput::Error {
            code: ToolErrorCode::InvalidArgs,
            message: "bad args".to_owned(),
        };
        let wrapped = wrap(mock);
        let out = wrapped
            .execute(serde_json::json!({"path": "x"}), &test_context())
            .await;
        assert_eq!(
            out,
            ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: "bad args".to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn t_default_side_effects_manifest_returns_none() {
        let mock = base_mock();
        let result = mock.side_effects_manifest(
            &serde_json::json!({"path": "x"}),
            &ToolOutput::Success {
                content: "ok".to_owned(),
            },
        );
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn t_execute_emits_tool_call_event() {
        let (wrapped, store, graph) = wrap_with_handles(base_mock());
        let out = wrapped
            .execute(serde_json::json!({"path":"a.txt"}), &test_context())
            .await;
        assert_eq!(
            out,
            ToolOutput::Success {
                content: "ok".to_owned()
            }
        );
        let calls = all_tool_calls(&graph);
        assert_eq!(calls.len(), 1);
        let (tool_id, input_hash, output_hash, side_effects_hash) = calls[0].clone();
        assert_eq!(tool_id, "tool-id");
        assert!(store.contains(&input_hash).expect("input exists"));
        assert!(store.contains(&output_hash).expect("output exists"));
        assert_eq!(side_effects_hash, None);
    }

    #[tokio::test]
    async fn t_execute_with_side_effects_manifest() {
        let mock = SideEffectsMockTool {
            inner: base_mock(),
            manifest: vec![1, 2, 3, 4],
        };
        let (wrapped, store, graph) = wrap_side_effects_with_handles(mock);
        let _ = wrapped
            .execute(serde_json::json!({"path":"b.txt"}), &test_context())
            .await;
        let (_, _, _, side_effects_hash) = latest_tool_call(&graph);
        let side_effects_hash = side_effects_hash.expect("side effects hash");
        assert!(
            store
                .contains(&side_effects_hash)
                .expect("side effects object exists")
        );
    }

    #[tokio::test]
    async fn t_execute_without_side_effects_manifest() {
        let (wrapped, _, graph) = wrap_with_handles(base_mock());
        let _ = wrapped
            .execute(serde_json::json!({"path":"c.txt"}), &test_context())
            .await;
        let (_, _, _, side_effects_hash) = latest_tool_call(&graph);
        assert_eq!(side_effects_hash, None);
    }

    #[tokio::test]
    async fn t_execute_records_event_for_tool_error() {
        let mut mock = base_mock();
        mock.output = ToolOutput::Error {
            code: ToolErrorCode::InvalidArgs,
            message: "bad args".to_owned(),
        };
        let (wrapped, store, graph) = wrap_with_handles(mock);
        let out = wrapped
            .execute(serde_json::json!({"path":"bad"}), &test_context())
            .await;
        assert_eq!(
            out,
            ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: "bad args".to_owned(),
            }
        );
        let (_, _, output_hash, _) = latest_tool_call(&graph);
        let stored = store
            .get(&output_hash)
            .expect("store get")
            .expect("output object");
        let expected = super::canonical_cbor_bytes(&ToolOutput::Error {
            code: ToolErrorCode::InvalidArgs,
            message: "bad args".to_owned(),
        })
        .expect("expected bytes");
        assert_eq!(stored.as_ref(), expected.as_slice());
    }

    #[tokio::test]
    async fn t_execute_input_hash_deterministic() {
        let (wrapped, _, graph) = wrap_with_handles(base_mock());
        for _ in 0..2 {
            let _ = wrapped
                .execute(serde_json::json!({"path":"same"}), &test_context())
                .await;
        }
        let calls = all_tool_calls(&graph);
        assert!(calls.len() >= 2);
        let (_, input_hash_a, _, _) = &calls[calls.len() - 1];
        let (_, input_hash_b, _, _) = &calls[calls.len() - 2];
        assert_eq!(input_hash_a, input_hash_b);
    }

    #[tokio::test]
    async fn t_execute_output_hash_deterministic() {
        let (wrapped, _, graph) = wrap_with_handles(base_mock());
        for _ in 0..2 {
            let _ = wrapped
                .execute(serde_json::json!({"path":"same"}), &test_context())
                .await;
        }
        let calls = all_tool_calls(&graph);
        assert!(calls.len() >= 2);
        let (_, _, output_hash_a, _) = &calls[calls.len() - 1];
        let (_, _, output_hash_b, _) = &calls[calls.len() - 2];
        assert_eq!(output_hash_a, output_hash_b);
    }

    #[tokio::test]
    async fn t_execute_journaling_failure_emits_degraded_event() {
        let failing_store = Arc::new(AlwaysFailPutStore {
            algorithm: HashAlgorithm::Sha256,
        });
        let graph_store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let graph = seeded_graph(&graph_store);
        let wrapped = JournalingTool::new(
            base_mock(),
            "tool-id".to_owned(),
            failing_store.clone(),
            Arc::clone(&graph),
        );

        let out = wrapped
            .execute(serde_json::json!({"path":"x"}), &test_context())
            .await;
        assert_eq!(
            out,
            ToolOutput::Success {
                content: "ok".to_owned(),
            }
        );
        let (_, input_hash, output_hash, side_effects_hash) = latest_tool_call(&graph);
        let zero = Hash::from_bytes(HashAlgorithm::Sha256, [0_u8; 32]);
        assert_eq!(input_hash, zero);
        assert_eq!(output_hash, zero);
        assert_eq!(side_effects_hash, None);
    }
}
