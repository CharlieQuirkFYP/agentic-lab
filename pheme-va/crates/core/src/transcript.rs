use serde::Serialize;

use crate::dictionary::DictionaryPrompt;

#[derive(Clone, Debug, Default, Serialize)]
pub struct TranscriptionOptions {
    pub language: Option<String>,
    pub dictionary_prompt: Option<DictionaryPrompt>,
    pub decoder: DecoderOptions,
}

#[derive(Clone, Debug, Serialize)]
pub struct DecoderOptions {
    pub threads: i32,
    pub entropy_threshold: f32,
    pub logprob_threshold: f32,
    pub no_speech_threshold: f32,
    pub temperature: f32,
    pub suppress_blank: bool,
    pub suppress_non_speech_tokens: bool,
}

impl Default for DecoderOptions {
    fn default() -> Self {
        Self {
            threads: 4,
            // These starting values mirror the guarded local Whisper settings
            // used by OpenWhispr. They need device/dataset evaluation.
            entropy_threshold: 2.8,
            logprob_threshold: -1.25,
            no_speech_threshold: 0.6,
            temperature: 0.0,
            suppress_blank: true,
            suppress_non_speech_tokens: true,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RawTranscription {
    pub text: String,
    pub language: Option<String>,
    pub segments: Vec<TranscriptSegment>,
    pub no_speech_probability: Option<f32>,
}

impl RawTranscription {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TranscriptSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub no_speech_probability: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum TranscriptGuardDecision {
    Accept,
    Empty,
    KnownSilenceMarker,
    DictionaryPromptEcho,
}

impl TranscriptGuardDecision {
    pub fn should_discard(self) -> bool {
        !matches!(self, Self::Accept)
    }
}

pub fn normalize_transcript(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Conservative post-STT protection against known blank markers and prompt
/// continuation. It deliberately does not discard ordinary words that happen
/// to occur in the dictionary.
pub fn guard_transcript(text: &str, prompt: Option<&DictionaryPrompt>) -> TranscriptGuardDecision {
    let normalized = normalize_transcript(text);
    if normalized.is_empty() {
        return TranscriptGuardDecision::Empty;
    }

    let marker = normalized.trim().to_ascii_lowercase();
    if matches!(
        marker.as_str(),
        "[blank_audio]" | "blank_audio" | "[silence]" | "[inaudible]" | "(silence)" | "silence"
    ) {
        return TranscriptGuardDecision::KnownSilenceMarker;
    }

    let Some(prompt) = prompt else {
        return TranscriptGuardDecision::Accept;
    };
    if is_prompt_echo(&normalized, &prompt.text) {
        TranscriptGuardDecision::DictionaryPromptEcho
    } else {
        TranscriptGuardDecision::Accept
    }
}

fn is_prompt_echo(text: &str, prompt: &str) -> bool {
    let text_words = words(text);
    let prompt_words = words(prompt);
    if text_words.is_empty() || prompt_words.is_empty() {
        return false;
    }
    if text_words == prompt_words {
        return true;
    }

    let unique_prompt: std::collections::HashSet<&str> = prompt_words.iter().copied().collect();
    let unique_text: std::collections::HashSet<&str> = text_words.iter().copied().collect();
    let overlap = unique_text
        .iter()
        .filter(|word| unique_prompt.contains(**word))
        .count();
    let composition = overlap as f32 / unique_text.len() as f32;
    let repeated_word = text_words.iter().any(|word| {
        text_words
            .iter()
            .filter(|candidate| *candidate == word)
            .count()
            >= 3
    });
    let prompt_sequence = prompt_words
        .windows(3)
        .any(|window| text_words.windows(3).any(|candidate| candidate == window));

    composition >= 0.9 && (repeated_word || prompt_sequence || text.ends_with(','))
}

fn words(text: &str) -> Vec<&str> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.trim())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_whitespace_without_rewriting_words() {
        assert_eq!(
            normalize_transcript("  East\n gate   is closed. "),
            "East gate is closed."
        );
    }

    #[test]
    fn rejects_known_silence_markers() {
        assert_eq!(
            guard_transcript(" [BLANK_AUDIO] ", None),
            TranscriptGuardDecision::KnownSilenceMarker
        );
    }

    #[test]
    fn rejects_a_prompt_continuation_but_keeps_legitimate_dictionary_speech() {
        let prompt = DictionaryPrompt {
            text: "Electron, renderer, west entrance".to_owned(),
        };
        assert_eq!(
            guard_transcript("Electron, renderer, west entrance", Some(&prompt)),
            TranscriptGuardDecision::DictionaryPromptEcho
        );
        assert_eq!(
            guard_transcript("Electron renderer", Some(&prompt)),
            TranscriptGuardDecision::Accept
        );
    }
}
