//! Portable speech-processing and incident-analysis primitives.
//!
//! The core deliberately has no microphone, UI, HTTP, or mobile lifecycle
//! dependencies. Hosts provide audio and choose concrete STT/LLM adapters.

mod audio;
pub mod chat;
pub mod conversation;
mod dictionary;
mod engine;
mod incident;
pub mod registry;
mod transcript;

pub use audio::{
    AudioBuffer, AudioError, NormalizedAudio, SpeechGateConfig, SpeechGateDecision,
    SpeechGateResult, TARGET_SAMPLE_RATE,
};
pub use conversation::{
    build_messages, validate_messages, validate_response, ConversationConfig, ConversationError,
    ConversationMessage, ConversationModel, ConversationRole,
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
pub use registry::{
    load_prompt, load_prompt_files, resolve_artifact, LoadedPrompt, ModelEntry, ModelManifest,
    ModelPurpose, StartupChoices, MAX_PROMPT_CHARS, MAX_PROMPT_FILES,
};
pub use transcript::{
    ModelTimings, RawTranscription, TranscriptGuardDecision, TranscriptSegment,
    TranscriptionOptions,
};
