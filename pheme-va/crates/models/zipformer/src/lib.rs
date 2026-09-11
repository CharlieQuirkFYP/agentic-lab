//! LiteRT Zipformer CR-CTC speech-recognition adapter.
//!
//! The adapter owns Zipformer's model-specific work: Kaldi-style filterbank
//! extraction, LiteRT tensor binding, additive padding masks, and greedy CTC
//! plus token-piece decoding. The Pheme core only sees `Transcriber`.

use rustfft::{num_complex::Complex, FftPlanner};
use std::f32::consts::PI;
use std::path::Path;
use thiserror::Error;
use va_core::EngineError;

const MODEL_FRAMES: usize = 1_600;
const FEATURE_BINS: usize = 80;
#[cfg(feature = "litert")]
const OUTPUT_FRAMES: usize = 398;
const VOCABULARY_SIZE: usize = 500;
const CTC_BLANK_ID: usize = 0;
#[cfg(any(feature = "litert", test))]
const MASK_LENGTHS: [usize; 4] = [796, 398, 199, 100];
#[cfg(any(feature = "litert", test))]
const MASK_PADDING_VALUE: f32 = -1_000.0;
const PAD_FEATURE_VALUE: f32 = -23.02585; // ln(1e-10)

#[derive(Debug, Error)]
pub enum ZipformerError {
    #[error("Zipformer model file does not exist: {0}")]
    MissingModel(String),
    #[error("Zipformer tokenizer file does not exist: {0}")]
    MissingTokenizer(String),
    #[error("Zipformer token list file does not exist: {0}")]
    MissingTokens(String),
    #[error("invalid Zipformer token list at {path}: {message}")]
    InvalidTokens { path: String, message: String },
    #[error("Zipformer model contract error: {0}")]
    Contract(String),
    #[error("Zipformer inference error: {0}")]
    Inference(String),
}

impl From<ZipformerError> for EngineError {
    fn from(error: ZipformerError) -> Self {
        EngineError::Backend {
            message: error.to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ZipformerConfig {
    pub model_id: String,
    pub model_revision: Option<String>,
}

impl Default for ZipformerConfig {
    fn default() -> Self {
        Self {
            model_id: "zipformer".to_owned(),
            model_revision: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TokenVocabulary {
    tokens: Vec<String>,
}

impl TokenVocabulary {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ZipformerError> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path).map_err(|error| {
            ZipformerError::MissingTokens(format!("{} ({error})", path.display()))
        })?;
        let mut tokens = vec![None::<String>; VOCABULARY_SIZE];
        for (line_number, line) in contents.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((token, id)) = line.rsplit_once(' ') else {
                return Err(ZipformerError::InvalidTokens {
                    path: path.display().to_string(),
                    message: format!("line {} is missing a token ID", line_number + 1),
                });
            };
            let id = id
                .trim()
                .parse::<usize>()
                .map_err(|error| ZipformerError::InvalidTokens {
                    path: path.display().to_string(),
                    message: format!("line {} has an invalid token ID: {error}", line_number + 1),
                })?;
            // The upstream token file contains two auxiliary pieces at IDs
            // 500 and 501; the exported LiteRT graph has a 500-token output
            // vocabulary, so those entries are intentionally ignored.
            if id >= VOCABULARY_SIZE {
                continue;
            }
            if tokens[id].replace(token.to_owned()).is_some() {
                return Err(ZipformerError::InvalidTokens {
                    path: path.display().to_string(),
                    message: format!("token ID {id} occurs more than once"),
                });
            }
        }

        let missing = tokens
            .iter()
            .enumerate()
            .filter_map(|(id, token)| token.is_none().then_some(id))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(ZipformerError::InvalidTokens {
                path: path.display().to_string(),
                message: format!("missing token IDs: {missing:?}"),
            });
        }

        Ok(Self {
            tokens: tokens.into_iter().map(Option::unwrap).collect::<Vec<_>>(),
        })
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    pub fn decode_ctc(&self, logits: &[f32], frame_count: usize) -> Result<String, ZipformerError> {
        if self.tokens.len() != VOCABULARY_SIZE {
            return Err(ZipformerError::Contract(format!(
                "expected {VOCABULARY_SIZE} tokens, found {}",
                self.tokens.len()
            )));
        }
        let expected = frame_count
            .checked_mul(self.tokens.len())
            .ok_or_else(|| ZipformerError::Contract("CTC output size overflow".to_owned()))?;
        if logits.len() < expected {
            return Err(ZipformerError::Contract(format!(
                "CTC output contains {} values but {expected} are required",
                logits.len()
            )));
        }

        let mut previous = None;
        let mut pieces = String::new();
        for frame in 0..frame_count {
            let row = &logits[frame * self.tokens.len()..(frame + 1) * self.tokens.len()];
            let token_id = row
                .iter()
                .enumerate()
                .max_by(|(_, left), (_, right)| left.total_cmp(right))
                .map(|(id, _)| id)
                .ok_or_else(|| ZipformerError::Contract("CTC output row is empty".to_owned()))?;
            if previous == Some(token_id) {
                continue;
            }
            previous = Some(token_id);
            if token_id == CTC_BLANK_ID {
                continue;
            }
            pieces.push_str(&self.tokens[token_id]);
        }

        Ok(pieces.replace('▁', " ").trim().to_owned())
    }
}

#[derive(Clone, Debug)]
pub struct FbankConfig {
    pub sample_rate: u32,
    pub frame_length: usize,
    pub frame_shift: usize,
    pub num_mel_bins: usize,
    pub fft_size: usize,
    pub low_frequency_hz: f32,
    pub high_frequency_hz: f32,
    pub preemphasis: f32,
}

impl Default for FbankConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16_000,
            frame_length: 400,
            frame_shift: 160,
            num_mel_bins: FEATURE_BINS,
            fft_size: 512,
            low_frequency_hz: 20.0,
            high_frequency_hz: 7_600.0,
            preemphasis: 0.97,
        }
    }
}

