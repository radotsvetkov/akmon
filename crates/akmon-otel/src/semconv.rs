//! Pinned OpenTelemetry GenAI semantic-conventions keys and operation values
//! (semconv v1.37.0 structured form).

use serde::{Deserialize, Serialize};

/// The semconv version this importer pins to.
pub const SEMCONV_VERSION: &str = "1.37.0";

// --- Attribute keys (structured GenAI form, v1.37.0) ---

/// `gen_ai.operation.name`.
pub const OPERATION_NAME: &str = "gen_ai.operation.name";
/// `gen_ai.provider.name`.
pub const PROVIDER_NAME: &str = "gen_ai.provider.name";
/// Deprecated `gen_ai.system` (legacy provider identity).
pub const SYSTEM_DEPRECATED: &str = "gen_ai.system";
/// `gen_ai.request.model`.
pub const REQUEST_MODEL: &str = "gen_ai.request.model";
/// `gen_ai.response.model`.
pub const RESPONSE_MODEL: &str = "gen_ai.response.model";
/// `gen_ai.response.id`.
pub const RESPONSE_ID: &str = "gen_ai.response.id";
/// `gen_ai.response.finish_reasons` (array).
pub const FINISH_REASONS: &str = "gen_ai.response.finish_reasons";
/// `gen_ai.usage.input_tokens`.
pub const USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";
/// `gen_ai.usage.output_tokens`.
pub const USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";
/// `gen_ai.request.temperature`.
pub const REQUEST_TEMPERATURE: &str = "gen_ai.request.temperature";
/// `gen_ai.request.max_tokens`.
pub const REQUEST_MAX_TOKENS: &str = "gen_ai.request.max_tokens";
/// `gen_ai.conversation.id`.
pub const CONVERSATION_ID: &str = "gen_ai.conversation.id";

// --- Opt-in content keys (often absent) ---

/// `gen_ai.system_instructions`.
pub const SYSTEM_INSTRUCTIONS: &str = "gen_ai.system_instructions";
/// `gen_ai.input.messages`.
pub const INPUT_MESSAGES: &str = "gen_ai.input.messages";
/// `gen_ai.output.messages`.
pub const OUTPUT_MESSAGES: &str = "gen_ai.output.messages";
/// `gen_ai.tool.call.arguments`.
pub const TOOL_CALL_ARGUMENTS: &str = "gen_ai.tool.call.arguments";
/// `gen_ai.tool.call.result`.
pub const TOOL_CALL_RESULT: &str = "gen_ai.tool.call.result";

// --- Tool keys ---

/// `gen_ai.tool.name`.
pub const TOOL_NAME: &str = "gen_ai.tool.name";
/// `gen_ai.tool.call.id`.
pub const TOOL_CALL_ID: &str = "gen_ai.tool.call.id";
/// `gen_ai.tool.type`.
pub const TOOL_TYPE: &str = "gen_ai.tool.type";
/// `gen_ai.tool.description`.
pub const TOOL_DESCRIPTION: &str = "gen_ai.tool.description";

// --- Error key ---

/// `error.type` (OTLP semconv): present when the span recorded an error.
pub const ERROR_TYPE: &str = "error.type";

/// The recognized GenAI operation kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// `chat` / `generate_content` / `text_completion`: a provider model call.
    ProviderCall,
    /// `embeddings`: an embeddings request (mapped as a provider call).
    Embeddings,
    /// `execute_tool`: a tool execution.
    ExecuteTool,
    /// `create_agent`: agent lifecycle (structural only in v1).
    CreateAgent,
    /// `invoke_agent`: an agent invocation (a candidate session root).
    InvokeAgent,
    /// `invoke_workflow`: a workflow invocation (a candidate session root).
    InvokeWorkflow,
    /// `retrieval`: a retrieval call (structural only in v1).
    Retrieval,
}

impl Operation {
    /// Maps a `gen_ai.operation.name` value to an [`Operation`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "chat" | "generate_content" | "text_completion" => Some(Self::ProviderCall),
            "embeddings" => Some(Self::Embeddings),
            "execute_tool" => Some(Self::ExecuteTool),
            "create_agent" => Some(Self::CreateAgent),
            "invoke_agent" => Some(Self::InvokeAgent),
            "invoke_workflow" => Some(Self::InvokeWorkflow),
            "retrieval" => Some(Self::Retrieval),
            _ => None,
        }
    }

    /// True when this operation can serve as the synthetic session root.
    #[must_use]
    pub fn is_session_root(self) -> bool {
        matches!(self, Self::InvokeAgent | Self::InvokeWorkflow)
    }
}

/// Whether real message/tool content was captured in the trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureLevel {
    /// At least one real message/tool content field was present.
    Full,
    /// Only structural metadata was present (content opt-in was off).
    Structural,
}

impl CaptureLevel {
    /// Lowercase string form used inside content objects.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Structural => "structural",
        }
    }
}
