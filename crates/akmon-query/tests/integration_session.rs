//! End-to-end session integration: Redb-backed journal, full agent loop, reopen + verify.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use akmon_core::{AgentConfig, Permission, PolicyEngine, PolicyEngineMode, Sandbox};
use akmon_journal::{
    EventKind, HashAlgorithm, JournalError, ObjectStore, RedbObjectStore, RedbSessionGraph,
    SessionGraph,
};
use akmon_models::{
    CompletionStream, LlmProvider, ModelError, ModelToolCall, StopReason, StreamEvent,
};
use akmon_query::{AgentSession, JournalHandle};
use akmon_tools::{Tool, ToolContext, ToolOutput};
use async_trait::async_trait;
use futures::stream;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

const TURN1_PROMPT: &str = "User asks for help with a task";
const TURN2_PROMPT: &str = "What about the next step?";
const TOOL_NAME: &str = "search_like";
const TEXT_BEFORE_TOOL: &str = "I'll run a search.";
const TEXT_AFTER_TOOL: &str = "Here are the results.";
const TEXT_TURN2: &str = "Follow-up reply text.";
const TOOL_OUTPUT: &str = "TOOL_OUTPUT_PRED";

fn journal_db_path(dir: &Path) -> PathBuf {
    dir.join("journal.redb")
}

fn open_journal_handle(
    dir: &Path,
    session_id: Uuid,
) -> Result<JournalHandle<RedbObjectStore, RedbSessionGraph>, JournalError> {
    std::fs::create_dir_all(dir).map_err(|e| JournalError::Verification(e.to_string()))?;
    let path = journal_db_path(dir);
    let store = Arc::new(RedbObjectStore::create(
        path.as_path(),
        HashAlgorithm::Sha256,
    )?);
    let graph = RedbSessionGraph::open_new(Arc::clone(&store), session_id)?;
    Ok(JournalHandle::new(store, Arc::new(Mutex::new(graph))))
}

fn reopen_journal_handle(
    dir: &Path,
    session_id: Uuid,
) -> Result<JournalHandle<RedbObjectStore, RedbSessionGraph>, JournalError> {
    let path = journal_db_path(dir);
    let store = Arc::new(RedbObjectStore::open(path.as_path())?);
    let graph = RedbSessionGraph::reopen(Arc::clone(&store), session_id)?;
    Ok(JournalHandle::new(store, Arc::new(Mutex::new(graph))))
}

fn assert_event_kinds(history: &[(akmon_journal::Hash, akmon_journal::Event)], expected: &[&str]) {
    let got: Vec<&str> = history
        .iter()
        .map(|(_, e)| match &e.kind {
            EventKind::SessionStart { .. } => "SessionStart",
            EventKind::UserTurn { .. } => "UserTurn",
            EventKind::ProviderCall { .. } => "ProviderCall",
            EventKind::ToolCall { .. } => "ToolCall",
            EventKind::PermissionGate { .. } => "PermissionGate",
            EventKind::AssistantTurn { .. } => "AssistantTurn",
            EventKind::RetrievalCall { .. } => "RetrievalCall",
            EventKind::SessionEnd { .. } => "SessionEnd",
        })
        .collect();
    assert_eq!(
        got, expected,
        "journal kind sequence mismatch:\n  expected: {expected:?}\n  got:      {got:?}"
    );
}

fn canonical_cbor_bytes<T: serde::Serialize + ?Sized>(v: &T) -> Vec<u8> {
    let mut b = Vec::new();
    ciborium::ser::into_writer(v, &mut b).expect("cbor");
    b
}

/// Returns one [`Permission::ReadFile`] on `"."` (same shape as session unit tests).
fn search_tool_perms() -> &'static [Permission] {
    use std::sync::OnceLock;
    static CELL: OnceLock<Vec<Permission>> = OnceLock::new();
    CELL.get_or_init(|| {
        vec![Permission::ReadFile {
            path: PathBuf::from("."),
        }]
    })
    .as_slice()
}

struct IntegrationSearchTool;

#[async_trait]
impl Tool for IntegrationSearchTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        "integration search mock"
    }

    fn required_permissions(&self) -> &[Permission] {
        search_tool_perms()
    }

    async fn execute(&self, _args: Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::Success {
            content: TOOL_OUTPUT.into(),
        }
    }
}

/// One [`LlmProvider::complete`] stream per construction sequence (multi-model-turn [`run`]).
struct MultiTurnMockProvider {
    sequences: Vec<Vec<Result<StreamEvent, ModelError>>>,
    call: AtomicUsize,
}