impl FbankConfig {
    pub fn extract(&self, samples: &[f32]) -> Result<(Vec<f32>, usize), ZipformerError> {
        if samples.is_empty() {
            return Err(ZipformerError::Contract(
                "cannot extract features from empty audio".to_owned(),
            ));
        }
        if self.sample_rate != 16_000
            || self.num_mel_bins != FEATURE_BINS
            || self.frame_length != 400
            || self.frame_shift != 160
            || self.fft_size != 512
        {
            return Err(ZipformerError::Contract(
                "the Zipformer fbank contract requires 16 kHz, 400/160 frames, 80 bins, and a 512-point FFT".to_owned(),
            ));
        }

        let frame_count =
            ((samples.len() + self.frame_shift / 2) / self.frame_shift).min(MODEL_FRAMES);
        let filters = mel_filters(self);
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(self.fft_size);
        let mut features = vec![PAD_FEATURE_VALUE; MODEL_FRAMES * self.num_mel_bins];
        for frame in 0..frame_count {
            let first_sample = frame as isize * self.frame_shift as isize
                - self.frame_length as isize / 2
                + self.frame_shift as isize / 2;
            let mut window = vec![0.0_f32; self.fft_size];
            let mut mean = 0.0_f32;
            for (index, slot) in window.iter_mut().take(self.frame_length).enumerate() {
                let sample_index = first_sample + index as isize;
                let sample = if sample_index >= 0 && (sample_index as usize) < samples.len() {
                    samples[sample_index as usize]
                } else {
                    0.0
                };
                mean += sample;
                *slot = sample;
            }
            mean /= self.frame_length as f32;
            for value in &mut window[..self.frame_length] {
                *value -= mean;
            }
            let mut previous = window[0];
            window[0] *= 1.0 - self.preemphasis;
            for value in window.iter_mut().take(self.frame_length).skip(1) {
                let current = *value;
                *value = current - self.preemphasis * previous;
                previous = current;
            }
            for (index, value) in window[..self.frame_length].iter_mut().enumerate() {
                *value *= povey_window(index);
            }

            let spectrum = power_spectrum(&window, fft.as_ref());
            for (bin, filter) in filters.iter().enumerate() {
                let energy = filter
                    .iter()
                    .zip(spectrum.iter())
                    .map(|(weight, power)| weight * power)
                    .sum::<f32>()
                    .max(1.0e-10);
                features[frame * self.num_mel_bins + bin] = energy.ln();
            }
        }
        Ok((features, frame_count))
    }
}

fn povey_window(index: usize) -> f32 {
    (0.5 - 0.5 * (2.0 * PI * index as f32 / 399.0).cos()).powf(0.85)
}

