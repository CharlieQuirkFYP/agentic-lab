use std::io::Cursor;

use hound::{SampleFormat, WavReader};
use serde::Serialize;
use thiserror::Error;

pub const TARGET_SAMPLE_RATE: u32 = 16_000;
const DEFAULT_WINDOW_MS: u32 = 20;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("audio has no samples")]
    Empty,
    #[error("sample rate must be greater than zero")]
    InvalidSampleRate,
    #[error("audio must contain at least one channel")]
    InvalidChannelCount,
    #[error("audio samples are not interleaved correctly for {channels} channels")]
    IncompleteFrame { channels: u16 },
    #[error("audio sample {index} is not finite")]
    NonFiniteSample { index: usize },
    #[error("audio duration exceeds the {max_seconds} second limit")]
    TooLong { max_seconds: u32 },
    #[error("unsupported WAV format: {sample_format:?} with {bits} bits")]
    UnsupportedWavFormat {
        sample_format: SampleFormat,
        bits: u16,
    },
    #[error("invalid WAV: {0}")]
    Wav(#[from] hound::Error),
}

#[derive(Clone, Debug)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved floating-point samples in the range [-1.0, 1.0].
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    pub fn new(sample_rate: u32, channels: u16, samples: Vec<f32>) -> Result<Self, AudioError> {
        if sample_rate == 0 {
            return Err(AudioError::InvalidSampleRate);
        }
        if channels == 0 {
            return Err(AudioError::InvalidChannelCount);
        }
        if samples.is_empty() {
            return Err(AudioError::Empty);
        }
        if samples.len() % channels as usize != 0 {
            return Err(AudioError::IncompleteFrame { channels });
        }
        for (index, sample) in samples.iter().enumerate() {
            if !sample.is_finite() {
                return Err(AudioError::NonFiniteSample { index });
            }
        }
        Ok(Self {
            sample_rate,
            channels,
            samples,
        })
    }

    pub fn duration_seconds(&self) -> f32 {
        self.samples.len() as f32 / self.channels as f32 / self.sample_rate as f32
    }

    pub fn from_wav(bytes: &[u8]) -> Result<Self, AudioError> {
        let mut reader = WavReader::new(Cursor::new(bytes))?;
        let spec = reader.spec();
        let samples = match spec.sample_format {
            SampleFormat::Float if spec.bits_per_sample == 32 => {
                reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?
            }
            SampleFormat::Int => match spec.bits_per_sample {
                8 => reader
                    .samples::<i8>()
                    .map(|sample| sample.map(|value| value as f32 / 128.0))
                    .collect::<Result<Vec<_>, _>>()?,
                16 => reader
                    .samples::<i16>()
                    .map(|sample| sample.map(|value| value as f32 / 32_768.0))
                    .collect::<Result<Vec<_>, _>>()?,
                24 => reader
                    .samples::<i32>()
                    .map(|sample| sample.map(|value| value as f32 / 8_388_608.0))
                    .collect::<Result<Vec<_>, _>>()?,
                32 => reader
                    .samples::<i32>()
                    .map(|sample| sample.map(|value| value as f32 / 2_147_483_648.0))
                    .collect::<Result<Vec<_>, _>>()?,
                bits => {
                    return Err(AudioError::UnsupportedWavFormat {
                        sample_format: spec.sample_format,
                        bits,
                    })
                }
            },
            sample_format => {
                return Err(AudioError::UnsupportedWavFormat {
                    sample_format,
                    bits: spec.bits_per_sample,
                })
            }
        };

        Self::new(spec.sample_rate, spec.channels, samples)
    }

    /// Convert to mono and 16 kHz using a small windowed-sinc resampler.
    ///
    /// This intentionally happens in the portable core rather than in a host
    /// recorder. iOS, Android, desktop, and HTTP callers therefore present the
    /// recognizer with the same audio contract.
    pub fn normalize(&self, max_seconds: Option<u32>) -> Result<NormalizedAudio, AudioError> {
        if let Some(max_seconds) = max_seconds {
            if self.duration_seconds() > max_seconds as f32 {
                return Err(AudioError::TooLong { max_seconds });
            }
        }

        let mono = downmix_to_mono(&self.samples, self.channels);
        let samples = resample(&mono, self.sample_rate, TARGET_SAMPLE_RATE);
        let gate = SpeechGateConfig::default().analyze(&samples, TARGET_SAMPLE_RATE);
        Ok(NormalizedAudio {
            sample_rate: TARGET_SAMPLE_RATE,
            samples,
            original_sample_rate: self.sample_rate,
            original_channels: self.channels,
            gate,
        })
    }
}

#[derive(Clone, Debug)]
pub struct NormalizedAudio {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
    pub original_sample_rate: u32,
    pub original_channels: u16,
    pub gate: SpeechGateResult,
}

