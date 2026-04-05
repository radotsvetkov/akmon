//! Building chat messages for the model from project config and session history.

use akmon_models::{Message, MessageRole};

/// Delimiters wrapping `AKMON.md` so the block reads as **data**, not as hidden instructions.
///
/// Triple-angle brackets are uncommon in natural prose and Markdown, so the model is less likely
/// to treat the file body as a second system prompt (prompt-injection mitigation). The explicit
/// `AKMON_MD` labels make audits and logs unambiguous.
pub const AKMON_MD_START: &str = "<<<AKMON_MD_START>>>";
/// Closing delimiter paired with [`AKMON_MD_START`].
pub const AKMON_MD_END: &str = "<<<AKMON_MD_END>>>";

/// Opening delimiter for the fixed project / tool / path-hint system block.
pub const PROJECT_CONTEXT_START: &str = "<<<PROJECT_CONTEXT_START>>>";
/// Closing delimiter paired with [`PROJECT_CONTEXT_START`].
pub const PROJECT_CONTEXT_END: &str = "<<<PROJECT_CONTEXT_END>>>";

fn format_project_context(project_root: &str, tool_names: &[&str]) -> String {
    let tools_line = tool_names.join(", ");
    let web_fetch_line = if tool_names.contains(&"web_fetch") {
        "  7. web_fetch url=\"https://...\" \
     to fetch public documentation\n"
    } else {
        ""
    };
    format!(
        "{PROJECT_CONTEXT_START}\n\
You are an AI coding assistant \n\
running inside the Akmon agent.\n\
\n\
Working directory: {project_root}\n\
Available tools: {tools_line}\n\
\n\
To work on this project:\n\
  1. list_directory path=\".\" \n\
     to explore structure\n\
  2. search pattern=\"...\" \n\
     to find relevant code\n\
  3. read_file path=\"...\" \n\
     to read specific files\n\
  4. edit path=\"...\" old_str=\"...\" \n\
     new_str=\"...\" for surgical \n\
     single-location changes\n\
  5. patch patch=\"...\" for changes \n\
     across multiple locations\n\
  6. write_file path=\"...\" \n\
     content=\"...\" only for \n\
     completely new files\n\
{web_fetch_line}\
  NEVER rewrite an entire existing \n\
  file — use edit or patch instead.\n\
  NEVER call read_file on a directory.\n\
  NEVER guess paths — always \n\
  list or search first.\n\
\n\
All paths must be relative to the \n\
working directory shown above.\n\
Absolute paths and paths with ../ \n\
will be rejected by the sandbox.\n\
{PROJECT_CONTEXT_END}"
    )
}

/// Builds the message list for one model call: optional system block from `AKMON.md`, a fixed
/// project-context system message, prior [`Message`] history in order, then the current user task
/// as the final message.
pub fn build_messages(
    akmon_md: Option<&str>,
    history: &[Message],
    task: &str,
    project_root: &str,
    tool_names: &[&str],
) -> Vec<Message> {
    let mut out = Vec::new();

    if let Some(md) = akmon_md {
        let body = format!(
            "Project configuration (AKMON.md):\n{}\n{md}\n{}",
            AKMON_MD_START, AKMON_MD_END,
        );
        out.push(Message {
            role: MessageRole::System,
            content: body,
        });
    }

    out.push(Message {
        role: MessageRole::System,
        content: format_project_context(project_root, tool_names),
    });

    out.extend(history.iter().cloned());

    out.push(Message {
        role: MessageRole::User,
        content: task.to_string(),
    });

    out
}

/// Builds messages for a follow-up model call after tool results (no extra trailing user line).
///
/// Use after the first turn once [`MessageRole::User`] / assistant / tool rows are already in
/// `history`.
pub fn build_followup_messages(
    akmon_md: Option<&str>,
    history: &[Message],
    project_root: &str,
    tool_names: &[&str],
) -> Vec<Message> {
    let mut out = Vec::new();

    if let Some(md) = akmon_md {
        let body = format!(
            "Project configuration (AKMON.md):\n{}\n{md}\n{}",
            AKMON_MD_START, AKMON_MD_END,
        );
        out.push(Message {
            role: MessageRole::System,
            content: body,
        });
    }

    out.push(Message {
        role: MessageRole::System,
        content: format_project_context(project_root, tool_names),
    });

    out.extend(history.iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_messages_with_akmon_md_starts_with_delimited_system() {
        let msgs = build_messages(Some("rules"), &[], "do it", "/repo", &["read_file"]);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, MessageRole::System);
        assert!(msgs[0].content.contains("Project configuration (AKMON.md):"));
        assert!(msgs[0].content.contains(AKMON_MD_START));
        assert!(msgs[0].content.contains("rules"));
        assert!(msgs[0].content.contains(AKMON_MD_END));
        assert_eq!(msgs[1].role, MessageRole::System);
        assert!(msgs[1].content.contains(PROJECT_CONTEXT_START));
        assert!(msgs[1].content.contains("Working directory: /repo"));
        assert!(msgs[1].content.contains("Available tools: read_file"));
        assert!(msgs[1].content.contains("To work on this project:"));
        assert!(msgs[1].content.contains("edit path"));
        assert!(msgs[1].content.contains("patch patch"));
        assert!(msgs[1].content.contains(PROJECT_CONTEXT_END));
    }

    #[test]
    fn build_messages_without_akmon_md_still_injects_project_context() {
        let msgs = build_messages(None, &[], "task", "/wd", &["a", "b"]);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, MessageRole::System);
        assert!(msgs[0].content.contains(PROJECT_CONTEXT_START));
        assert!(msgs[0].content.contains("Working directory: /wd"));
        assert!(msgs[0].content.contains("Available tools: a, b"));
        assert_eq!(msgs[1].role, MessageRole::User);
        assert_eq!(msgs[1].content, "task");
    }

    #[test]
    fn build_messages_last_is_always_user_task() {
        let hist = vec![Message {
            role: MessageRole::Assistant,
            content: "prev".into(),
        }];
        let msgs = build_messages(None, &hist, "final ask", "/", &[]);
        let last = msgs
            .last()
            .expect("build_messages always ends with the user task");
        assert_eq!(last.role, MessageRole::User);
        assert_eq!(last.content, "final ask");
    }

    #[test]
    fn project_context_mentions_web_fetch_when_tool_enabled() {
        let msgs = build_messages(None, &[], "t", "/repo", &["read_file", "web_fetch"]);
        let ctx = msgs
            .iter()
            .find(|m| m.role == MessageRole::System && m.content.contains("web_fetch url="))
            .expect("project context should document web_fetch when listed");
        assert!(ctx.content.contains("7. web_fetch url=\"https://...\""));
        assert!(ctx.content.contains("fetch public documentation"));
    }
}