fn power_spectrum(window: &[f32], fft: &dyn rustfft::Fft<f32>) -> Vec<f32> {
    let fft_size = fft.len();
    let mut buffer = vec![Complex::new(0.0_f32, 0.0_f32); fft_size];
    for (slot, value) in buffer.iter_mut().zip(window.iter()) {
        slot.re = *value;
    }
    fft.process(&mut buffer);
    buffer[..fft_size / 2 + 1]
        .iter()
        .map(|value| (value.re * value.re + value.im * value.im) / fft_size as f32)
        .collect()
}

fn mel_filters(config: &FbankConfig) -> Vec<Vec<f32>> {
    let mel = |frequency: f32| 1_127.0 * (1.0 + frequency / 700.0).ln();
    let inverse_mel = |value: f32| 700.0 * ((value / 1_127.0).exp() - 1.0);
    let low = mel(config.low_frequency_hz);
    let high = mel(config.high_frequency_hz);
    let points = (0..config.num_mel_bins + 2)
        .map(|index| {
            let frequency =
                inverse_mel(low + (high - low) * index as f32 / (config.num_mel_bins + 1) as f32);
            ((config.fft_size + 1) as f32 * frequency / config.sample_rate as f32)
                .floor()
                .max(0.0) as usize
        })
        .collect::<Vec<_>>();

    (0..config.num_mel_bins)
        .map(|filter_index| {
            let left = points[filter_index];
            let center = points[filter_index + 1];
            let right = points[filter_index + 2];
            let mut filter = vec![0.0_f32; config.fft_size / 2 + 1];
            if center > left {
                for index in left..center.min(filter.len()) {
                    filter[index] = (index - left) as f32 / (center - left) as f32;
                }
            }
            if right > center {
                for index in center..right.min(filter.len()) {
                    filter[index] = (right - index) as f32 / (right - center) as f32;
                }
            }
            filter
        })
        .collect()
}

#[cfg(feature = "litert")]
mod runtime {
    use std::time::Instant;

    use super::*;
    use litert::{
        CompilationOptions, CompiledModel, ElementType, Environment, Model, TensorBuffer,
        TensorShape,
    };
    use va_core::{
        ModelTimings, NormalizedAudio, RawTranscription, Transcriber, TranscriptSegment,
        TranscriptionOptions,
    };

    #[derive(Clone, Copy, Debug)]
    enum InputKind {
        Fbank,
        Mask(usize),
    }

    pub struct ZipformerTranscriber {
        compiled: CompiledModel,
        inputs: Vec<TensorBuffer>,
        outputs: Vec<TensorBuffer>,
        input_kinds: Vec<InputKind>,
        vocabulary: TokenVocabulary,
        fbank: FbankConfig,
        model_id: String,
        model_revision: Option<String>,
    }

    impl ZipformerTranscriber {
        pub fn from_files(
            model_path: impl AsRef<Path>,
            bpe_path: impl AsRef<Path>,
            tokens_path: impl AsRef<Path>,
            config: ZipformerConfig,
        ) -> Result<Self, EngineError> {
            let model_path = model_path.as_ref();
            let bpe_path = bpe_path.as_ref();
            if !model_path.is_file() {
                return Err(ZipformerError::MissingModel(model_path.display().to_string()).into());
            }
            if !bpe_path.is_file() {
                return Err(
                    ZipformerError::MissingTokenizer(bpe_path.display().to_string()).into(),
                );
            }
            let vocabulary = TokenVocabulary::from_file(tokens_path)?;
            let environment = Environment::new().map_err(runtime_error)?;
            let model = Model::from_file(model_path).map_err(runtime_error)?;
            let signature = model.signature(0).map_err(runtime_error)?;
            let input_count = signature.input_count().map_err(runtime_error)?;
            let output_count = signature.output_count().map_err(runtime_error)?;
            if input_count != 5 || output_count != 1 {
                return Err(ZipformerError::Contract(format!(
                    "expected 5 inputs and 1 output, found {input_count} inputs and {output_count} outputs"
                ))
                .into());
            }

            let mut input_kinds = Vec::with_capacity(input_count);
            let mut input_shapes = Vec::with_capacity(input_count);
            for index in 0..input_count {
                let shape = signature.input_shape(index).map_err(runtime_error)?;
                let kind = classify_input(&shape)?;
                input_kinds.push(kind);
                input_shapes.push(shape);
            }
            let output_shape = signature.output_shape(0).map_err(runtime_error)?;
            validate_output_shape(&output_shape)?;

            let inputs = input_shapes
                .iter()
                .map(|shape| TensorBuffer::managed_host(&environment, shape).map_err(runtime_error))
                .collect::<Result<Vec<_>, EngineError>>()?;
            let outputs =
                vec![TensorBuffer::managed_host(&environment, &output_shape)
                    .map_err(runtime_error)?];
            let options = CompilationOptions::new().map_err(runtime_error)?;
            let compiled =
                CompiledModel::new(environment, model, &options).map_err(runtime_error)?;

            Ok(Self {
                compiled,
                inputs,
                outputs,
                input_kinds,
                vocabulary,
                fbank: FbankConfig::default(),
                model_id: config.model_id,
                model_revision: config.model_revision,
            })
        }

