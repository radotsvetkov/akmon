//! OpenTelemetry GenAI semantic-conventions keys, event names and operation
//! values understood by the importer.
//!
//! The GenAI conventions lived in the core `open-telemetry/semantic-conventions`
//! repository up to v1.41 and moved to `open-telemetry/semantic-conventions-genai`
//! with v1.42 (still marked Development there). The importer understands the
//! structured form introduced in v1.37 through the v1.42.x GenAI repository
//! (attribute content, the `gen_ai.client.inference.operation.details` event,
//! `gen_ai.evaluation.result`, and the current operation names), plus the legacy
//! (<= v1.36) message-event form.

use serde::{Deserialize, Serialize};

/// The newest GenAI semantic-conventions version this importer is tested
/// against (structured form v1.37 .. v1.42.x).
///
/// This is the importer's *supported* version. What the producer actually
/// declared (its `schemaUrl`) is recorded separately as `source_semconv`.
pub const SEMCONV_VERSION: &str = "1.42.0";

// --- Attribute keys (structured GenAI form) ---

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
/// `gen_ai.usage.cache_read.input_tokens` (v1.40+).
pub const USAGE_CACHE_READ_INPUT_TOKENS: &str = "gen_ai.usage.cache_read.input_tokens";
/// `gen_ai.usage.cache_creation.input_tokens` (v1.40+).
pub const USAGE_CACHE_CREATION_INPUT_TOKENS: &str = "gen_ai.usage.cache_creation.input_tokens";
/// `gen_ai.usage.reasoning.output_tokens` (v1.41+).
pub const USAGE_REASONING_OUTPUT_TOKENS: &str = "gen_ai.usage.reasoning.output_tokens";
/// `gen_ai.request.temperature`.
pub const REQUEST_TEMPERATURE: &str = "gen_ai.request.temperature";
/// `gen_ai.request.max_tokens`.
pub const REQUEST_MAX_TOKENS: &str = "gen_ai.request.max_tokens";
/// `gen_ai.request.stream` (v1.41+).
pub const REQUEST_STREAM: &str = "gen_ai.request.stream";
/// `gen_ai.output.type`.
pub const OUTPUT_TYPE: &str = "gen_ai.output.type";
/// `gen_ai.conversation.id`.
pub const CONVERSATION_ID: &str = "gen_ai.conversation.id";
/// `gen_ai.agent.name`.
pub const AGENT_NAME: &str = "gen_ai.agent.name";
/// `gen_ai.agent.id`.
pub const AGENT_ID: &str = "gen_ai.agent.id";
/// `gen_ai.agent.version` (v1.40+).
pub const AGENT_VERSION: &str = "gen_ai.agent.version";

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
/// `gen_ai.tool.definitions` (v1.38+, opt-in).
pub const TOOL_DEFINITIONS: &str = "gen_ai.tool.definitions";

// --- Tool keys ---

/// `gen_ai.tool.name`.
pub const TOOL_NAME: &str = "gen_ai.tool.name";
/// `gen_ai.tool.call.id`.
pub const TOOL_CALL_ID: &str = "gen_ai.tool.call.id";

// --- Error keys ---

/// `error.type` (OTLP semconv): present when the span recorded an error.
pub const ERROR_TYPE: &str = "error.type";
/// Generic OTel `exception` span event (records a failure).
pub const EVENT_EXCEPTION: &str = "exception";
/// `exception.type` attribute of an exception event.
pub const EXCEPTION_TYPE: &str = "exception.type";
/// `exception.message` attribute of an exception event.
pub const EXCEPTION_MESSAGE: &str = "exception.message";

// --- Current (non-legacy) GenAI event names ---

/// `gen_ai.client.inference.operation.details` (v1.37+): the event form of the
/// input/output messages and system instructions. Imported as content.
pub const EVENT_INFERENCE_DETAILS: &str = "gen_ai.client.inference.operation.details";
/// `gen_ai.client.operation.exception` (v1.41+): records a failed operation.
pub const EVENT_OPERATION_EXCEPTION: &str = "gen_ai.client.operation.exception";

// --- Legacy (<= v1.36) span-event names ---
//
// These are the five supported legacy GenAI message-event forms. A span carrying
// these events (rather than the structured `gen_ai.input.messages` /
// `gen_ai.output.messages` attributes) is reduced to the same structured content.

