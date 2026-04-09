//! Replace stale [`MessageRole::Tool`] bodies with a short placeholder to trim prompt size.

use akmon_models::{Message, MessageRole, estimate_tokens_for_content};
use akmon_tools::ToolOutput;
use serde_json::{Value, json};

/// Default: tool results older than the last N non-system messages may be cleared.
pub const MICROCOMPACT_KEEP_RECENT_DEFAULT: usize = 20;
/// Groq has no prompt cache — shrink stale tool output sooner.
pub const MICROCOMPACT_KEEP_RECENT_GROQ: usize = 12;

/// Clear shell output only when serialized tool output exceeds this many characters.
const SHELL_OUTPUT_CLEAR_MIN_CHARS: usize = 500;

const CLEARED: &str = "[Old tool result content cleared]";

fn never_clear_tool(name: &str) -> bool {
    matches!(
        name,
        "write_file" | "edit" | "patch" | "apply_patch" | "edit_file"
    )
}

fn clearable_explore_tool(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "list_directory"
            | "search"
            | "semantic_search"
            | "web_fetch"
            | "WebFetch"
            | "grep"
            | "Grep"
            | "glob"
            | "Glob"
            | "web_search"
            | "WebSearch"
    )
}

/// Clears eligible tool payloads in `messages` (mutates in place). Returns estimated tokens reclaimed.
#[must_use]
pub fn apply_microcompact_context(messages: &mut [Message], keep_recent_non_system: usize) -> u32 {
    let n_sys = messages
        .iter()
        .take_while(|m| m.role == MessageRole::System)
        .count();
    let body = &mut messages[n_sys..];
    if body.len() <= keep_recent_non_system {
        return 0;
    }
    let clear_prefix_len = body.len().saturating_sub(keep_recent_non_system);

    let mut saved_tokens: usize = 0;
    for msg in body.iter_mut().take(clear_prefix_len) {
        if msg.role != MessageRole::Tool {
            continue;
        }
        let Ok(mut v) = serde_json::from_str::<Value>(&msg.content) else {
            continue;
        };
        let Some(name) = v.get("tool_name").and_then(|x| x.as_str()) else {
            continue;
        };
        if never_clear_tool(name) {
            continue;
        }

        let clear = if name == "shell" {
            let out_len = v
                .get("output")
                .map(|o| serde_json::to_string(o).unwrap_or_default().len())
                .unwrap_or(0);
            out_len > SHELL_OUTPUT_CLEAR_MIN_CHARS
        } else {
            clearable_explore_tool(name)
        };

        if !clear {
            continue;
        }

        let old_est = estimate_tokens_for_content(&msg.content);
        let placeholder =
            serde_json::to_value(&ToolOutput::Success { content: CLEARED.into() }).unwrap_or(
                json!({ "status": "success", "content": CLEARED }),
            );
        if let Some(out) = v.get_mut("output") {
            *out = placeholder;
        }
        let Some(new_s) = serde_json::to_string(&v).ok() else {
            continue;
        };
        msg.content = new_s;
        let new_est = estimate_tokens_for_content(&msg.content);
        saved_tokens = saved_tokens.saturating_add(old_est.saturating_sub(new_est));
    }

    u32::try_from(saved_tokens).unwrap_or(u32::MAX)
}
