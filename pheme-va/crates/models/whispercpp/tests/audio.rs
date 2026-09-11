use va_core::{Engine, EngineConfig, TranscriptionStatus};

use whispercpp::{WhisperConfig, WhisperTranscriber};

#[test]
#[ignore = "requires PHEME_VA_WHISPER_MODEL and PHEME_VA_TEST_AUDIO"]
fn real_whisper_transcribes_a_supplied_audio_fixture() {
    assert_transcribes(std::env::var("PHEME_VA_TEST_LANGUAGE").ok());
}

#[test]
#[ignore = "requires PHEME_VA_WHISPER_MODEL and PHEME_VA_TEST_AUDIO"]
fn real_whisper_auto_detects_language_and_transcribes() {
    // Deliberately ignore PHEME_VA_TEST_LANGUAGE to exercise the default TUI path.
    assert_transcribes(None);
}

fn assert_transcribes(language: Option<String>) {
    let model = std::env::var("PHEME_VA_WHISPER_MODEL").unwrap();
    let audio = std::env::var("PHEME_VA_TEST_AUDIO").unwrap();
    let transcriber = WhisperTranscriber::from_file(model, WhisperConfig::default()).unwrap();
    let config = EngineConfig {
        language,
        ..EngineConfig::default()
    };
    let mut engine = Engine::with_config(transcriber, config);
    let wav = std::fs::read(audio).unwrap();
    let result = engine.transcribe_wav(&wav).unwrap();

    assert_eq!(result.status, TranscriptionStatus::Speech);
    assert!(result.language.is_some(), "Whisper returned no language");
    assert!(!result.segments.is_empty(), "Whisper returned no segments");
    assert!(
        !result.text.trim().is_empty(),
        "Whisper returned no transcript"
    );
}
