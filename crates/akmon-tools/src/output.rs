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
}
