use std::time::Instant;

use serde::Serialize;
use thiserror::Error;

use crate::audio::{
    AudioBuffer, AudioError, NormalizedAudio, SpeechGateConfig, SpeechGateDecision,
    SpeechGateResult,
};
use crate::dictionary::DictionaryHints;
use crate::transcript::{
    guard_transcript, normalize_transcript, RawTranscription, TranscriptGuardDecision,
    TranscriptionOptions,
};

pub trait Transcriber: Send {
    fn name(&self) -> &str;
    fn is_ready(&self) -> bool;
    fn transcribe(
        &mut self,
        audio: &NormalizedAudio,
        options: &TranscriptionOptions,
    ) -> Result<RawTranscription, EngineError>;
}

pub trait TextCleaner: Send {
    fn name(&self) -> &str;
    fn clean(&mut self, text: &str) -> Result<String, EngineError>;
}

/// Model-runtime boundary for optional cleanup or structured extraction.
/// Implementations may call llama.cpp, another local runtime, a cloud API, or
/// a deterministic test model; the audio pipeline does not depend on any one
/// provider.
pub trait LanguageModel: Send {
    fn name(&self) -> &str;
    fn complete(&mut self, prompt: &str) -> Result<String, EngineError>;
}

pub struct LlmTextCleaner<M> {
    model: M,
    instruction: String,
}

impl<M> LlmTextCleaner<M>
where
    M: LanguageModel,
{
    pub fn new(model: M) -> Self {
        Self {
            model,
            instruction: "Rewrite the transcript as clear written text. Preserve every fact, name, number, negation, uncertainty, and requested action. Do not add information. Return only the rewritten text.".to_owned(),
        }
    }

    pub fn with_instruction(mut self, instruction: impl Into<String>) -> Self {
        self.instruction = instruction.into();
        self
    }
}

impl<M> TextCleaner for LlmTextCleaner<M>
where
    M: LanguageModel,
{
    fn name(&self) -> &str {
        self.model.name()
    }

    fn clean(&mut self, text: &str) -> Result<String, EngineError> {
        let prompt = format!(
            "{}\n\n<transcript>\n{}\n</transcript>",
            self.instruction, text
        );
        self.model
            .complete(&prompt)
            .map_err(|error| EngineError::Cleanup {
                message: error.to_string(),
            })
    }
}

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub max_audio_seconds: Option<u32>,
    pub dictionary: DictionaryHints,
    pub gate: SpeechGateConfig,
    pub decoder: DecoderConfig,
    pub language: Option<String>,
    pub cleanup_enabled: bool,
    /// Retry one dictionary-prompt echo without the prompt before discarding it.
    pub retry_without_dictionary_prompt: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_audio_seconds: Some(120),
            dictionary: DictionaryHints::new(),
            gate: SpeechGateConfig::default(),
            decoder: DecoderConfig::default(),
            language: None,
            cleanup_enabled: true,
            retry_without_dictionary_prompt: true,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DecoderConfig {
    pub threads: i32,
    pub entropy_threshold: f32,
    pub logprob_threshold: f32,
    pub no_speech_threshold: f32,
    pub temperature: f32,
    pub suppress_blank: bool,
    pub suppress_non_speech_tokens: bool,
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self {
            threads: 4,
            entropy_threshold: 2.8,
            logprob_threshold: -1.25,
            no_speech_threshold: 0.6,
            temperature: 0.0,
            suppress_blank: true,
            suppress_non_speech_tokens: true,
        }
    }
}