/// Legacy `gen_ai.system.message` event (system instruction body).
pub const EVENT_SYSTEM_MESSAGE: &str = "gen_ai.system.message";
/// Legacy `gen_ai.user.message` event (user message body).
pub const EVENT_USER_MESSAGE: &str = "gen_ai.user.message";
/// Legacy `gen_ai.assistant.message` event (assistant message body).
pub const EVENT_ASSISTANT_MESSAGE: &str = "gen_ai.assistant.message";
/// Legacy `gen_ai.tool.message` event (tool result body).
pub const EVENT_TOOL_MESSAGE: &str = "gen_ai.tool.message";
/// Legacy `gen_ai.choice` event (model response choice body).
pub const EVENT_CHOICE: &str = "gen_ai.choice";

/// The five supported legacy GenAI message-event names.
pub const SUPPORTED_LEGACY_EVENTS: [&str; 5] = [
    EVENT_SYSTEM_MESSAGE,
    EVENT_USER_MESSAGE,
    EVENT_ASSISTANT_MESSAGE,
    EVENT_TOOL_MESSAGE,
    EVENT_CHOICE,
];

// --- Legacy (<= v1.36) event-body attribute keys ---

/// Legacy event-body `role` attribute.
pub const BODY_ROLE: &str = "role";
/// Legacy event-body `content` attribute.
pub const BODY_CONTENT: &str = "content";
/// Legacy event-body `id` attribute (tool call/result id).
pub const BODY_ID: &str = "id";
/// Legacy event-body `tool_calls` attribute (JSON array of OpenAI-style calls).
pub const BODY_TOOL_CALLS: &str = "tool_calls";
/// Legacy event-body `index` attribute (choice index).
pub const BODY_INDEX: &str = "index";
/// Legacy event-body `finish_reason` attribute (choice finish reason).
pub const BODY_FINISH_REASON: &str = "finish_reason";
/// Legacy event-body `message` attribute (a `gen_ai.choice` nested message JSON).
pub const BODY_MESSAGE: &str = "message";

/// Classification of a span event name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenAiEventClass {
    /// One of the five supported legacy (<= v1.36) message events.
    SupportedLegacy,
    /// A legacy-shaped message event (`gen_ai.<role>.message`) we cannot reduce.
    UnknownLegacy,
    /// The inference-details content event.
    InferenceDetails,
    /// A failure-recording event (`exception`, `gen_ai.client.operation.exception`).
    Exception,
    /// Any other (current or future) non-legacy `gen_ai.*` event, for example
    /// `gen_ai.evaluation.result`: recorded as structural metadata.
    StructuralMetadata,
    /// Not a GenAI event at all.
    NotGenAi,
}

/// Classifies a span event name (OTEL-2).
///
/// Only legacy-*shaped* message events that are not among the five supported
/// forms are refused: the legacy convention named its message events
/// `gen_ai.<role>.message`, so an unknown `gen_ai.<x>.message` is an ambiguous
/// legacy message body we cannot reduce. Every other unknown `gen_ai.*` event is
/// treated as current-convention structural metadata and recorded, not refused.
#[must_use]
pub fn classify_event(name: &str) -> GenAiEventClass {
    if name == EVENT_EXCEPTION || name == EVENT_OPERATION_EXCEPTION {
        return GenAiEventClass::Exception;
    }
    let Some(rest) = name.strip_prefix("gen_ai.") else {
        return GenAiEventClass::NotGenAi;
    };
    if SUPPORTED_LEGACY_EVENTS.contains(&name) {
        return GenAiEventClass::SupportedLegacy;
    }
    if name == EVENT_INFERENCE_DETAILS {
        return GenAiEventClass::InferenceDetails;
    }
    let legacy_shaped = rest
        .strip_suffix(".message")
        .is_some_and(|role| !role.is_empty() && !role.contains('.'));
    if legacy_shaped {
        GenAiEventClass::UnknownLegacy
    } else {
        GenAiEventClass::StructuralMetadata
    }
}

/// The recognized GenAI operation kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// `chat` / `generate_content` / `text_completion`: a provider model call.
    ProviderCall,
    /// `embeddings`: an embeddings request (mapped as a provider call).
    Embeddings,
    /// `execute_tool`: a tool execution.
    ExecuteTool,
    /// `create_agent`: agent lifecycle (structural only).
    CreateAgent,
    /// `invoke_agent`: an agent invocation (a candidate session root).
    InvokeAgent,
    /// `invoke_workflow`: a workflow invocation (a candidate session root).
    InvokeWorkflow,
    /// `retrieval`: a retrieval call (structural only).
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

    /// True when this operation produces an AGEF event (provider or tool call).
    #[must_use]
    pub fn emits_event(self) -> bool {
        matches!(
            self,
            Self::ProviderCall | Self::Embeddings | Self::ExecuteTool
        )
    }
}

