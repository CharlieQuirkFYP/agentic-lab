use va_core::{Engine, EngineConfig, TranscriptionStatus};

use whispercpp::{WhisperConfig, WhisperTranscriber};

#[test]
#[ignore = "requires PHEME_VA_WHISPER_MODEL and PHEME_VA_TEST_AUDIO"]
fn real_whisper_transcribes_a_supplied_audio_fixture() {
    let model = std::env::var("PHEME_VA_WHISPER_MODEL").unwrap();
    let audio = std::env::var("PHEME_VA_TEST_AUDIO").unwrap();
    let language = std::env::var("PHEME_VA_TEST_LANGUAGE").ok();
    let transcriber = WhisperTranscriber::from_file(model, WhisperConfig::default()).unwrap();
    let config = EngineConfig {
        language,
        ..EngineConfig::default()
    };
    let mut engine = Engine::with_config(transcriber, config);
    let wav = std::fs::read(audio).unwrap();
    let result = engine.transcribe_wav(&wav).unwrap();

    assert_eq!(result.status, TranscriptionStatus::Speech);
    assert!(
        !result.text.trim().is_empty(),
        "Whisper returned no transcript"
    );
}