impl From<DecoderConfig> for crate::transcript::DecoderOptions {
    fn from(value: DecoderConfig) -> Self {
        Self {
            threads: value.threads,
            entropy_threshold: value.entropy_threshold,
            logprob_threshold: value.logprob_threshold,
            no_speech_threshold: value.no_speech_threshold,
            temperature: value.temperature,
            suppress_blank: value.suppress_blank,
            suppress_non_speech_tokens: value.suppress_non_speech_tokens,
        }
    }
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Audio(#[from] AudioError),
    #[error("speech recognizer `{backend}` is not ready")]
    BackendUnavailable { backend: String },
    #[error("speech recognizer failed: {message}")]
    Backend { message: String },
    #[error("cleanup failed: {message}")]
    Cleanup { message: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum TranscriptionStatus {
    Speech,
    NoSpeech,
    InsufficientSpeech,
    DictionaryPromptEcho,
    KnownSilenceMarker,
    EmptyTranscript,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CleanupStatus {
    Disabled,
    NotConfigured,
    Applied,
    FallbackToRaw,
}

#[derive(Clone, Debug, Serialize)]
pub struct TranscriptionResult {
    pub status: TranscriptionStatus,
    pub raw_text: String,
    pub text: String,
    pub language: Option<String>,
    pub segments: Vec<crate::transcript::TranscriptSegment>,
    pub gate: SpeechGateResult,
    pub dictionary_prompt: Option<String>,
    pub stt_backend: String,
    pub cleanup_backend: Option<String>,
    pub cleanup_status: CleanupStatus,
    pub recovery_attempted: bool,
    pub processing_time_ms: u128,
}

pub struct Engine {
    transcriber: Box<dyn Transcriber>,
    cleaner: Option<Box<dyn TextCleaner>>,
    config: EngineConfig,
}

impl Engine {
    pub fn new<T>(transcriber: T) -> Self
    where
        T: Transcriber + 'static,
    {
        Self {
            transcriber: Box::new(transcriber),
            cleaner: None,
            config: EngineConfig::default(),
        }
    }

    pub fn with_config<T>(transcriber: T, config: EngineConfig) -> Self
    where
        T: Transcriber + 'static,
    {
        Self {
            transcriber: Box::new(transcriber),
            cleaner: None,
            config,
        }
    }

    pub fn with_cleaner<C>(mut self, cleaner: C) -> Self
    where
        C: TextCleaner + 'static,
    {
        self.cleaner = Some(Box::new(cleaner));
        self
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut EngineConfig {
        &mut self.config
    }

    pub fn is_ready(&self) -> bool {
        self.transcriber.is_ready()
    }

    pub fn backend_name(&self) -> &str {
        self.transcriber.name()
    }

    pub fn transcribe(&mut self, audio: AudioBuffer) -> Result<TranscriptionResult, EngineError> {
        let started = Instant::now();
        let normalized = audio.normalize(self.config.max_audio_seconds)?;
        self.transcribe_normalized(normalized, started)
    }

    pub fn transcribe_wav(&mut self, wav: &[u8]) -> Result<TranscriptionResult, EngineError> {
        self.transcribe(AudioBuffer::from_wav(wav)?)
    }

    fn transcribe_normalized(
        &mut self,
        mut audio: NormalizedAudio,
        started: Instant,
    ) -> Result<TranscriptionResult, EngineError> {
        // The configured gate is deliberately re-run here so callers can tune
        // thresholds without changing the shared audio normalizer.
        audio.gate = self.config.gate.analyze(&audio.samples, audio.sample_rate);
        let gate = audio.gate.clone();
        let prompt = self.config.dictionary.prompt();
        let prompt_text = prompt.as_ref().map(|prompt| prompt.text.clone());
        let backend_name = self.transcriber.name().to_owned();

        if gate.decision.should_skip_transcription() {
            let status = match gate.decision {
                SpeechGateDecision::Silence | SpeechGateDecision::Unavailable => {
                    TranscriptionStatus::NoSpeech
                }
                SpeechGateDecision::InsufficientSpeech => TranscriptionStatus::InsufficientSpeech,
                SpeechGateDecision::SpeechDetected => unreachable!(),
            };
            return Ok(TranscriptionResult {
                status,
                raw_text: String::new(),
                text: String::new(),
                language: self.config.language.clone(),
                segments: Vec::new(),
                gate,
                dictionary_prompt: prompt_text,
                stt_backend: backend_name,
                cleanup_backend: self
                    .cleaner
                    .as_ref()
                    .map(|cleaner| cleaner.name().to_owned()),
                cleanup_status: if self.config.cleanup_enabled {
                    CleanupStatus::NotConfigured
                } else {
                    CleanupStatus::Disabled
                },
                recovery_attempted: false,
                processing_time_ms: started.elapsed().as_millis(),
            });
        }

        if !self.transcriber.is_ready() {
            return Err(EngineError::BackendUnavailable {
                backend: backend_name,
            });
        }

        let options = TranscriptionOptions {
            language: self.config.language.clone(),
            dictionary_prompt: prompt,
            decoder: self.config.decoder.clone().into(),
        };
        let mut raw = self.transcriber.transcribe(&audio, &options)?;
        let mut raw_text = normalize_transcript(&raw.text);
        let mut guard = guard_transcript(&raw_text, options.dictionary_prompt.as_ref());
        let mut recovery_attempted = false;
        if guard == TranscriptGuardDecision::DictionaryPromptEcho
            && self.config.retry_without_dictionary_prompt
        {
            recovery_attempted = true;
            let mut retry_options = options.clone();
            retry_options.dictionary_prompt = None;
            // A recovery failure preserves the original guarded result rather
            // than turning a successful transcription into a transport error.
            if let Ok(retry_raw) = self.transcriber.transcribe(&audio, &retry_options) {
                let retry_text = normalize_transcript(&retry_raw.text);
                let retry_guard = guard_transcript(&retry_text, None);
                if retry_guard != TranscriptGuardDecision::DictionaryPromptEcho {
                    raw = retry_raw;
                    raw_text = retry_text;
                    guard = retry_guard;
                }
            }
        }
        let (status, text, cleanup_status) = match guard {
            TranscriptGuardDecision::Accept => {
                let (text, cleanup_status) = self.clean_text(&raw_text);
                (TranscriptionStatus::Speech, text, cleanup_status)
            }
            TranscriptGuardDecision::Empty => (
                TranscriptionStatus::EmptyTranscript,
                String::new(),
                CleanupStatus::NotConfigured,
            ),
            TranscriptGuardDecision::KnownSilenceMarker => (
                TranscriptionStatus::KnownSilenceMarker,
                String::new(),
                CleanupStatus::NotConfigured,
            ),
            TranscriptGuardDecision::DictionaryPromptEcho => (
                TranscriptionStatus::DictionaryPromptEcho,
                String::new(),
                CleanupStatus::NotConfigured,
            ),
        };

        Ok(TranscriptionResult {
            status,
            raw_text,
            text,
            language: raw.language.or(self.config.language.clone()),
            segments: raw.segments,
            gate,
            dictionary_prompt: prompt_text,
            stt_backend: backend_name,
            cleanup_backend: self
                .cleaner
                .as_ref()
                .map(|cleaner| cleaner.name().to_owned()),
            cleanup_status,
            recovery_attempted,
            processing_time_ms: started.elapsed().as_millis(),
        })
    }

    fn clean_text(&mut self, raw_text: &str) -> (String, CleanupStatus) {
        if !self.config.cleanup_enabled {
            return (raw_text.to_owned(), CleanupStatus::Disabled);
        }
        let Some(cleaner) = self.cleaner.as_mut() else {
            return (raw_text.to_owned(), CleanupStatus::NotConfigured);
        };
        match cleaner.clean(raw_text) {
            Ok(cleaned) if !normalize_transcript(&cleaned).is_empty() => {
                (normalize_transcript(&cleaned), CleanupStatus::Applied)
            }
            Ok(_) | Err(_) => (raw_text.to_owned(), CleanupStatus::FallbackToRaw),
        }
    }
}

/// A safe formatting pass that only normalizes whitespace and sentence casing.
/// It is intentionally not an LLM and does not infer or remove facts.
#[derive(Clone, Debug, Default)]
pub struct RuleBasedFormatter;

impl TextCleaner for RuleBasedFormatter {
    fn name(&self) -> &str {
        "rule-based-formatter"
    }

    fn clean(&mut self, text: &str) -> Result<String, EngineError> {
        let text = crate::transcript::normalize_transcript(text);
        let mut chars = text.chars();
        let Some(first) = chars.next() else {
            return Ok(String::new());
        };
        let mut result = first.to_uppercase().collect::<String>();
        result.push_str(chars.as_str());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioBuffer;
    use crate::transcript::RawTranscription;

    struct FakeTranscriber {
        text: String,
    }

    impl Transcriber for FakeTranscriber {
        fn name(&self) -> &str {
            "fake"
        }

        fn is_ready(&self) -> bool {
            true
        }

        fn transcribe(
            &mut self,
            _audio: &NormalizedAudio,
            _options: &TranscriptionOptions,
        ) -> Result<RawTranscription, EngineError> {
            Ok(RawTranscription::text(self.text.clone()))
        }
    }

    struct EchoRecoveringTranscriber;

    impl Transcriber for EchoRecoveringTranscriber {
        fn name(&self) -> &str {
            "echo-recovering-fake"
        }

        fn is_ready(&self) -> bool {
            true
        }

        fn transcribe(
            &mut self,
            _audio: &NormalizedAudio,
            options: &TranscriptionOptions,
        ) -> Result<RawTranscription, EngineError> {
            if options.dictionary_prompt.is_some() {
                Ok(RawTranscription::text("Electron, renderer, west entrance"))
            } else {
                Ok(RawTranscription::text("Electron renderer"))
            }
        }
    }

    struct FakeLanguageModel;

    impl LanguageModel for FakeLanguageModel {
        fn name(&self) -> &str {
            "fake-llm"
        }

        fn complete(&mut self, prompt: &str) -> Result<String, EngineError> {
            assert!(prompt.contains("<transcript>"));
            Ok("A cleaned transcript.".to_owned())
        }
    }

    fn speech_audio() -> AudioBuffer {
        AudioBuffer::new(
            TARGET_SAMPLE_RATE,
            1,
            vec![0.05; TARGET_SAMPLE_RATE as usize],
        )
        .unwrap()
    }

    use crate::TARGET_SAMPLE_RATE;

    #[test]
    fn runs_audio_through_gate_stt_and_cleanup() {
        let mut agent = Engine::new(FakeTranscriber {
            text: "  collision   at west gate  ".to_owned(),
        })
        .with_cleaner(RuleBasedFormatter);
        let result = agent.transcribe(speech_audio()).unwrap();
        assert_eq!(result.status, TranscriptionStatus::Speech);
        assert_eq!(result.raw_text, "collision at west gate");
        assert_eq!(result.text, "Collision at west gate");
        assert_eq!(result.cleanup_status, CleanupStatus::Applied);
    }

    #[test]
    fn retries_a_dictionary_echo_without_the_prompt() {
        let config = EngineConfig {
            dictionary: DictionaryHints::with_terms(["Electron", "renderer", "west entrance"]),
            ..EngineConfig::default()
        };
        let mut agent = Engine::with_config(EchoRecoveringTranscriber, config);
        let result = agent.transcribe(speech_audio()).unwrap();
        assert_eq!(result.status, TranscriptionStatus::Speech);
        assert_eq!(result.text, "Electron renderer");
        assert!(result.recovery_attempted);
    }

    #[test]
    fn routes_cleanup_through_the_swappable_language_model() {
        let mut agent = Engine::new(FakeTranscriber {
            text: "messy transcript".to_owned(),
        })
        .with_cleaner(LlmTextCleaner::new(FakeLanguageModel));
        let result = agent.transcribe(speech_audio()).unwrap();
        assert_eq!(result.cleanup_backend.as_deref(), Some("fake-llm"));
        assert_eq!(result.text, "A cleaned transcript.");
        assert_eq!(result.cleanup_status, CleanupStatus::Applied);
    }

    #[test]
    fn never_calls_stt_for_silence() {
        let mut agent = Engine::new(FakeTranscriber {
            text: "Thank you for watching".to_owned(),
        });
        let audio = AudioBuffer::new(
            TARGET_SAMPLE_RATE,
            1,
            vec![0.0; TARGET_SAMPLE_RATE as usize],
        )
        .unwrap();
        let result = agent.transcribe(audio).unwrap();
        assert_eq!(result.status, TranscriptionStatus::NoSpeech);
        assert!(result.text.is_empty());
    }
}