/// Whether real message/tool content was captured for the session (OTEL-1).
///
/// The level is computed from what the importer actually *emitted*, slot by
/// slot. The content slots are exactly:
///
/// - per chat / text-completion / generate-content provider call: the **request**
///   slot, real only when **input messages** were captured
///   (`gen_ai.input.messages` or its details-event / legacy equivalent) — captured
///   `gen_ai.system_instructions` alone do NOT make it real, because the
///   user/input side of the conversation is still missing; and the **response**
///   slot, real when output messages were captured;
/// - per `execute_tool` call: the **input** slot (`gen_ai.tool.call.arguments`)
///   and the **output** slot (`gen_ai.tool.call.result`).
///
/// Embeddings calls emit a provider-call event but contribute NO slots (the
/// semantic conventions define no message content for embeddings). A slot counts
/// as real content only when it was filled from captured message/tool content (a
/// structured attribute, an inference-details event, or a legacy message body)
/// that is non-empty; empty or whitespace-only strings, `null`, `[]` and `{}`
/// count as absent. Content on spans that emit no event (for example an
/// `invoke_agent` span) does not count. A session with no slots at all (for
/// example embeddings only) is `structural`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureLevel {
    /// Every content slot of every emitted provider/tool call holds real content.
    Full,
    /// Some, but not all, content slots hold real content.
    Partial,
    /// No content slot holds real content (metadata only; content opt-in was off).
    Structural,
}

impl CaptureLevel {
    /// Lowercase string form used inside content objects.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Partial => "partial",
            Self::Structural => "structural",
        }
    }

    /// Derives the level from the number of real-content slots out of all slots.
    #[must_use]
    pub fn from_slots(real: u64, total: u64) -> Self {
        if real == 0 {
            Self::Structural
        } else if real >= total {
            Self::Full
        } else {
            Self::Partial
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_classification() {
        assert_eq!(
            classify_event("gen_ai.user.message"),
            GenAiEventClass::SupportedLegacy
        );
        assert_eq!(
            classify_event("gen_ai.developer.message"),
            GenAiEventClass::UnknownLegacy
        );
        assert_eq!(
            classify_event("gen_ai.client.inference.operation.details"),
            GenAiEventClass::InferenceDetails
        );
        assert_eq!(
            classify_event("gen_ai.evaluation.result"),
            GenAiEventClass::StructuralMetadata
        );
        assert_eq!(
            classify_event("gen_ai.some_future.event"),
            GenAiEventClass::StructuralMetadata
        );
        assert_eq!(classify_event("exception"), GenAiEventClass::Exception);
        assert_eq!(
            classify_event("gen_ai.client.operation.exception"),
            GenAiEventClass::Exception
        );
        assert_eq!(classify_event("http.request"), GenAiEventClass::NotGenAi);
    }

    #[test]
    fn operation_names() {
        for name in ["chat", "text_completion", "generate_content"] {
            assert_eq!(Operation::from_name(name), Some(Operation::ProviderCall));
        }
        assert_eq!(
            Operation::from_name("embeddings"),
            Some(Operation::Embeddings)
        );
        assert_eq!(
            Operation::from_name("execute_tool"),
            Some(Operation::ExecuteTool)
        );
        assert_eq!(
            Operation::from_name("invoke_agent"),
            Some(Operation::InvokeAgent)
        );
        assert_eq!(
            Operation::from_name("create_agent"),
            Some(Operation::CreateAgent)
        );
        assert_eq!(
            Operation::from_name("invoke_workflow"),
            Some(Operation::InvokeWorkflow)
        );
        assert_eq!(Operation::from_name("bogus"), None);
    }

    #[test]
    fn capture_from_slots() {
        assert_eq!(CaptureLevel::from_slots(0, 4), CaptureLevel::Structural);
        assert_eq!(CaptureLevel::from_slots(2, 4), CaptureLevel::Partial);
        assert_eq!(CaptureLevel::from_slots(4, 4), CaptureLevel::Full);
    }
}
