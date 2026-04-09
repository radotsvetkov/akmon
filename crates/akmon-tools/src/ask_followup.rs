//! Ask the user a clarifying question (interactive TUI only).

use akmon_core::Permission;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::Tool;
use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};

/// Ask the user a question and wait for a typed answer (handled by the TUI).
pub struct AskFollowupTool;

#[derive(Deserialize)]
struct Input {
    question: String,
    #[serde(default)]
    suggestions: Vec<String>,
}

#[async_trait]
impl Tool for AskFollowupTool {
    fn name(&self) -> &str {
        "ask_followup"
    }

    fn description(&self) -> &str {
        "Ask the user a question and wait for their answer. \
         Use when you need clarification before proceeding. \
         Do not use this more than once per turn."
    }

    fn required_permissions(&self) -> &[Permission] {
        &[]
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                },
                "suggestions": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional suggested answers (shown as options)"
                }
            },
            "required": ["question"]
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> ToolOutput {
        if !ctx.is_interactive() {
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: "ask_followup is not available in headless mode. \
                 Use --task to provide all required information upfront."
                    .into(),
            };
        }

        let input: Input = match serde_json::from_value(input) {
            Ok(v) => v,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: format!("Invalid input: {e}"),
                };
            }
        };

        ToolOutput::Question {
            question: input.question,
            suggestions: input.suggestions,
        }
    }
}
