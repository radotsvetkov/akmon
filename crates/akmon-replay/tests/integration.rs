use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use akmon_core::{AgentConfig, Permission, PolicyEngine, PolicyEngineMode, Sandbox};
use akmon_journal::{
    EventKind, Hash, HashAlgorithm, RedbObjectStore, RedbSessionGraph, SessionGraph,
};
use akmon_models::{
    CompletionStream, LlmProvider, ModelError, ModelToolCall, StopReason, StreamEvent,
};
use akmon_query::{AgentSession, JournalHandle, journal_db_path, open_journal_read_only};
use akmon_replay::{
    ReplayEngine, ReplayEngineConfig, ReplayMode, assemble_report, compare,
    load_source_session_from_journal,
};
use akmon_tools::{Tool, ToolContext, ToolOutput};
use async_trait::async_trait;
use futures::stream;
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::sync::mpsc;
use uuid::Uuid;

const TOOL_NAME: &str = "integration_search";

struct IntegrationSearchTool;

#[async_trait]
impl Tool for IntegrationSearchTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        "replay playback tool for integration_search"
    }

    fn required_permissions(&self) -> &[Permission] {
        &[]
    }

    async fn execute(&self, _args: Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::Success {
            content: "TOOL_OK".to_owned(),
        }
    }
}

struct SequenceMockProvider {
    sequences: Vec<Vec<Result<StreamEvent, ModelError>>>,
    call: AtomicUsize,
}

impl SequenceMockProvider {
    fn new(sequences: Vec<Vec<Result<StreamEvent, ModelError>>>) -> Self {
        Self {
            sequences,
            call: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LlmProvider for SequenceMockProvider {
    fn name(&self) -> &str {
        "integration-provider"
    }

    fn context_window_tokens(&self) -> usize {
        200_000
    }

    fn completion_model_id(&self) -> &str {
        "integration-model"
    }

    async fn complete(
        &self,
        _messages: &[akmon_models::Message],
        _config: &akmon_models::CompletionConfig,
    ) -> Result<CompletionStream, ModelError> {
        let idx = self.call.fetch_add(1, Ordering::SeqCst);
        let events = self.sequences.get(idx).cloned().unwrap_or_default();
        Ok(Box::pin(stream::iter(events)))
    }
}

fn open_writable_journal(
    dir: &Path,
    session_id: Uuid,
) -> JournalHandle<RedbObjectStore, RedbSessionGraph> {
    std::fs::create_dir_all(dir).expect("mkdir");
    let db_path = journal_db_path(dir);
    let store = Arc::new(
        RedbObjectStore::create(db_path.as_path(), HashAlgorithm::Sha256).expect("create store"),
    );
    let graph = RedbSessionGraph::open_new(Arc::clone(&store), session_id).expect("open graph");
    JournalHandle::new(store, Arc::new(Mutex::new(graph)))
}

async fn write_source_session(
    dir: &Path,
    session_id: Uuid,
    prompts: &[&str],
    sequences: Vec<Vec<Result<StreamEvent, ModelError>>>,
) {
    let journal = open_writable_journal(dir, session_id);
    let mut session = AgentSession::new(
        AgentConfig {
            session_id,
            ..AgentConfig::default()
        },
        Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
        Arc::new(SequenceMockProvider::new(sequences)),
        vec![Box::new(IntegrationSearchTool)],
        Arc::new(Sandbox::new(dir)),
        None,
        false,
        journal,
    )
    .expect("session");
    let (tx, _rx) = mpsc::channel(64);
    let mut no_policy = None;
    for prompt in prompts {
        session
            .run(
                (*prompt).to_owned(),
                tx.clone(),
                &mut no_policy,
                &mut None,
                None,
            )
            .await
            .expect("run");
    }
    session.end(None).expect("end");
}

fn one_turn_with_tool_sequences() -> Vec<Vec<Result<StreamEvent, ModelError>>> {
    vec![
        vec![
            Ok(StreamEvent::TextDelta {
                text: "Searching...".to_owned(),
            }),
            Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "call-1".to_owned(),
                    name: TOOL_NAME.to_owned(),
                    arguments: json!({"query":"widgets"}),
                }],
            }),
        ],
        vec![
            Ok(StreamEvent::TextDelta {
                text: "Done.".to_owned(),
            }),
            Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: Vec::new(),
            }),
        ],
    ]
}

fn multi_turn_sequences(turns: usize) -> Vec<Vec<Result<StreamEvent, ModelError>>> {
    (0..turns)
        .map(|idx| {
            vec![
                Ok(StreamEvent::TextDelta {
                    text: format!("turn-{idx}-reply"),
                }),
                Ok(StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    tool_calls: Vec::new(),
                }),
            ]
        })
        .collect()
}

#[tokio::test]
async fn t_replay_full_session_via_real_agent_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    write_source_session(
        tmp.path(),
        sid,
        &["run one turn with tool"],
        one_turn_with_tool_sequences(),
    )
    .await;
    let source = load_source_session_from_journal(tmp.path(), sid).expect("source");
    let engine = ReplayEngine::new(
        source,
        ReplayEngineConfig {
            mode: ReplayMode::Default,
            persist: false,
            persist_journal_dir: None,
        },
    )
    .expect("engine");
    let report = engine.run_to_report().await.expect("report");
    assert!(report.passed, "{report:?}");
    assert_eq!(report.source_event_count, report.replay_event_count);
    assert!(report.divergences.is_empty(), "{report:?}");
}

