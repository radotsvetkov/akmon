//! Single-turn LLM generation of `AKMON.md` bodies (no tools).

use akmon_models::{
    CompletionConfig, LlmProvider, Message, MessageRole, ModelError, StopReason, StreamEvent,
};
use futures::StreamExt;

/// System instructions for the one-shot `AKMON.md` author model call.
pub const AKMON_MD_SYSTEM_PROMPT: &str = "You are generating an AKMON.md project memory file. Write in concise markdown. This file will be loaded as system context for an AI coding agent. Be accurate and specific. Output only the markdown content, no preamble.";

/// Calls the configured provider once (streaming) and returns concatenated assistant markdown.
///
/// `title_for_heading` is inserted into the requested document outline so the model keeps a
/// consistent top-level heading.
pub async fn generate_akmon_md_markdown(
    provider: &dyn LlmProvider,
    project_context: &str,
    extra_user_notes: Option<&str>,
    title_for_heading: &str,
) -> Result<String, ModelError> {
    let mut user = String::new();
    user.push_str(project_context);
    if let Some(n) = extra_user_notes {
        let t = n.trim();
        if !t.is_empty() {
            user.push_str("\n\n## Author-provided description\n\n");
            user.push_str(t);
        }
    }
    user.push_str(&format!(
        "\n\nFollow this structure:\n# {title}\n\n## What this is\nOne paragraph describing the project.\n\n## Project structure\nBullet list of key files/directories with one-line descriptions.\n\n## Tech stack\nKey languages, frameworks, and dependencies.\n\n## Conventions\nCoding conventions if detectable from the project.\n\n## Current goals\n(Leave this section empty with a comment: \"# Update this section with your current sprint goals\")\n",
        title = title_for_heading
    ));

    let messages = vec![
        Message {
            role: MessageRole::System,
            content: AKMON_MD_SYSTEM_PROMPT.to_string(),
        },
        Message {
            role: MessageRole::User,
            content: user,
        },
    ];

    let cfg = CompletionConfig {
        tools: Vec::new(),
        ..CompletionConfig::default()
    };
    let mut stream = provider.complete(&messages, &cfg).await?;

    let mut out = String::new();
    let mut finished = false;
    while let Some(item) = stream.next().await {
        match item {
            Ok(StreamEvent::TextDelta { text }) => out.push_str(&text),
            Ok(StreamEvent::Done {
                stop_reason,
                tool_calls,
            }) => {
                if stop_reason == StopReason::ToolUse && !tool_calls.is_empty() {
                    return Err(ModelError::StreamInterrupted {
                        message: "model requested tool calls".into(),
                    });
                }
                finished = true;
                break;
            }
            Ok(StreamEvent::UsageReport(_)) => {}
            Ok(StreamEvent::Error { error }) => return Err(error),
            Err(e) => return Err(e),
        }
    }

    if !finished {
        return Err(ModelError::StreamInterrupted {
            message: "stream ended before completion".into(),
        });
    }

    Ok(out.trim().to_string())
}
