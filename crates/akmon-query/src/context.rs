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

/// Long static tool reference inlined at compile time. Dual purpose: (1) accurate parameter and
/// output documentation for the model, and (2) enough text that the Anthropic system block meets
/// the prompt-cache minimum for **`claude-haiku-4-5-20251001`** (4096+ tokens in the cached block
/// per Anthropic documentation).
const TOOL_REFERENCE: &str = include_str!("tool_reference.txt");

/// Markdown-style instructions injected when the session is in read-only plan mode (`--plan`, `/plan`).
pub const PLAN_MODE_SYSTEM_ADDON: &str = "\n\
PLAN MODE ACTIVE.\n\
You are in read-only analysis mode.\n\
You CANNOT write, edit, or delete files.\n\
You CANNOT run shell commands.\n\
You CANNOT commit to git.\n\
\n\
Your task: analyze the codebase and produce a detailed implementation plan.\n\
\n\
Your plan MUST include:\n\
1. Understanding: what does the relevant existing code do?\n\
   Reference specific files and functions by name.\n\
2. Impact analysis: which files will need to change and why?\n\
3. New files: what new files are needed and what will they contain?\n\
4. Implementation sequence: in what order should changes be made, and why that order?\n\
5. Risks: what could go wrong, and how should it be handled?\n\
6. Open questions: what do you need the developer to clarify before implementation?\n\
\n\
Format the plan as a markdown document. Be specific — include exact file paths, function names, struct names, and type signatures.\n\
Do not be vague. \"Update the policy engine\" is not useful. \"Add a new NetworkFetch variant to the Permission enum in\n\
crates/akmon-core/src/permission.rs and update the match arms in\n\
policy.rs lines 45-67\" is useful.\n";

fn format_project_context_plan_mode(
    project_root: &str,
    tool_names: &[&str],
    has_semantic: bool,
    has_web_fetch: bool,
) -> String {
    let tools_line = tool_names.join(", ");
    let step1 = if has_semantic {
        "  STEP 1 — Understand the codebase:\n\
    Start with semantic_search for conceptual queries before search or list_directory.\n"
    } else {
        "  STEP 1 — Explore:\n\
    Use search with focused patterns; avoid list_directory-only loops.\n"
    };

    let semantic_block = if has_semantic {
        "\n\
SEMANTIC SEARCH IS AVAILABLE.\n\
Use it as your primary exploration tool.\n\
Examples of good semantic_search queries:\n\
  \"policy permission evaluation\"\n\
  \"error handling for file operations\"\n\
Examples of bad semantic_search queries (use search instead):\n\
  \"fn validate_url\" ← use search\n\
  \"TODO\" ← use search\n\n"
    } else {
        ""
    };

    let step_web = if has_web_fetch {
        "\n\
  STEP 5 — Optional documentation:\n\
    web_fetch url=\"https://...\"\n\
    Only when web_fetch is in your tool list.\n"
    } else {
        ""
    };

    format!(
        "{PROJECT_CONTEXT_START}\n\
You are an AI coding assistant running inside the Akmon agent.\n\
\n\
Working directory: {project_root}\n\
Available tools (read-only session): {tools_line}\n\
{PLAN_MODE_SYSTEM_ADDON}\n\
To work on this project in PLAN MODE:\n\
{step1}\
\n\
  STEP 2 — Navigate structure:\n\
    list_directory path=\".\" only when you need to see filenames.\n\
\n\
  STEP 3 — Find exact strings:\n\
    search pattern=\"exact_string\" for identifiers and literals.\n\
\n\
  STEP 4 — Read specific files:\n\
    read_file path=\"...\" after you know a file is relevant.\n\
{step_web}\
RULES:\n\
  - Produce a written plan only; do not propose tool calls that modify the repository.\n\
  - read_file only after locating paths via semantic_search or search.\n\
  - NEVER guess file paths.\n\
{semantic_block}\
All paths must be relative to the working directory shown above.\n\
\n\
<<<TOOL_REFERENCE_START>>>\n\
{TOOL_REFERENCE}\
<<<TOOL_REFERENCE_END>>>\n\
{PROJECT_CONTEXT_END}"
    )
}