#[tokio::test]
async fn t_replay_strict_mode_full_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    write_source_session(
        tmp.path(),
        sid,
        &["run one turn with tool"],
        one_turn_with_tool_sequences(),
    )
    .await;
    let source = load_source_session_from_journal(tmp.path(), sid).expect("source");
    let engine = ReplayEngine::new(
        source,
        ReplayEngineConfig {
            mode: ReplayMode::Strict,
            persist: false,
            persist_journal_dir: None,
        },
    )
    .expect("engine");
    let report = engine.run_to_report().await.expect("report");
    assert!(report.passed, "{report:?}");
    assert_eq!(report.source_event_count, report.replay_event_count);
    assert!(report.divergences.is_empty(), "{report:?}");
}

#[tokio::test]
async fn t_replay_persist_creates_inspectable_session() {
    let tmp = tempdir().expect("source tempdir");
    let persist_tmp = tempdir().expect("persist tempdir");
    let persist_tmp_report = tempdir().expect("persist report tempdir");
    let sid = Uuid::new_v4();
    write_source_session(
        tmp.path(),
        sid,
        &["run one turn with tool"],
        one_turn_with_tool_sequences(),
    )
    .await;
    let source = load_source_session_from_journal(tmp.path(), sid).expect("source");
    let engine = ReplayEngine::new(
        source,
        ReplayEngineConfig {
            mode: ReplayMode::Default,
            persist: true,
            persist_journal_dir: Some(persist_tmp.path().to_path_buf()),
        },
    )
    .expect("engine");
    let output = engine.drive_replay().await.expect("drive");
    assert_ne!(output.replay_session_id, output.source_session_id);
    let replay =
        open_journal_read_only(persist_tmp.path(), output.replay_session_id).expect("reopen");
    let history = replay
        .graph
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .history()
        .expect("history");
    let persisted_hashes: Vec<Hash> = history.iter().map(|(h, _)| h.clone()).collect();
    let output_hashes: Vec<Hash> = output
        .replay_history
        .iter()
        .map(|(h, _)| h.clone())
        .collect();
    assert_eq!(persisted_hashes, output_hashes);

    let source_again = load_source_session_from_journal(tmp.path(), sid).expect("source again");
    let report_engine = ReplayEngine::new(
        source_again,
        ReplayEngineConfig {
            mode: ReplayMode::Default,
            persist: true,
            persist_journal_dir: Some(persist_tmp_report.path().to_path_buf()),
        },
    )
    .expect("report engine");
    let report = report_engine.run_to_report().await.expect("report");
    assert!(report.replay_session_id.is_some(), "{report:?}");
}

#[tokio::test]
async fn t_replay_detects_synthetic_divergence() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    write_source_session(
        tmp.path(),
        sid,
        &["run one turn with tool"],
        one_turn_with_tool_sequences(),
    )
    .await;
    let source = load_source_session_from_journal(tmp.path(), sid).expect("source");
    let engine = ReplayEngine::new(
        source,
        ReplayEngineConfig {
            mode: ReplayMode::Default,
            persist: false,
            persist_journal_dir: None,
        },
    )
    .expect("engine");
    let mut output = engine.drive_replay().await.expect("drive");
    if let Some((_, event)) = output
        .replay_history
        .iter_mut()
        .find(|(_, e)| matches!(e.kind, EventKind::UserTurn { .. }))
    {
        event.kind = EventKind::UserTurn {
            prompt_hash: Hash::from_bytes(HashAlgorithm::Sha256, [0xAA; 32]),
        };
    } else {
        panic!("fixture missing UserTurn");
    }
    let engine_divergences = compare(&output);
    let report = assemble_report(output, engine_divergences);
    assert!(!report.passed);
    assert!(!report.divergences.is_empty());
}

#[tokio::test]
async fn t_replay_handles_source_with_no_user_turns() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    write_source_session(tmp.path(), sid, &[], Vec::new()).await;
    let source = load_source_session_from_journal(tmp.path(), sid).expect("source");
    let engine = ReplayEngine::new(
        source,
        ReplayEngineConfig {
            mode: ReplayMode::Default,
            persist: false,
            persist_journal_dir: None,
        },
    )
    .expect("engine");
    let report = engine.run_to_report().await.expect("report");
    assert!(report.passed, "{report:?}");
    assert_eq!(report.source_event_count, 2);
    assert_eq!(report.replay_event_count, 2);
    // events_compared == 2 because SessionStart and SessionEnd are events; they
    // get compared even when there are no UserTurns.
    assert_eq!(report.events_compared, 2);
    assert!(report.divergences.is_empty());
}

#[tokio::test]
async fn t_replay_handles_multi_turn_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let prompts = ["turn 1", "turn 2", "turn 3"];
    write_source_session(
        tmp.path(),
        sid,
        &prompts,
        multi_turn_sequences(prompts.len()),
    )
    .await;
    let source = load_source_session_from_journal(tmp.path(), sid).expect("source");
    let engine = ReplayEngine::new(
        source,
        ReplayEngineConfig {
            mode: ReplayMode::Default,
            persist: false,
            persist_journal_dir: None,
        },
    )
    .expect("engine");
    let report = engine.run_to_report().await.expect("report");
    assert!(report.passed, "{report:?}");
    assert_eq!(report.source_event_count, report.replay_event_count);
    assert!(report.divergences.is_empty(), "{report:?}");
}
