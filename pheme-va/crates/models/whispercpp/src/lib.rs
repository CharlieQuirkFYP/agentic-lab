use std::path::Path;

use std::time::Instant;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use va_core::{
    EngineError, ModelTimings, NormalizedAudio, RawTranscription, Transcriber, TranscriptSegment,
    TranscriptionOptions,
};

#[derive(Clone, Debug)]
pub struct WhisperConfig {
    pub threads: i32,
    pub use_gpu: bool,
    pub flash_attention: bool,
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            threads: 4,
            use_gpu: cfg!(feature = "metal"),
            flash_attention: false,
        }
    }
}

/// Prewarmed in-process whisper.cpp adapter. The context is loaded once and a
/// decoding state is created for each request, which keeps model startup out of
/// the recording-to-transcript path.
pub struct WhisperTranscriber {
    context: WhisperContext,
    model_name: String,
    model_id: String,
    model_revision: Option<String>,
    config: WhisperConfig,
}

impl WhisperTranscriber {
    pub fn from_file(path: impl AsRef<Path>, config: WhisperConfig) -> Result<Self, EngineError> {
        let path = path.as_ref();
        Self::from_file_with_metadata(path, config, path.display().to_string(), None)
    }

    pub fn from_file_with_metadata(
        path: impl AsRef<Path>,
        config: WhisperConfig,
        model_id: impl Into<String>,
        model_revision: Option<String>,
    ) -> Result<Self, EngineError> {
        let path = path.as_ref();
        let context = load_context(path, &config).map_err(|error| EngineError::Backend {
            message: format!("could not load Whisper model: {error}"),
        })?;
        Ok(Self {
            context,
            model_name: path.display().to_string(),
            model_id: model_id.into(),
            model_revision,
            config,
        })
    }

    pub fn model_name(&self) -> &str {
        &self.model_name
    }
}

fn load_context(path: &Path, config: &WhisperConfig) -> Result<WhisperContext, String> {
    let mut parameters = WhisperContextParameters::default();
    parameters.use_gpu(config.use_gpu);
    parameters.flash_attn(config.flash_attention);
    match WhisperContext::new_with_params(path, parameters) {
        Ok(context) => Ok(context),
        Err(gpu_error) if config.use_gpu => {
            // GPU/Metal availability is a deployment detail. Fall back to CPU
            // at model-load time instead of making the application unusable.
            let mut cpu_parameters = WhisperContextParameters::default();
            cpu_parameters.use_gpu(false);
            cpu_parameters.flash_attn(false);
            WhisperContext::new_with_params(path, cpu_parameters).map_err(|cpu_error| {
                format!("GPU load failed ({gpu_error}); CPU fallback failed ({cpu_error})")
            })
        }
        Err(error) => Err(error.to_string()),
    }
}

impl Transcriber for WhisperTranscriber {
    fn name(&self) -> &str {
        &self.model_id
    }

    fn model_family(&self) -> &str {
        "whisper"
    }

    fn model_revision(&self) -> Option<&str> {
        self.model_revision.as_deref()
    }

    fn is_ready(&self) -> bool {
        true
    }

    fn transcribe(
        &mut self,
        audio: &NormalizedAudio,
        options: &TranscriptionOptions,
    ) -> Result<RawTranscription, EngineError> {
        let started = Instant::now();
        let mut state = self
            .context
            .create_state()
            .map_err(|error| EngineError::Backend {
                message: format!("could not create Whisper state: {error}"),
            })?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 5 });
        params.set_n_threads(
            options
                .decoder
                .threads
                .max(1)
                .min(self.config.threads.max(1)),
        );
        params.set_no_context(true);
        params.set_single_segment(false);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(options.decoder.suppress_blank);
        params.set_suppress_nst(options.decoder.suppress_non_speech_tokens);
        params.set_temperature(options.decoder.temperature);
        params.set_entropy_thold(options.decoder.entropy_threshold);
        params.set_logprob_thold(options.decoder.logprob_threshold);
        params.set_no_speech_thold(options.decoder.no_speech_threshold);
        params.set_split_on_word(true);
        // A missing language enables automatic detection during transcription.
        // detect_language=true instead returns after detection, without decoding text.
        params.set_language(options.language.as_deref());
        params.set_detect_language(false);
        if let Some(prompt) = options.dictionary_prompt.as_ref() {
            params.set_initial_prompt(&prompt.text);
        }

        state
            .full(params, &audio.samples)
            .map_err(|error| EngineError::Backend {
                message: format!("Whisper decoding failed: {error}"),
            })?;

        let mut text = String::new();
        let mut segments = Vec::new();
        let segment_count = state.full_n_segments();
        let mut no_speech_probability = None;
        for index in 0..segment_count {
            let Some(segment) = state.get_segment(index) else {
                continue;
            };
            let segment_text = segment.to_str().map_err(|error| EngineError::Backend {
                message: format!("Whisper returned invalid segment text: {error}"),
            })?;
            text.push_str(segment_text);
            let no_speech = segment.no_speech_probability();
            no_speech_probability = Some(no_speech_probability.unwrap_or(0.0_f32).max(no_speech));
            segments.push(TranscriptSegment {
                start_ms: segment.start_timestamp() * 10,
                end_ms: segment.end_timestamp() * 10,
                text: segment_text.to_owned(),
                no_speech_probability: Some(no_speech),
            });
        }

        Ok(RawTranscription {
            text,
            language: whisper_rs::get_lang_str(state.full_lang_id_from_state()).map(str::to_owned),
            segments,
            no_speech_probability,
            timings: ModelTimings {
                feature_extraction_ms: None,
                inference_ms: Some(started.elapsed().as_secs_f64() * 1_000.0),
                decoding_ms: None,
            },
        })
    }
}
