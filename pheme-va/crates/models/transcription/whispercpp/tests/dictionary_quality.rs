//! Opt-in dictionary evaluation on one caller-supplied WAV and reference text.
//! Results go to stdout; the test never writes audio or generated artifacts.

use std::time::Instant;

use va_core::{DictionaryHints, Engine, EngineConfig, TranscriptionResult, TranscriptionStatus};
use whispercpp::{WhisperConfig, WhisperTranscriber};

#[test]
#[ignore = "requires PHEME_VA_WHISPER_MODEL, PHEME_VA_TEST_WAV, PHEME_VA_TEST_REFERENCE and PHEME_VA_TEST_DICTIONARY"]
fn compare_plain_and_dictionary_hinted_whisper_on_the_same_wav() {
    let model_path = std::env::var_os("PHEME_VA_WHISPER_MODEL")
        .expect("set PHEME_VA_WHISPER_MODEL to local Whisper weights");
    let wav_path = std::env::var_os("PHEME_VA_TEST_WAV")
        .expect("set PHEME_VA_TEST_WAV to a local speech recording");
    let reference = std::env::var("PHEME_VA_TEST_REFERENCE")
        .expect("set PHEME_VA_TEST_REFERENCE to the verbatim reference transcript");
    let terms = std::env::var("PHEME_VA_TEST_DICTIONARY")
        .expect("set PHEME_VA_TEST_DICTIONARY to comma-separated terms")
        .split(',')
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let reference_words = words(&reference);
    assert!(
        !reference_words.is_empty(),
        "reference transcript must contain words"
    );
    assert!(
        !terms.is_empty(),
        "dictionary must contain at least one term"
    );
    let dictionary = DictionaryHints::with_terms(&terms);
    let prompt = dictionary.prompt().expect("dictionary has a usable prompt");
    assert!(
        terms
            .iter()
            .all(|term| contains_words(&words(&prompt.text), &words(term))),
        "supply a dictionary that fits the prompt budget"
    );
    let wav = std::fs::read(&wav_path).expect("read the supplied WAV");
    let transcriber = WhisperTranscriber::from_file(&model_path, WhisperConfig::default())
        .expect("load local Whisper model");
    let mut engine = Engine::with_config(
        transcriber,
        EngineConfig {
            language: std::env::var("PHEME_VA_TEST_LANGUAGE").ok(),
            cleanup_enabled: false,
            ..EngineConfig::default()
        },
    );
    println!(
        "dictionary_evaluation model={model_path:?} wav={wav_path:?} reference={reference:?} terms={terms:?} language={:?} decoder={:?} order=plain_then_hinted load_excluded=true",
        engine.config().language,
        engine.config().decoder
    );

    let mut scores = Vec::new();
    for label in ["plain", "hinted"] {
        if label == "hinted" {
            engine.config_mut().dictionary = dictionary.clone();
        }
        let started = Instant::now();
        let result = engine
            .transcribe_wav(&wav)
            .expect("transcribe the supplied WAV");
        let elapsed_ms = started.elapsed().as_millis();
        let errors = word_errors(&reference_words, &words(&result.text));
        let wer = errors as f64 / reference_words.len() as f64;
        print_result(
            label,
            &result,
            elapsed_ms,
            errors,
            reference_words.len(),
            wer,
        );
        for term in &terms {
            let term_words = words(term);
            assert!(
                !term_words.is_empty(),
                "dictionary terms must contain words"
            );
            let expected = contains_words(&reference_words, &term_words);
            let observed = contains_words(&words(&result.text), &term_words);
            println!(
                "mode={label} term={term:?} present_in_reference={expected} present_in_transcript={observed} correct={}",
                expected == observed
            );
        }
        assert_eq!(
            result.status,
            TranscriptionStatus::Speech,
            "fixture must produce speech"
        );
        assert!(!result.text.trim().is_empty());
        if label == "plain" {
            assert!(result.dictionary_prompt.is_none());
        } else {
            assert_eq!(
                result.dictionary_prompt.as_deref(),
                Some(prompt.text.as_str())
            );
        }
        scores.push(wer);
    }
    // Hints can help proper nouns, be neutral, or introduce errors. Report the
    // observed difference without encoding a universal improvement claim.
    println!(
        "dictionary_summary plain_wer={:.4} hinted_wer={:.4} hinted_minus_plain={:.4}",
        scores[0],
        scores[1],
        scores[1] - scores[0]
    );
}

fn print_result(
    label: &str,
    result: &TranscriptionResult,
    elapsed_ms: u128,
    errors: usize,
    reference_words: usize,
    wer: f64,
) {
    println!(
        "mode={label} status={:?} elapsed_ms={elapsed_ms} processing_ms={} word_errors={errors} reference_words={reference_words} wer={wer:.4} recovery_attempted={} dictionary_prompt={:?} raw={:?} transcript={:?}",
        result.status,
        result.processing_time_ms,
        result.recovery_attempted,
        result.dictionary_prompt,
        result.raw_text,
        result.text
    );
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

fn contains_words(text: &[String], term: &[String]) -> bool {
    !term.is_empty() && text.windows(term.len()).any(|window| window == term)
}

/// Standard word-level edit distance: substitutions, deletions, insertions.
fn word_errors(reference: &[String], hypothesis: &[String]) -> usize {
    let mut previous = (0..=hypothesis.len()).collect::<Vec<_>>();
    for (reference_index, expected) in reference.iter().enumerate() {
        let mut current = vec![reference_index + 1; hypothesis.len() + 1];
        for (hypothesis_index, observed) in hypothesis.iter().enumerate() {
            current[hypothesis_index + 1] = (previous[hypothesis_index + 1] + 1)
                .min(current[hypothesis_index] + 1)
                .min(previous[hypothesis_index] + usize::from(expected != observed));
        }
        previous = current;
    }
    previous[hypothesis.len()]
}
