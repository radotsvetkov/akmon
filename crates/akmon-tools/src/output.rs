//! Structured results returned from [`crate::Tool::execute`].

use serde::{Deserialize, Serialize};

/// Machine-readable failure kind for [`ToolOutput::Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorCode {
    /// [`akmon_core::Sandbox::resolve`] rejected the path as outside the sandbox (including symlink escape).
    PathEscape,
    /// Target path does not exist after sandbox resolution.
    NotFound,
    /// Resolved path refers to a directory (or other non-file) when a regular file was required.
    NotAFile,
    /// File bytes are not valid UTF-8.
    BinaryContent,
    /// File or write payload exceeds the configured size limit.
    TooLarge,
    /// Policy denied the action, or an unexpected I/O permission error occurred.
    PermissionDenied,
    /// JSON args are missing required fields or have the wrong shape.
    InvalidArgs,
    /// The search string matched more than once — add surrounding context to make it unique.
    AmbiguousMatch,
    /// The patch could not be applied — the file content may have changed since the patch was generated.
    PatchFailed,
    /// Allowlisted subprocess could not be spawned, waited on, or hit the configured timeout.
    SubprocessFailed,
}

/// Outcome of a single tool invocation (serializable for audit logs and headless JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolOutput {
    /// Tool finished successfully; `content` is user/model-facing text (not raw binary).
    Success {
        /// Human-readable result body.
        content: String,
    },
    /// Tool could not complete; `code` categorizes the failure for the agent loop.
    Error {
        /// Stable error category.
        code: ToolErrorCode,
        /// Explanation safe to log (no secrets).
        message: String,
    },
    /// Interactive-only: ask the user a question and block until the UI supplies an answer.
    ///
    /// The session layer turns this into [`ToolOutput::Success`] after the answer is received.
    Question {
        /// Question body shown to the user.
        question: String,
        /// Optional quick-reply hints (UI may show as numbered options).
        suggestions: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_output_success_json_shape() {
        let o = ToolOutput::Success {
            content: "hello".into(),
        };
        let v = serde_json::to_value(&o).expect("serialize");
        assert_eq!(v["status"], "success");
        assert_eq!(v["content"], "hello");
    }

    #[test]
    fn tool_output_error_json_shape() {
        let o = ToolOutput::Error {
            code: ToolErrorCode::InvalidArgs,
            message: "missing path".into(),
        };
        let v = serde_json::to_value(&o).expect("serialize");
        assert_eq!(v["status"], "error");
        assert_eq!(v["code"], "invalid_args");
        assert_eq!(v["message"], "missing path");
    }

    #[test]
    fn tool_output_question_json_shape() {
        let o = ToolOutput::Question {
            question: "Which port?".into(),
            suggestions: vec!["3000".into()],
        };
        let v = serde_json::to_value(&o).expect("serialize");
        assert_eq!(v["status"], "question");
        assert_eq!(v["question"], "Which port?");
    }

    #[test]
    fn patch_failed_serializes_to_snake_case() {
        let o = ToolOutput::Error {
            code: ToolErrorCode::PatchFailed,
            message: "hunk 2 failed".into(),
        };
        let v = serde_json::to_value(&o).expect("serialize");
        assert_eq!(v["status"], "error");
        assert_eq!(v["code"], "patch_failed");
        assert_eq!(v["message"], "hunk 2 failed");
    }

    #[test]
    fn ambiguous_match_serializes_to_snake_case() {
        let o = ToolOutput::Error {
            code: ToolErrorCode::AmbiguousMatch,
            message: "old_str matches 2 times".into(),
        };
        let v = serde_json::to_value(&o).expect("serialize");
        assert_eq!(v["status"], "error");
        assert_eq!(v["code"], "ambiguous_match");
        assert_eq!(v["message"], "old_str matches 2 times");
    }
}