impl MultiTurnMockProvider {
    fn new(sequences: Vec<Vec<Result<StreamEvent, ModelError>>>) -> Self {
        Self {
            sequences,
            call: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LlmProvider for MultiTurnMockProvider {
    fn name(&self) -> &str {
        "integration-mock"
    }

    fn context_window_tokens(&self) -> usize {
        200_000
    }

    fn completion_model_id(&self) -> &str {
        "integration-mock-model"
    }

    async fn complete(
        &self,
        _messages: &[akmon_models::Message],
        _config: &akmon_models::CompletionConfig,
    ) -> Result<CompletionStream, ModelError> {
        let i = self.call.fetch_add(1, Ordering::SeqCst);
        let events = self.sequences.get(i).cloned().unwrap_or_default();
        Ok(Box::pin(stream::iter(events)))
    }
}

fn integration_sequences() -> Vec<Vec<Result<StreamEvent, ModelError>>> {
    vec![
        vec![
            Ok(StreamEvent::TextDelta {
                text: TEXT_BEFORE_TOOL.into(),
            }),
            Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "call-1".into(),
                    name: TOOL_NAME.into(),
                    arguments: json!({"query": "widgets"}),
                }],
            }),
        ],
        vec![
            Ok(StreamEvent::TextDelta {
                text: TEXT_AFTER_TOOL.into(),
            }),
            Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            }),
        ],
        vec![
            Ok(StreamEvent::TextDelta {
                text: TEXT_TURN2.into(),
            }),
            Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            }),
        ],
    ]
}

/// Full pipeline: Redb journal, two [`AgentSession::run`] turns, explicit [`AgentSession::end`], reopen, verify.
#[tokio::test]
async fn t_full_session_produces_complete_event_sequence_via_end() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal create");

    let cfg = AgentConfig {
        max_iterations: 8,
        confirmation_timeout_secs: 30,
        session_id: sid,
        auto_commit: false,
        max_completion_tokens: None,
        subagent_style: false,
        max_budget_usd: None,
        fallback_model: None,
        model_estimates: Vec::new(),
    };

    let mut session = AgentSession::new(
        cfg,
        Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
            confirm_writes: true,
        })),
        Arc::new(MultiTurnMockProvider::new(integration_sequences())),
        vec![Box::new(IntegrationSearchTool)],
        Arc::new(Sandbox::new(tmp.path())),
        None,
        false,
        journal,
    )
    .expect("session new");

    let (tx, _rx) = mpsc::channel(64);
    let mut no_policy = None;
    session
        .run(
            TURN1_PROMPT.into(),
            tx.clone(),
            &mut no_policy,
            &mut None,
            None,
        )
        .await
        .expect("run 1");
    session
        .run(TURN2_PROMPT.into(), tx, &mut no_policy, &mut None, None)
        .await
        .expect("run 2");
    session.end(None).expect("session end");
    // Redb keeps the DB file locked until the store is dropped; reopen requires a separate process-equivalent (fresh handle).
    drop(session);

    let verify = reopen_journal_handle(tmp.path(), sid).expect("reopen journal");
    let history = {
        let g = verify.graph.lock().expect("graph lock");
        g.history().expect("history")
    };

    let expected = [
        "SessionStart",
        "UserTurn",
        "ProviderCall",
        "PermissionGate",
        "ToolCall",
        "AssistantTurn",
        "ProviderCall",
        "AssistantTurn",
        "UserTurn",
        "ProviderCall",
        "AssistantTurn",
        "SessionEnd",
    ];
    assert_event_kinds(&history, &expected);

    let report = {
        let g = verify.graph.lock().expect("graph lock");
        g.verify().expect("verify")
    };
    assert!(
        report.missing_objects.is_empty(),
        "missing_objects: {:?}",
        report.missing_objects
    );
    assert!(
        report.hash_mismatches.is_empty(),
        "hash_mismatches: {:?}",
        report.hash_mismatches
    );
    assert!(
        report.broken_parent_links.is_empty(),
        "broken_parent_links: {:?}",
        report.broken_parent_links
    );
    assert!(
        report.sequence_violations.is_empty(),
        "sequence_violations: {:?}",
        report.sequence_violations
    );

    let ut1 = history
        .iter()
        .find_map(|(_, e)| match &e.kind {
            EventKind::UserTurn { prompt_hash } => Some(prompt_hash.clone()),
            _ => None,
        })
        .expect("first UserTurn");
    let ut1_bytes = verify.store.get(&ut1).expect("get ut1").expect("blob ut1");
    assert_eq!(ut1_bytes.as_ref(), TURN1_PROMPT.as_bytes());

    let ut2 = history
        .iter()
        .filter_map(|(_, e)| match &e.kind {
            EventKind::UserTurn { prompt_hash } => Some(prompt_hash.clone()),
            _ => None,
        })
        .nth(1)
        .expect("second UserTurn");
    let ut2_bytes = verify.store.get(&ut2).expect("get ut2").expect("blob ut2");
    assert_eq!(ut2_bytes.as_ref(), TURN2_PROMPT.as_bytes());

    let assistant_with_tools = history.iter().find_map(|(_, e)| match &e.kind {
        EventKind::AssistantTurn {
            message_hash,
            tool_calls_hash,
        } => tool_calls_hash
            .as_ref()
            .map(|tch| (message_hash.clone(), tch.clone())),
        _ => None,
    });
    let (msg_h, tc_h) = assistant_with_tools.expect("AssistantTurn with tool_calls");
    let msg_bytes = verify
        .store
        .get(&msg_h)
        .expect("get assistant msg")
        .expect("blob assistant msg");
    assert_eq!(msg_bytes.as_ref(), TEXT_BEFORE_TOOL.as_bytes());

    let expected_tc = canonical_cbor_bytes(&[ModelToolCall {
        id: "call-1".into(),
        name: TOOL_NAME.into(),
        arguments: json!({"query": "widgets"}),
    }]);
    let tc_bytes = verify
        .store
        .get(&tc_h)
        .expect("get tool_calls blob")
        .expect("blob tool_calls");
    assert_eq!(tc_bytes.as_ref(), expected_tc.as_slice());

    let tool_call = history.iter().find_map(|(_, e)| match &e.kind {
        EventKind::ToolCall {
            input_hash,
            output_hash,
            side_effects_hash,
            ..
        } => Some((
            input_hash.clone(),
            output_hash.clone(),
            side_effects_hash.clone(),
        )),
        _ => None,
    });
    let (in_h, out_h, side) = tool_call.expect("ToolCall");
    assert!(side.is_none());
    let in_bytes = verify.store.get(&in_h).expect("get").expect("in blob");
    let tool_args: Value = json!({"query": "widgets"});
    assert_eq!(
        in_bytes.as_ref(),
        canonical_cbor_bytes(&tool_args).as_slice()
    );
    let out_expected = canonical_cbor_bytes(&ToolOutput::Success {
        content: TOOL_OUTPUT.into(),
    });
    let out_bytes = verify.store.get(&out_h).expect("get").expect("out blob");
    assert_eq!(out_bytes.as_ref(), out_expected.as_slice());
}

