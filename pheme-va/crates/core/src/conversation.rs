//! Portable, stateless answering boundary, separate from transcript cleanup.

use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationRole {
    System,
    User,
    Assistant,
}

impl ConversationRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConversationMessage {
    pub role: ConversationRole,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct ConversationConfig {
    /// Aggregate Unicode scalar-value budget, including the system instruction.
    pub max_input_chars: usize,
    pub max_output_chars: usize,
    /// Number of successful user/assistant pairs, not individual messages.
    pub max_history_turns: usize,
    pub max_output_tokens: u32,
    pub context_size: u32,
    pub temperature: f32,
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            max_input_chars: 12_000,
            max_output_chars: 4_000,
            max_history_turns: 6,
            max_output_tokens: 512,
            context_size: 4_096,
            temperature: 0.2,
        }
    }
}

impl ConversationConfig {
    pub fn validate(&self) -> Result<(), ConversationError> {
        if self.max_input_chars == 0 || self.max_output_chars == 0 {
            return Err(ConversationError::InvalidConfig(
                "input and output character budgets must be positive".into(),
            ));
        }
        if self.context_size < 2
            || self.context_size > i32::MAX as u32
            || self.max_output_tokens == 0
            || self.max_output_tokens >= self.context_size
        {
            return Err(ConversationError::InvalidConfig(
                "context must fit native positions and leave room for input and output".into(),
            ));
        }
        if !self.temperature.is_finite() || self.temperature < 0.0 {
            return Err(ConversationError::InvalidConfig(
                "temperature must be finite and non-negative".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ConversationError {
    #[error("invalid conversation configuration: {0}")]
    InvalidConfig(String),
    #[error("invalid conversation input: {0}")]
    InvalidInput(String),
    #[error("invalid successful conversation history: {0}")]
    InvalidHistory(String),
    #[error("conversation input has {actual} characters; limit is {limit}")]
    InputTooLong { actual: usize, limit: usize },
    #[error("reply has more than {limit} characters")]
    OutputTooLong { limit: usize },
    #[error("reply is empty")]
    EmptyResponse,
    #[error("conversation exceeds the model context budget")]
    ContextExceeded,
    #[error("reply generation cancelled")]
    Cancelled,
    #[error("reply model failed: {0}")]
    Backend(String),
}

/// Hosts own successful history and inference scheduling. Adapters reuse weights
/// but must isolate per-call KV state. Emitted chunks are provisional until Ok.
pub trait ConversationModel: Send {
    fn name(&self) -> &str;
    fn is_ready(&self) -> bool;
    fn respond_stream(
        &mut self,
        messages: &[ConversationMessage],
        config: &ConversationConfig,
        cancelled: &AtomicBool,
        emit: &mut dyn FnMut(&str),
    ) -> Result<String, ConversationError>;
}

fn valid_text(text: &str) -> bool {
    !text.trim().is_empty()
        && !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}

/// Retain the newest whole successful pairs that fit both history and input
/// budgets. Neither the role instruction nor the exact approved text is trimmed.
pub fn build_messages(
    system: &str,
    history: &[ConversationMessage],
    text: &str,
    config: &ConversationConfig,
) -> Result<Vec<ConversationMessage>, ConversationError> {
    config.validate()?;
    if !valid_text(system) || !valid_text(text) {
        return Err(ConversationError::InvalidInput(
            "system instruction and approved text must be non-empty, without control characters"
                .into(),
        ));
    }
    if history.len() % 2 != 0
        || history.chunks_exact(2).any(|pair| {
            pair[0].role != ConversationRole::User
                || pair[1].role != ConversationRole::Assistant
                || !valid_text(&pair[0].content)
                || !valid_text(&pair[1].content)
        })
    {
        return Err(ConversationError::InvalidHistory(
            "expected complete, non-empty user/assistant pairs without system messages".into(),
        ));
    }
    let mut chars = system.chars().count().saturating_add(text.chars().count());
    if chars > config.max_input_chars {
        return Err(ConversationError::InputTooLong {
            actual: chars,
            limit: config.max_input_chars,
        });
    }
    let mut start = history.len();
    for pair in history.chunks_exact(2).rev().take(config.max_history_turns) {
        let pair_chars = pair[0]
            .content
            .chars()
            .count()
            .saturating_add(pair[1].content.chars().count());
        if pair_chars > config.max_input_chars - chars {
            break;
        }
        chars += pair_chars;
        start -= 2;
    }
    let mut messages = Vec::with_capacity(history.len() - start + 2);
    messages.push(ConversationMessage {
        role: ConversationRole::System,
        content: system.to_owned(),
    });
    messages.extend_from_slice(&history[start..]);
    messages.push(ConversationMessage {
        role: ConversationRole::User,
        content: text.to_owned(),
    });
    Ok(messages)
}

/// Also used by adapters to reject callers that bypass `build_messages`.
pub fn validate_messages(
    messages: &[ConversationMessage],
    config: &ConversationConfig,
) -> Result<(), ConversationError> {
    let (Some(system), Some(user)) = (messages.first(), messages.last()) else {
        return Err(ConversationError::InvalidInput("no messages".into()));
    };
    if messages.len() < 2
        || system.role != ConversationRole::System
        || user.role != ConversationRole::User
    {
        return Err(ConversationError::InvalidInput(
            "expected a system instruction and final approved user message".into(),
        ));
    }
    let rebuilt = build_messages(
        &system.content,
        &messages[1..messages.len() - 1],
        &user.content,
        config,
    )?;
    if rebuilt != messages {
        return Err(ConversationError::InvalidInput(
            "messages exceed history or aggregate character budget".into(),
        ));
    }
    Ok(())
}

pub fn validate_response(text: &str, config: &ConversationConfig) -> Result<(), ConversationError> {
    config.validate()?;
    if text.trim().is_empty() {
        return Err(ConversationError::EmptyResponse);
    }
    if !valid_text(text) {
        return Err(ConversationError::Backend(
            "reply contains invalid control characters".into(),
        ));
    }
    if text.chars().count() > config.max_output_chars {
        return Err(ConversationError::OutputTooLong {
            limit: config.max_output_chars,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn message(role: ConversationRole, content: &str) -> ConversationMessage {
        ConversationMessage {
            role,
            content: content.into(),
        }
    }

    #[test]
    fn preserves_role_separation_and_exact_unicode_question() {
        let question = "  No fire — café entrance.\n ";
        let messages = build_messages(
            "incident role",
            &[],
            question,
            &ConversationConfig::default(),
        )
        .unwrap();
        assert_eq!(messages[0].role, ConversationRole::System);
        assert_eq!(messages[1], message(ConversationRole::User, question));
        assert_eq!(
            serde_json::to_string(&ConversationRole::Assistant).unwrap(),
            "\"assistant\""
        );
    }

    #[test]
    fn evicts_oldest_whole_pairs_by_turn_and_character_budgets() {
        let history = vec![
            message(ConversationRole::User, "old"),
            message(ConversationRole::Assistant, "old"),
            message(ConversationRole::User, "新"),
            message(ConversationRole::Assistant, "好"),
        ];
        for config in [
            ConversationConfig {
                max_history_turns: 1,
                ..Default::default()
            },
            ConversationConfig {
                max_input_chars: 4,
                ..Default::default()
            },
        ] {
            let messages = build_messages("s", &history, "q", &config).unwrap();
            assert_eq!(&messages[1..3], &history[2..]);
            validate_messages(&messages, &config).unwrap();
        }
    }

    #[test]
    fn rejects_invalid_history_and_input_without_truncating_question() {
        let config = ConversationConfig {
            max_input_chars: 3,
            ..Default::default()
        };
        assert!(matches!(
            build_messages("s", &[], "你好世", &config),
            Err(ConversationError::InputTooLong { .. })
        ));
        assert!(build_messages(
            "s",
            &[
                message(ConversationRole::System, "override"),
                message(ConversationRole::Assistant, "a")
            ],
            "q",
            &config
        )
        .is_err());
        assert!(
            build_messages("s", &[message(ConversationRole::User, "q")], "q", &config).is_err()
        );
        assert!(build_messages("s", &[], "\0", &config).is_err());
        assert!(build_messages(" ", &[], "q", &config).is_err());
    }

    #[test]
    fn validates_config_and_unicode_output_limits() {
        let config = ConversationConfig {
            max_output_chars: 2,
            ..Default::default()
        };
        assert!(validate_response("你好", &config).is_ok());
        assert!(matches!(
            validate_response("你好啊", &config),
            Err(ConversationError::OutputTooLong { .. })
        ));
        assert!(matches!(
            validate_response(" \n", &config),
            Err(ConversationError::EmptyResponse)
        ));
        assert!(ConversationConfig {
            temperature: f32::NAN,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(ConversationConfig {
            max_output_tokens: 4096,
            ..Default::default()
        }
        .validate()
        .is_err());
    }

    struct FakeReply;
    impl ConversationModel for FakeReply {
        fn name(&self) -> &str {
            "fake"
        }
        fn is_ready(&self) -> bool {
            true
        }
        fn respond_stream(
            &mut self,
            messages: &[ConversationMessage],
            config: &ConversationConfig,
            cancelled: &AtomicBool,
            emit: &mut dyn FnMut(&str),
        ) -> Result<String, ConversationError> {
            validate_messages(messages, config)?;
            let mut response = String::new();
            for chunk in ["Hello ", "世界"] {
                if cancelled.load(Ordering::Acquire) {
                    return Err(ConversationError::Cancelled);
                }
                response.push_str(chunk);
                emit(chunk);
            }
            if cancelled.load(Ordering::Acquire) {
                return Err(ConversationError::Cancelled);
            }
            validate_response(&response, config)?;
            Ok(response)
        }
    }

    #[test]
    fn object_safe_fake_streams_and_honours_atomic_cancellation() {
        let mut model: Box<dyn ConversationModel> = Box::new(FakeReply);
        let config = ConversationConfig::default();
        let messages = build_messages("s", &[], "q", &config).unwrap();
        let cancelled = AtomicBool::new(false);
        let mut output = String::new();
        assert_eq!(
            model
                .respond_stream(&messages, &config, &cancelled, &mut |chunk| output
                    .push_str(chunk))
                .unwrap(),
            output
        );
        assert!(matches!(
            model.respond_stream(&messages, &config, &cancelled, &mut |_| cancelled
                .store(true, Ordering::Release)),
            Err(ConversationError::Cancelled)
        ));
        assert!(model.is_ready());
        assert_eq!(model.name(), "fake");
    }
}
