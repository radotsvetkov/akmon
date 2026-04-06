//! Token budget and message splitting for long-running agent sessions.

use akmon_models::{LlmProvider, Message, MessageRole, approximate_tokens};

/// Decides when chat history should be compacted and how to split messages for summarization.
pub struct ContextManager {
    /// Maximum tokens (context window cap) used with [`Self::threshold`] to trigger summarization.
    pub max_tokens: usize,
    /// Fraction of `max_tokens` that triggers summarization (e.g. `0.85`).
    pub threshold: f64,
    /// How many recent non-system messages to preserve after summarization.
    pub keep_recent: usize,
    /// Leading system rows that [`crate::build_messages`] / [`crate::build_followup_messages`] prepend (1 or 2).
    pub fixed_system_messages: usize,
}

impl Default for ContextManager {
    fn default() -> Self {
        Self {
            max_tokens: 8192,
            threshold: 0.85,
            keep_recent: 10,
            fixed_system_messages: 1,
        }
    }
}

impl ContextManager {
    /// Returns true when estimated tokens exceed `(max_tokens * threshold)` (strict `>`).
    pub fn needs_summarization(&self, messages: &[Message], provider: &dyn LlmProvider) -> bool {
        let tokens = provider
            .estimate_tokens(messages)
            .unwrap_or_else(|| approximate_tokens(messages));
        let limit = (self.max_tokens as f64 * self.threshold) as usize;
        tokens > limit
    }

    /// Splits the message list into `(to_summarize, to_keep)`.
    ///
    /// Skips the first [`Self::fixed_system_messages`] entries (project/system preamble), then any
    /// leading [`MessageRole::System`] rows in the body (e.g. prior `<<<SUMMARY>>>` blocks). The
    /// remaining tail is split so the last [`Self::keep_recent`] messages stay in `to_keep` and
    /// earlier messages go to `to_summarize`. Preamble and skipped body system rows are in neither
    /// slice; callers must retain them when rebuilding history.
    pub fn messages_to_summarize<'a>(
        &self,
        messages: &'a [Message],
    ) -> (&'a [Message], &'a [Message]) {
        let n_fixed = self.fixed_system_messages.min(messages.len());
        let body = &messages[n_fixed..];
        let mut j = 0usize;
        while j < body.len() && body[j].role == MessageRole::System {
            j += 1;
        }
        let rest = &body[j..];
        if rest.is_empty() {
            return (&messages[0..0], &messages[0..0]);
        }
        if rest.len() <= self.keep_recent {
            return (&messages[0..0], rest);
        }
        let k = rest.len() - self.keep_recent;
        (&rest[..k], &rest[k..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_models::MessageRole;
    use async_trait::async_trait;
    use futures::stream;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FixedTokenProvider {
        n: AtomicUsize,
        each: usize,
    }

    impl FixedTokenProvider {
        fn new(each: usize) -> Self {
            Self {
                n: AtomicUsize::new(0),
                each,
            }
        }
    }

    #[async_trait]
    impl LlmProvider for FixedTokenProvider {
        fn name(&self) -> &str {
            "fixed"
        }

        fn context_window_tokens(&self) -> usize {
            10_000
        }

        fn estimate_tokens(&self, messages: &[Message]) -> Option<usize> {
            let c = self.n.fetch_add(1, Ordering::SeqCst);
            Some(
                messages
                    .len()
                    .saturating_mul(self.each)
                    .saturating_add(c % 7),
            )
        }

        async fn complete(
            &self,
            _messages: &[Message],
            _config: &akmon_models::CompletionConfig,
        ) -> Result<akmon_models::CompletionStream, akmon_models::ModelError> {
            Ok(Box::pin(stream::empty()))
        }
    }

    #[test]
    fn default_values() {
        let d = ContextManager::default();
        assert_eq!(d.max_tokens, 8192);
        assert_eq!(d.threshold, 0.85);
        assert_eq!(d.keep_recent, 10);
        assert_eq!(d.fixed_system_messages, 1);
    }

    #[test]
    fn needs_summarization_false_under_threshold() {
        let cm = ContextManager {
            max_tokens: 1000,
            threshold: 0.85,
            keep_recent: 10,
            fixed_system_messages: 1,
        };
        let p = FixedTokenProvider::new(10);
        let msgs = vec![Message {
            role: MessageRole::User,
            content: "x".into(),
        }];
        assert!(!cm.needs_summarization(&msgs, &p));
    }

    #[test]
    fn needs_summarization_true_over_threshold() {
        let cm = ContextManager {
            max_tokens: 100,
            threshold: 0.85,
            keep_recent: 10,
            fixed_system_messages: 1,
        };
        let p = FixedTokenProvider::new(100);
        let msgs: Vec<Message> = (0..20)
            .map(|_| Message {
                role: MessageRole::User,
                content: "hi".into(),
            })
            .collect();
        assert!(cm.needs_summarization(&msgs, &p));
    }

    #[test]
    fn messages_to_summarize_excludes_fixed_system() {
        let cm = ContextManager {
            max_tokens: 100,
            threshold: 0.85,
            keep_recent: 2,
            fixed_system_messages: 1,
        };
        let msgs = vec![
            Message {
                role: MessageRole::System,
                content: "sys".into(),
            },
            Message {
                role: MessageRole::User,
                content: "a".into(),
            },
            Message {
                role: MessageRole::User,
                content: "b".into(),
            },
            Message {
                role: MessageRole::User,
                content: "c".into(),
            },
        ];
        let (s, k) = cm.messages_to_summarize(&msgs);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].content, "a");
        assert_eq!(k.len(), 2);
        assert_eq!(k[0].content, "b");
        assert_eq!(k[1].content, "c");
    }

    #[test]
    fn messages_to_summarize_skips_body_system_then_splits() {
        let cm = ContextManager {
            max_tokens: 100,
            threshold: 0.85,
            keep_recent: 1,
            fixed_system_messages: 1,
        };
        let msgs = vec![
            Message {
                role: MessageRole::System,
                content: "fixed".into(),
            },
            Message {
                role: MessageRole::System,
                content: "prior summary".into(),
            },
            Message {
                role: MessageRole::User,
                content: "old".into(),
            },
            Message {
                role: MessageRole::User,
                content: "new".into(),
            },
        ];
        let (s, k) = cm.messages_to_summarize(&msgs);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].content, "old");
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].content, "new");
    }

    #[test]
    fn messages_to_summarize_keep_recent_tail() {
        let cm = ContextManager {
            max_tokens: 100,
            threshold: 0.85,
            keep_recent: 3,
            fixed_system_messages: 0,
        };
        let msgs: Vec<Message> = (0..8)
            .map(|i| Message {
                role: MessageRole::Assistant,
                content: format!("m{i}"),
            })
            .collect();
        let (s, k) = cm.messages_to_summarize(&msgs);
        assert_eq!(s.len(), 5);
        assert_eq!(k.len(), 3);
        assert_eq!(k[2].content, "m7");
    }
}
