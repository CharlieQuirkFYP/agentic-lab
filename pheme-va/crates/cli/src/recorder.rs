use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use va_core::AudioBuffer;

pub struct Recording {
    stream: Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    errors: Arc<Mutex<Vec<String>>>,
    sample_rate: u32,
    channels: u16,
    started: Instant,
}

impl Recording {
    pub fn start() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow!("no default input device was found"))?;
        let supported_config = device
            .default_input_config()
            .context("could not query the default microphone format")?;
        let sample_format = supported_config.sample_format();
        let config: StreamConfig = supported_config.into();
        let sample_rate = config.sample_rate.0;
        let channels = config.channels;
        let samples = Arc::new(Mutex::new(Vec::new()));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let sample_sink = Arc::clone(&samples);
        let error_sink = Arc::clone(&errors);
        let error_callback = move |error: cpal::StreamError| {
            if let Ok(mut errors) = error_sink.lock() {
                errors.push(error.to_string());
            }
        };

        let stream = match sample_format {
            SampleFormat::F32 => {
                let sink = Arc::clone(&sample_sink);
                device.build_input_stream(
                    &config,
                    move |data: &[f32], _| append_samples(&sink, data.iter().copied()),
                    error_callback,
                    None,
                )?
            }
            SampleFormat::I16 => {
                let sink = Arc::clone(&sample_sink);
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _| {
                        append_samples(&sink, data.iter().map(|sample| *sample as f32 / 32_768.0))
                    },
                    error_callback,
                    None,
                )?
            }
            SampleFormat::U16 => {
                let sink = Arc::clone(&sample_sink);
                device.build_input_stream(
                    &config,
                    move |data: &[u16], _| {
                        append_samples(
                            &sink,
                            data.iter().map(|sample| *sample as f32 / 32_768.0 - 1.0),
                        )
                    },
                    error_callback,
                    None,
                )?
            }
            format => return Err(anyhow!("unsupported microphone sample format: {format:?}")),
        };
        stream
            .play()
            .context("could not start the microphone stream")?;

        Ok(Self {
            stream,
            samples,
            errors,
            sample_rate,
            channels,
            started: Instant::now(),
        })
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn finish(self) -> Result<AudioBuffer> {
        drop(self.stream);
        let errors = self
            .errors
            .lock()
            .map_err(|_| anyhow!("microphone error state was poisoned"))?
            .clone();
        if !errors.is_empty() {
            return Err(anyhow!("microphone stream failed: {}", errors.join("; ")));
        }
        let samples = self
            .samples
            .lock()
            .map_err(|_| anyhow!("microphone sample buffer was poisoned"))?
            .clone();
        AudioBuffer::new(self.sample_rate, self.channels, samples)
            .map_err(|error| anyhow!("recorded audio was invalid: {error}"))
    }

    pub fn discard(self) {
        drop(self.stream);
    }
}

fn append_samples<I>(sink: &Arc<Mutex<Vec<f32>>>, samples: I)
where
    I: IntoIterator<Item = f32>,
{
    if let Ok(mut sink) = sink.lock() {
        sink.extend(samples.into_iter().map(|sample| sample.clamp(-1.0, 1.0)));
    }
}
