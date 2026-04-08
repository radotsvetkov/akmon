//! Single-turn LLM generation of `AKMON.md` bodies (no tools).

use akmon_models::{
    CompletionConfig, LlmProvider, Message, MessageRole, ModelError, StopReason, StreamEvent,
    max_tokens_for_model,
};
use futures::StreamExt;

/// System instructions for the one-shot `AKMON.md` author model call.
pub const AKMON_MD_SYSTEM_PROMPT: &str = "You are generating an AKMON.md project steering document. Write in concise markdown. This file is loaded as system context for an AI coding agent. Be accurate and specific. The Current sprint section is the highest-impact steering signal—keep it concrete. Output only the markdown content, no preamble.";

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
        "\n\nFollow this structure exactly (use these ## headings):\n\
# {title_for_heading}\n\n\
## Product\n\
What this is, who it is for, what problem it solves. One paragraph. Plain language, no jargon.\n\n\
## Architecture\n\
High-level structure: key components and how they relate. Tell the reader where to look in the repo.\n\n\
## Tech stack\n\
Languages, frameworks, and important crates/packages. Include versions when relevant.\n\n\
## Conventions\n\
Coding standards the agent must follow (error handling, naming, tests layout, commit style, etc.). Use bullets.\n\n\
## Current sprint\n\
What you are working on right now. Update at the start of each session. If unknown, write: _Update this at the start of each work session._\n\n\
## Done\n\
Brief completed milestones for historical context.\n",
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
        max_tokens: max_tokens_for_model(provider.completion_model_id()),
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
            Ok(StreamEvent::ProviderReady { .. }) => {}
            Ok(StreamEvent::StatusHint { .. }) => {}
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