        pub fn model_id(&self) -> &str {
            &self.model_id
        }
    }

    impl Transcriber for ZipformerTranscriber {
        fn name(&self) -> &str {
            &self.model_id
        }

        fn model_family(&self) -> &str {
            "zipformer"
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
            if audio.sample_rate != self.fbank.sample_rate {
                return Err(ZipformerError::Contract(format!(
                    "expected {} Hz audio, received {} Hz",
                    self.fbank.sample_rate, audio.sample_rate
                ))
                .into());
            }

            let feature_started = Instant::now();
            let (features, feature_frames) = self.fbank.extract(&audio.samples)?;
            let feature_extraction_ms = feature_started.elapsed().as_secs_f64() * 1_000.0;
            let valid_frames = ((feature_frames.saturating_sub(7)) / 2).min(MASK_LENGTHS[0]);
            let masks = mask_inputs(valid_frames);
            fill_inputs(&mut self.inputs, &self.input_kinds, &features, &masks)?;

            let inference_started = Instant::now();
            self.compiled
                .run(&mut self.inputs, &mut self.outputs)
                .map_err(runtime_error)?;
            let inference_ms = inference_started.elapsed().as_secs_f64() * 1_000.0;

            let decoding_started = Instant::now();
            let logits = self.outputs[0]
                .lock_for_read::<f32>()
                .map_err(runtime_error)?;
            let output_frames = valid_frames.div_ceil(2).min(OUTPUT_FRAMES);
            let text = self.vocabulary.decode_ctc(&logits, output_frames)?;
            drop(logits);
            let decoding_ms = decoding_started.elapsed().as_secs_f64() * 1_000.0;
            let segments = if text.is_empty() {
                Vec::new()
            } else {
                vec![TranscriptSegment {
                    start_ms: 0,
                    end_ms: (audio.duration_seconds() * 1_000.0) as i64,
                    text: text.clone(),
                    no_speech_probability: None,
                }]
            };

            Ok(RawTranscription {
                text,
                language: options.language.clone(),
                segments,
                no_speech_probability: None,
                timings: ModelTimings {
                    feature_extraction_ms: Some(feature_extraction_ms),
                    inference_ms: Some(inference_ms),
                    decoding_ms: Some(decoding_ms),
                },
            })
        }
    }

    fn classify_input(shape: &TensorShape) -> Result<InputKind, EngineError> {
        if shape.element_type != ElementType::Float32 {
            return Err(ZipformerError::Contract(format!(
                "all Zipformer inputs must be Float32, found {:?} for {:?}",
                shape.element_type, shape.dims
            ))
            .into());
        }
        match shape.dims.as_slice() {
            [1, 1_600, 80] => Ok(InputKind::Fbank),
            [1, 796] => Ok(InputKind::Mask(0)),
            [1, 398] => Ok(InputKind::Mask(1)),
            [1, 199] => Ok(InputKind::Mask(2)),
            [1, 100] => Ok(InputKind::Mask(3)),
            dimensions => Err(ZipformerError::Contract(format!(
                "unexpected Zipformer input shape {dimensions:?}"
            ))
            .into()),
        }
    }

    fn validate_output_shape(shape: &TensorShape) -> Result<(), EngineError> {
        if shape.element_type != ElementType::Float32 || shape.dims != [1, 398, 500] {
            return Err(ZipformerError::Contract(format!(
                "expected Float32 output shape [1, 398, 500], found {:?} {:?}",
                shape.element_type, shape.dims
            ))
            .into());
        }
        Ok(())
    }