#[tokio::test]
async fn t_full_session_produces_complete_event_sequence_via_drop() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal create");

    let cfg = AgentConfig {
        max_iterations: 8,
        confirmation_timeout_secs: 30,
        session_id: sid,
        auto_commit: false,
        max_completion_tokens: None,
        subagent_style: false,
        max_budget_usd: None,
        fallback_model: None,
        model_estimates: Vec::new(),
    };

    {
        let mut session = AgentSession::new(
            cfg,
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(MultiTurnMockProvider::new(integration_sequences())),
            vec![Box::new(IntegrationSearchTool)],
            Arc::new(Sandbox::new(tmp.path())),
            None,
            false,
            journal,
        )
        .expect("session new");

        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run(
                TURN1_PROMPT.into(),
                tx.clone(),
                &mut no_policy,
                &mut None,
                None,
            )
            .await
            .expect("run 1");
        session
            .run(TURN2_PROMPT.into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run 2");
    }

    let verify = reopen_journal_handle(tmp.path(), sid).expect("reopen after drop");
    let history = {
        let g = verify.graph.lock().expect("graph lock");
        g.history().expect("history")
    };
    assert!(
        matches!(
            history.last().map(|(_, e)| &e.kind),
            Some(EventKind::SessionEnd { .. })
        ),
        "expected SessionEnd after Drop, got {:?}",
        history.last().map(|(_, e)| &e.kind)
    );
    let g = verify.graph.lock().expect("graph lock");
    let report = g.verify().expect("verify");
    assert!(report.missing_objects.is_empty());
    assert!(report.hash_mismatches.is_empty());
    assert!(report.broken_parent_links.is_empty());
}
