//! Portable speech-processing and incident-analysis primitives.
//!
//! The core deliberately has no microphone, UI, HTTP, or mobile lifecycle
//! dependencies. Hosts provide audio and choose concrete STT/LLM adapters.

mod audio;
mod dictionary;
mod engine;
mod incident;
mod transcript;

#[cfg(feature = "whisper")]
mod whisper;

pub use audio::{
    AudioBuffer, AudioError, NormalizedAudio, SpeechGateConfig, SpeechGateDecision,
    SpeechGateResult, TARGET_SAMPLE_RATE,
};
pub use dictionary::{DictionaryHints, DictionaryPrompt};
pub use engine::{
    CleanupStatus, DecoderConfig, Engine, EngineConfig, EngineError, LanguageModel, LlmTextCleaner,
    RuleBasedFormatter, TextCleaner, Transcriber, TranscriptionResult, TranscriptionStatus,
};
pub use incident::{
    analyze_with_metrics, IncidentAnalyzer, IncidentReport, RuleBasedIncidentAnalyzer,
};
pub use metrics::{MetricsConfig, MetricsContext, MetricsHub, MetricsSubscriber, Stage};
pub use transcript::{
    RawTranscription, TranscriptGuardDecision, TranscriptSegment, TranscriptionOptions,
};

#[cfg(feature = "whisper")]
pub use whisper::{WhisperConfig, WhisperTranscriber};