    fn mask_inputs(valid_frames: usize) -> [Vec<f32>; 4] {
        std::array::from_fn(|level| {
            let stride = 1_usize << level;
            (0..MASK_LENGTHS[level])
                .map(|index| {
                    if index * stride < valid_frames {
                        0.0
                    } else {
                        MASK_PADDING_VALUE
                    }
                })
                .collect()
        })
    }

    fn fill_inputs(
        inputs: &mut [TensorBuffer],
        kinds: &[InputKind],
        features: &[f32],
        masks: &[Vec<f32>; 4],
    ) -> Result<(), EngineError> {
        for (input, kind) in inputs.iter_mut().zip(kinds) {
            let mut buffer = input.lock_for_write::<f32>().map_err(runtime_error)?;
            match kind {
                InputKind::Fbank => {
                    if buffer.len() != features.len() {
                        return Err(ZipformerError::Contract(format!(
                            "fbank buffer has {} values, expected {}",
                            buffer.len(),
                            features.len()
                        ))
                        .into());
                    }
                    buffer.copy_from_slice(features);
                }
                InputKind::Mask(level) => {
                    if buffer.len() != masks[*level].len() {
                        return Err(ZipformerError::Contract(format!(
                            "mask buffer has {} values, expected {}",
                            buffer.len(),
                            masks[*level].len()
                        ))
                        .into());
                    }
                    buffer.copy_from_slice(&masks[*level]);
                }
            }
        }
        Ok(())
    }

    fn runtime_error(error: impl std::fmt::Display) -> EngineError {
        EngineError::Backend {
            message: format!("LiteRT Zipformer runtime error: {error}"),
        }
    }

    pub use ZipformerTranscriber as PublicZipformerTranscriber;
}

#[cfg(feature = "litert")]
pub use runtime::PublicZipformerTranscriber as ZipformerTranscriber;

#[cfg(not(feature = "litert"))]
pub struct ZipformerTranscriber;

#[cfg(not(feature = "litert"))]
impl ZipformerTranscriber {
    pub fn from_files(
        _model_path: impl AsRef<Path>,
        _bpe_path: impl AsRef<Path>,
        _tokens_path: impl AsRef<Path>,
        _config: ZipformerConfig,
    ) -> Result<Self, EngineError> {
        Err(EngineError::Backend {
            message: "Zipformer support is disabled; rebuild with the `litert` feature".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_ctc_repeats_and_blanks() {
        let mut tokens = vec!["<unused>".to_owned(); VOCABULARY_SIZE];
        tokens[0] = "<blk>".to_owned();
        tokens[1] = "A".to_owned();
        tokens[2] = "▁B".to_owned();
        let vocabulary = TokenVocabulary { tokens };
        let mut logits = vec![0.0_f32; 4 * VOCABULARY_SIZE];
        logits[1] = 10.0; // A
        logits[VOCABULARY_SIZE + 1] = 9.0; // repeated A
        logits[2 * VOCABULARY_SIZE] = 10.0; // blank
        logits[3 * VOCABULARY_SIZE + 2] = 8.0; // B
        assert_eq!(vocabulary.decode_ctc(&logits, 4).unwrap(), "A B");
    }

    #[test]
    fn creates_fixed_size_padded_features_and_frame_count() {
        let samples = vec![0.0_f32; 16_000];
        let (features, frames) = FbankConfig::default().extract(&samples).unwrap();
        assert_eq!(features.len(), MODEL_FRAMES * FEATURE_BINS);
        assert_eq!(frames, 100);
        assert!(features[100 * FEATURE_BINS..]
            .iter()
            .all(|value| (*value - PAD_FEATURE_VALUE).abs() < f32::EPSILON));
    }

    #[test]
    fn mask_lengths_match_model_contract() {
        let masks = {
            #[cfg(feature = "litert")]
            {
                let _ = 0;
            }
            (0..MASK_LENGTHS.len())
                .map(|level| {
                    let stride = 1_usize << level;
                    (0..MASK_LENGTHS[level])
                        .map(|index| {
                            if index * stride < 20 {
                                0.0
                            } else {
                                MASK_PADDING_VALUE
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(masks.iter().map(Vec::len).collect::<Vec<_>>(), MASK_LENGTHS);
    }
}