fn format_project_context(project_root: &str, tool_names: &[&str], plan_mode: bool) -> String {
    let has_semantic = tool_names.contains(&"semantic_search");
    let has_web_fetch = tool_names.contains(&"web_fetch");
    if plan_mode {
        return format_project_context_plan_mode(
            project_root,
            tool_names,
            has_semantic,
            has_web_fetch,
        );
    }
    let tools_line = tool_names.join(", ");
    let has_git = tool_names.contains(&"git");
    let has_shell = tool_names.contains(&"shell");

    let step1 = if has_semantic {
        "  STEP 1 — Understand the codebase:\n\
    If --index is available, ALWAYS\n\
    start with semantic_search for \n\
    any conceptual or exploratory query:\n\
      semantic_search query=\"error handling\"\n\
      semantic_search query=\"authentication\"\n\
      semantic_search query=\"policy evaluation\"\n\
    semantic_search finds relevant code\n\
    across the entire project by meaning,\n\
    not just by string matching.\n\
    Use it BEFORE search or list_directory\n\
    for any task involving understanding\n\
    or finding code.\n"
    } else {
        "  STEP 1 — Understand the codebase:\n\
    The semantic_search tool is NOT available\n\
    (enable with CLI --index). For exploration,\n\
    use search with focused patterns and read_file\n\
    on likely files; avoid list_directory-only loops.\n"
    };

    let step4b = if has_git {
        "\n\
  STEP 4b — Check git state:\n\
    git subcommand=\"status\"\n\
      before editing to see what\n\
      is already changed.\n\
    git subcommand=\"diff\"\n\
      after editing to verify \n\
      your changes look correct.\n\
    git subcommand=\"commit\" \n\
      message=\"feat: ...\" \n\
      after staging with git add.\n\
    ALWAYS commit working changes \n\
    before starting a new task.\n\
    ALWAYS check git status at the \n\
    start of any editing task.\n"
    } else {
        ""
    };

    let step8_shell = if has_shell {
        "\n\
  STEP 8 — Run commands:\n\
    shell command=\"...\"\n\
    Only commands in the allowlist.\n"
    } else {
        ""
    };

    let step9_web = if has_web_fetch {
        "\n\
  STEP 9 — Fetch documentation:\n\
    web_fetch url=\"https://...\"\n\
    Only when --web-fetch is enabled.\n"
    } else {
        ""
    };

    let rule_semantic_priority = if has_semantic {
        "  - semantic_search before search \n\
    for ANY conceptual query\n"
    } else {
        "  - Without --index, use search plus \n\
    read_file for exploration\n"
    };

    let step4_discover_tools = if has_semantic {
        "semantic_search or list_directory.\n"
    } else {
        "search or list_directory.\n"
    };

    let step3_tail = if has_semantic {
        "    Do NOT use search for conceptual \n\
    queries — use semantic_search instead.\n"
    } else {
        "    Without semantic_search, combine \n\
    search hits with read_file; avoid \n\
    list_directory-only discovery.\n"
    };

    let semantic_block = if has_semantic {
        "\n\
SEMANTIC SEARCH IS AVAILABLE.\n\
Use it as your primary exploration tool.\n\
Examples of good semantic_search queries:\n\
  \"policy permission evaluation\"\n\
  \"error handling for file operations\"\n\
  \"agent FSM state transitions\"\n\
  \"MCP tool discovery\"\n\
Examples of bad semantic_search queries\n\
(use search instead):\n\
  \"fn validate_url\" ← use search\n\
  \"TODO\" ← use search\n\
  \"use akmon_core\" ← use search\n\n"
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
{step1}\
\n\
  STEP 2 — Navigate structure:\n\
    list_directory path=\".\" to explore\n\
    Only use list_directory when you need\n\
    to know what files exist, not to \n\
    find relevant code.\n\
\n\
  STEP 3 — Find exact strings:\n\
    search pattern=\"exact_string\"\n\
    Use search for exact identifier names,\n\
    function names, or literal strings.\n\
{step3_tail}\
\n\
  STEP 4 — Read specific files:\n\
    read_file path=\"...\"\n\
    Only read a file after you know it\n\
    is relevant. Never read files to \n\
    discover structure — use \n\
    {step4_discover_tools}\
{step4b}\
\n\
  STEP 5 — Edit existing files:\n\
    edit path=\"...\" old_str=\"...\" \n\
      new_str=\"...\"\n\
    ALWAYS use edit for changes to \n\
    existing files. Never rewrite an \n\
    entire file.\n\
\n\
  STEP 6 — Apply diffs:\n\
    patch patch=\"...\"\n\
    Use for multi-location changes.\n\
\n\
  STEP 7 — New files only:\n\
    write_file path=\"...\" content=\"...\"\n\
    Only for files that do not exist yet.\n\
{step8_shell}\
{step9_web}\
RULES:\n\
{rule_semantic_priority}\
  - read_file only after locating \n\
    the file via semantic_search \n\
    or search\n\
  - NEVER list_directory to find \n\
    relevant code\n\
  - NEVER guess file paths\n\
  - NEVER rewrite entire existing files\n\
\n\
{semantic_block}\
All paths must be relative to the \n\
working directory shown above.\n\
Absolute paths and paths with ../ \n\
will be rejected by the sandbox.\n\
\n\
<<<TOOL_REFERENCE_START>>>\n\
{TOOL_REFERENCE}\
<<<TOOL_REFERENCE_END>>>\n\
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
    plan_mode: bool,
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
        content: format_project_context(project_root, tool_names, plan_mode),
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
    plan_mode: bool,
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
        content: format_project_context(project_root, tool_names, plan_mode),
    });

    out.extend(history.iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_messages_with_akmon_md_starts_with_delimited_system() {
        let msgs = build_messages(Some("rules"), &[], "do it", "/repo", &["read_file"], false);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, MessageRole::System);
        assert!(
            msgs[0]
                .content
                .contains("Project configuration (AKMON.md):")
        );
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
        assert!(msgs[1].content.contains("<<<TOOL_REFERENCE_START>>>"));
    }

    /// Anthropic Claude Haiku 4.5 requires a large cacheable prefix (4096+ tokens); this guards the
    /// padded project-context block plus typical `AKMON.md` so caching can activate.
    #[test]
    fn combined_system_messages_meet_haiku_45_cache_char_threshold() {
        let akmon = include_str!("../../../AKMON.md");
        let msgs = build_messages(
            Some(akmon),
            &[],
            "task",
            "/repo",
            &[
                "read_file",
                "write_file",
                "list_directory",
                "search",
                "edit",
                "patch",
            ],
            false,
        );
        let mut joined_len = 0usize;
        let mut n_sys = 0usize;
        for m in &msgs {
            if m.role == MessageRole::System {
                joined_len += m.content.len();
                n_sys += 1;
            }
        }
        if n_sys > 1 {
            joined_len += 2 * (n_sys - 1);
        }
        let approx_tokens = joined_len as f64 / 3.5;
        assert!(
            approx_tokens >= 4096.0,
            "approx_tokens={approx_tokens} joined_chars={joined_len} (need >= 4096 for Haiku 4.5 cache minimum)"
        );
    }

    #[test]
    fn project_context_alone_meets_haiku_45_cache_char_threshold_without_akmon_md() {
        let msgs = build_messages(None, &[], "t", "/wd", &["read_file"], false);
        let sys = msgs
            .iter()
            .find(|m| m.role == MessageRole::System)
            .expect("project context");
        let approx_tokens = sys.content.len() as f64 / 3.5;
        assert!(
            approx_tokens >= 4096.0,
            "approx_tokens={approx_tokens} chars={} (no AKMON.md)",
            sys.content.len()
        );
    }

    #[test]
    fn build_messages_without_akmon_md_still_injects_project_context() {
        let msgs = build_messages(None, &[], "task", "/wd", &["a", "b"], false);
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
        let msgs = build_messages(None, &hist, "final ask", "/", &[], false);
        let last = msgs
            .last()
            .expect("build_messages always ends with the user task");
        assert_eq!(last.role, MessageRole::User);
        assert_eq!(last.content, "final ask");
    }

    #[test]
    fn project_context_mentions_web_fetch_when_tool_enabled() {
        let msgs = build_messages(None, &[], "t", "/repo", &["read_file", "web_fetch"], false);
        let ctx = msgs
            .iter()
            .find(|m| m.role == MessageRole::System && m.content.contains("web_fetch url="))
            .expect("project context should document web_fetch when listed");
        assert!(ctx.content.contains("STEP 9 — Fetch documentation"));
        assert!(ctx.content.contains("web_fetch url=\"https://...\""));
    }

    #[test]
    fn project_context_mentions_semantic_search_when_tool_enabled() {
        let msgs = build_messages(
            None,
            &[],
            "t",
            "/repo",
            &["read_file", "semantic_search"],
            false,
        );
        let ctx = msgs
            .iter()
            .find(|m| m.role == MessageRole::System && m.content.contains("semantic_search query="))
            .expect("project context should document semantic_search when listed");
        assert!(ctx.content.contains("STEP 1 — Understand the codebase"));
        assert!(
            ctx.content
                .contains("semantic_search query=\"error handling\"")
        );
        assert!(ctx.content.contains("SEMANTIC SEARCH IS AVAILABLE."));
        assert!(
            ctx.content
                .contains("Examples of good semantic_search queries")
        );
        assert!(
            ctx.content
                .contains("Examples of bad semantic_search queries")
        );
    }

    #[test]
    fn semantic_search_listed_before_search_in_context() {
        let msgs = build_messages(
            None,
            &[],
            "t",
            "/repo",
            &["read_file", "search", "semantic_search"],
            false,
        );
        let ctx = msgs
            .iter()
            .find(|m| m.role == MessageRole::System)
            .expect("system");
        let body = &ctx.content;
        let i_sem = body
            .find("semantic_search query=\"error handling\"")
            .expect("semantic_search examples");
        let i_search = body
            .find("search pattern=\"exact_string\"")
            .expect("search step");
        assert!(
            i_sem < i_search,
            "semantic_search guidance must appear before exact-string search step"
        );
    }

    #[test]
    fn plan_mode_system_contains_active_banner() {
        let msgs = build_messages(
            None,
            &[],
            "task",
            "/r",
            &["read_file", "search", "list_directory"],
            true,
        );
        let ctx = msgs
            .iter()
            .find(|m| m.role == MessageRole::System)
            .expect("ctx");
        assert!(ctx.content.contains("PLAN MODE ACTIVE."));
        assert!(!ctx.content.contains("STEP 5 — Edit existing files"));
    }
}
