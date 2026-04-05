//! Chat messages exchanged with providers.

use serde::{Deserialize, Serialize};

/// Who produced a [`Message`]'s [`Message::content`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    /// System instructions (fixed at session start; structurally isolated in Akmon).
    System,
    /// End-user input.
    User,
    /// Model output (natural language).
    Assistant,
    /// Tool result payload presented back to the model.
    Tool,
}

/// One turn in the chat history sent to [`crate::LlmProvider::complete`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Speaker role for this block.
    pub role: MessageRole,
    /// UTF-8 text body (tool JSON, markdown, plain user text, etc., depending on [`Message::role`]).
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn message_json_shape() {
        let m = Message {
            role: MessageRole::User,
            content: "hello".into(),
        };
        let v = serde_json::to_value(&m).expect("serialize");
        assert_eq!(v, json!({"role": "user", "content": "hello"}));
    }
}