impl NormalizedAudio {
    pub fn duration_seconds(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SpeechGateConfig {
    pub window_ms: u32,
    pub silence_rms_threshold: f32,
    pub speech_window_rms_threshold: f32,
    pub speech_window_peak_threshold: f32,
    pub strong_speech_rms_threshold: f32,
    pub minimum_speech_windows: usize,
}

impl Default for SpeechGateConfig {
    fn default() -> Self {
        // These are conservative starting values inspired by OpenWhispr's
        // local gate. They are configuration, not validated mobile defaults.
        Self {
            window_ms: DEFAULT_WINDOW_MS,
            silence_rms_threshold: 0.002,
            speech_window_rms_threshold: 0.003,
            speech_window_peak_threshold: 0.02,
            strong_speech_rms_threshold: 0.006,
            minimum_speech_windows: 1,
        }
    }
}

impl SpeechGateConfig {
    pub fn analyze(&self, samples: &[f32], sample_rate: u32) -> SpeechGateResult {
        if samples.is_empty() || sample_rate == 0 {
            return SpeechGateResult {
                decision: SpeechGateDecision::Unavailable,
                peak_rms: 0.0,
                peak_amplitude: 0.0,
                window_count: 0,
                speech_window_count: 0,
                max_consecutive_speech_windows: 0,
            };
        }

        let window_size = ((sample_rate as u64 * self.window_ms as u64) / 1_000).max(1) as usize;
        let mut peak_rms: f32 = 0.0;
        let mut peak_amplitude: f32 = 0.0;
        let mut window_count = 0;
        let mut speech_window_count = 0;
        let mut consecutive_speech_windows = 0;
        let mut max_consecutive_speech_windows = 0;

        for window in samples.chunks(window_size) {
            window_count += 1;
            let rms = root_mean_square(window);
            let peak = window
                .iter()
                .map(|sample| sample.abs())
                .fold(0.0_f32, f32::max);
            peak_rms = peak_rms.max(rms);
            peak_amplitude = peak_amplitude.max(peak);

            if rms >= self.speech_window_rms_threshold && peak >= self.speech_window_peak_threshold
            {
                speech_window_count += 1;
                consecutive_speech_windows += 1;
                max_consecutive_speech_windows =
                    max_consecutive_speech_windows.max(consecutive_speech_windows);
            } else {
                consecutive_speech_windows = 0;
            }
        }

        let decision = if peak_rms < self.silence_rms_threshold {
            SpeechGateDecision::Silence
        } else if speech_window_count >= self.minimum_speech_windows
            || peak_rms >= self.strong_speech_rms_threshold
        {
            SpeechGateDecision::SpeechDetected
        } else {
            SpeechGateDecision::InsufficientSpeech
        };

        SpeechGateResult {
            decision,
            peak_rms,
            peak_amplitude,
            window_count,
            speech_window_count,
            max_consecutive_speech_windows,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum SpeechGateDecision {
    SpeechDetected,
    Silence,
    InsufficientSpeech,
    Unavailable,
}

impl SpeechGateDecision {
    pub fn should_skip_transcription(self) -> bool {
        matches!(
            self,
            Self::Silence | Self::InsufficientSpeech | Self::Unavailable
        )
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SpeechGateResult {
    pub decision: SpeechGateDecision,
    pub peak_rms: f32,
    pub peak_amplitude: f32,
    pub window_count: usize,
    pub speech_window_count: usize,
    pub max_consecutive_speech_windows: usize,
}

fn downmix_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels == 1 {
        return samples.to_vec();
    }
    samples
        .chunks(channels as usize)
        .map(|frame| frame.iter().copied().sum::<f32>() / frame.len() as f32)
        .collect()
}

fn resample(samples: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    if samples.is_empty() || source_rate == target_rate {
        return samples.to_vec();
    }

    let ratio = target_rate as f64 / source_rate as f64;
    let output_len = (samples.len() as f64 * ratio).round().max(1.0) as usize;
    let cutoff = 0.5 * ratio.min(1.0);
    let radius = 16.0_f64;
    let mut output = Vec::with_capacity(output_len);

    for output_index in 0..output_len {
        let source_position = output_index as f64 / ratio;
        let first = source_position.floor() as isize - radius as isize + 1;
        let mut value = 0.0_f64;
        let mut weight_sum = 0.0_f64;

        for tap in 0..(radius as isize * 2) {
            let source_index = first + tap;
            if source_index < 0 || source_index >= samples.len() as isize {
                continue;
            }
            let distance = source_position - source_index as f64;
            if distance.abs() >= radius {
                continue;
            }
            let sinc_argument = 2.0 * cutoff * distance;
            let sinc = if sinc_argument.abs() < f64::EPSILON {
                1.0
            } else {
                (std::f64::consts::PI * sinc_argument).sin()
                    / (std::f64::consts::PI * sinc_argument)
            };
            let window = 0.5 * (1.0 + (std::f64::consts::PI * distance / radius).cos());
            let weight = 2.0 * cutoff * sinc * window;
            value += samples[source_index as usize] as f64 * weight;
            weight_sum += weight;
        }

        let sample = if weight_sum.abs() > f64::EPSILON {
            value / weight_sum
        } else {
            samples[(source_position.floor() as usize).min(samples.len() - 1)] as f64
        };
        output.push(sample.clamp(-1.0, 1.0) as f32);
    }
    output
}

fn root_mean_square(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmixes_stereo_and_resamples_to_whisper_format() {
        let source_rate = 8_000;
        let source_samples = vec![0.2, 0.4, 0.2, 0.4];
        let audio = AudioBuffer::new(source_rate, 2, source_samples).unwrap();
        let normalized = audio.normalize(None).unwrap();

        assert_eq!(normalized.sample_rate, TARGET_SAMPLE_RATE);
        assert_eq!(normalized.original_channels, 2);
        assert_eq!(normalized.samples.len(), 16_000 / 8_000 * 2);
        assert!(normalized.samples.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn silent_audio_is_rejected_by_the_gate() {
        let config = SpeechGateConfig::default();
        let result = config.analyze(&vec![0.0; TARGET_SAMPLE_RATE as usize], TARGET_SAMPLE_RATE);
        assert_eq!(result.decision, SpeechGateDecision::Silence);
        assert!(result.decision.should_skip_transcription());
    }

    #[test]
    fn speech_gate_requires_energy_and_peak() {
        let config = SpeechGateConfig::default();
        let result = config.analyze(&vec![0.05; TARGET_SAMPLE_RATE as usize], TARGET_SAMPLE_RATE);
        assert_eq!(result.decision, SpeechGateDecision::SpeechDetected);
        assert_eq!(result.speech_window_count, 50);
    }
}
