use std::f32::consts::TAU;
use std::path::PathBuf;

use va_core::{
    Engine, EngineError, NormalizedAudio, RawTranscription, Transcriber, TranscriptionOptions,
    TranscriptionStatus,
};

struct FixtureTranscriber {
    calls: usize,
}

impl Transcriber for FixtureTranscriber {
    fn name(&self) -> &str {
        "fixture-transcriber"
    }

    fn is_ready(&self) -> bool {
        true
    }

    fn transcribe(
        &mut self,
        audio: &NormalizedAudio,
        _options: &TranscriptionOptions,
    ) -> Result<RawTranscription, EngineError> {
        self.calls += 1;
        assert_eq!(audio.sample_rate, 16_000);
        assert_eq!(audio.original_channels, 2);
        assert_eq!(
            audio.gate.decision,
            va_core::SpeechGateDecision::SpeechDetected
        );
        Ok(RawTranscription::text(
            "  smoke reported near west entrance  ",
        ))
    }
}

#[test]
fn wav_audio_reaches_transcription_engine() {
    let path = fixture_path();
    write_stereo_wav(&path);
    let wav = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);

    let mut agent = Engine::new(FixtureTranscriber { calls: 0 });
    let result = agent.transcribe_wav(&wav).unwrap();

    assert_eq!(result.status, TranscriptionStatus::Speech);
    assert_eq!(result.raw_text, "smoke reported near west entrance");
    assert_eq!(result.text, "smoke reported near west entrance");
    assert_eq!(
        result.gate.decision,
        va_core::SpeechGateDecision::SpeechDetected
    );
}

fn fixture_path() -> PathBuf {
    std::env::temp_dir().join(format!("pheme-va-audio-fixture-{}.wav", std::process::id()))
}

fn write_stereo_wav(path: &std::path::Path) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for index in 0..(44_100 / 2) {
        let sample = (0.08 * (TAU * 440.0 * index as f32 / 44_100.0).sin() * 32_767.0) as i16;
        writer.write_sample(sample).unwrap();
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
}

#[cfg(feature = "whisper")]
#[test]
#[ignore = "requires PHEME_VA_WHISPER_MODEL and PHEME_VA_TEST_AUDIO"]
fn real_whisper_transcribes_a_supplied_audio_fixture() {
    use va_core::{EngineConfig, WhisperConfig, WhisperTranscriber};

    let model = std::env::var("PHEME_VA_WHISPER_MODEL").unwrap();
    let audio = std::env::var("PHEME_VA_TEST_AUDIO").unwrap();
    let language = std::env::var("PHEME_VA_TEST_LANGUAGE").ok();
    let transcriber = WhisperTranscriber::from_file(model, WhisperConfig::default()).unwrap();
    let config = EngineConfig {
        language,
        ..EngineConfig::default()
    };
    let mut agent = Engine::with_config(transcriber, config);
    let result = agent
        .transcribe_wav(&std::fs::read(audio).unwrap())
        .unwrap();

    assert_eq!(result.status, TranscriptionStatus::Speech);
    assert!(
        !result.text.trim().is_empty(),
        "Whisper returned no transcript"
    );
}
