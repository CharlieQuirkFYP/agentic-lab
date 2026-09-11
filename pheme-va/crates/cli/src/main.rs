use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use crossterm::event::{self, Event, KeyCode};
use va_core::{AudioBuffer, Engine, TranscriptionStatus};

mod model;

#[derive(Debug, Parser)]
#[command(
    name = "pheme-va",
    about = "Record and transcribe speech with the Pheme VA"
)]
struct Cli {
    /// Model ID from the model manifest.
    #[arg(
        long,
        global = true,
        env = "PHEME_VA_STT_MODEL",
        default_value = "whisper-large-v3-turbo"
    )]
    stt_model: String,
    /// Model manifest describing paths and runtime requirements.
    #[arg(
        long,
        global = true,
        env = "PHEME_VA_MODEL_MANIFEST",
        default_value = "models/manifest.toml"
    )]
    model_manifest: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Record from the default microphone, then transcribe the recording.
    Tui {
        /// Maximum duration for one recording.
        #[arg(long, default_value_t = 120)]
        max_seconds: u32,
        /// Whisper language code. Omit for language detection.
        #[arg(long)]
        language: Option<String>,
        /// Comma-separated words/phrases to bias Whisper toward.
        #[arg(long, value_delimiter = ',')]
        dictionary: Vec<String>,
    },
    /// Transcribe an existing WAV file through the same pipeline as the TUI.
    Transcribe {
        /// Path to a WAV file. Any channel count/sample rate is normalized.
        input: PathBuf,
        #[arg(long)]
        language: Option<String>,
        #[arg(long, value_delimiter = ',')]
        dictionary: Vec<String>,
        #[arg(long, default_value_t = 120)]
        max_seconds: u32,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let model_id = cli.stt_model;
    let model_manifest = cli.model_manifest;
    match cli.command {
        Command::Tui {
            max_seconds,
            language,
            dictionary,
        } => run_tui(
            &model_id,
            &model_manifest,
            max_seconds,
            language,
            dictionary,
        ),
        Command::Transcribe {
            input,
            language,
            dictionary,
            max_seconds,
        } => run_transcribe(
            &input,
            &model_id,
            &model_manifest,
            max_seconds,
            language,
            dictionary,
        ),
    }
}

fn run_transcribe(
    input: &Path,
    model_id: &str,
    model_manifest: &Path,
    max_seconds: u32,
    language: Option<String>,
    dictionary: Vec<String>,
) -> Result<()> {
    let wav =
        std::fs::read(input).with_context(|| format!("could not read {}", input.display()))?;
    let mut agent =
        model::create_engine(model_id, model_manifest, max_seconds, language, dictionary)?;
    let result = agent
        .transcribe_wav(&wav)
        .with_context(|| format!("could not transcribe {}", input.display()))?;

    println!("status: {:?}", result.status);
    println!("model: {} ({})", result.model_id, result.model_family);
    println!("backend: {}", result.stt_backend);
    println!("processing: {} ms", result.processing_time_ms);
    println!("raw: {}", result.raw_text);
    println!("text: {}", result.text);
    if result.status != TranscriptionStatus::Speech {
        println!("gate: {:?}", result.gate.decision);
    }
    Ok(())
}

fn run_tui(
    model_id: &str,
    model_manifest: &Path,
    max_seconds: u32,
    language: Option<String>,
    dictionary: Vec<String>,
) -> Result<()> {
    let mut agent =
        model::create_engine(model_id, model_manifest, max_seconds, language, dictionary)?;
    println!("Speech model loaded: {}", agent.model_id());
    println!("The model is prewarmed. No audio is sent anywhere by this CLI.");
    println!();
    println!("Press Enter or Space to start recording; press it again to stop.");
    println!("Press q to quit.");

    crossterm::terminal::enable_raw_mode().context("could not enable terminal input")?;
    let terminal_result = run_recording_loop(&mut agent, max_seconds);
    let _ = crossterm::terminal::disable_raw_mode();
    terminal_result
}

fn run_recording_loop(agent: &mut Engine, max_seconds: u32) -> Result<()> {
    let mut recording: Option<Recording> = None;
    let max_duration = Duration::from_secs(max_seconds as u64);

    loop {
        if event::poll(Duration::from_millis(100)).context("could not read terminal input")? {
            if let Event::Key(key) = event::read().context("could not read terminal event")? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        println!("\r\nExiting.");
                        return Ok(());
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        if let Some(current) = recording.take() {
                            let audio = current.finish()?;
                            println!(
                                "\r\nProcessing {} seconds of audio...",
                                audio.duration_seconds()
                            );
                            print_result(agent.transcribe(audio)?);
                            return Ok(());
                        }

                        recording = Some(Recording::start()?);
                        println!("\rRecording... press Enter or Space to stop.");
                    }
                    _ => {}
                }
            }
        }

        if recording
            .as_ref()
            .is_some_and(|recording| recording.started.elapsed() >= max_duration)
        {
            let current = recording.take().expect("recording existed");
            let audio = current.finish()?;
            println!("\r\nMaximum duration reached; processing...");
            print_result(agent.transcribe(audio)?);
            return Ok(());
        }

        if let Some(current) = recording.as_ref() {
            print!(
                "\rRecording {:.1}s ",
                current.started.elapsed().as_secs_f32()
            );
            std::io::Write::flush(&mut std::io::stdout())?;
        }
    }
}

fn print_result(result: va_core::TranscriptionResult) {
    println!("status: {:?}", result.status);
    println!(
        "gate: {:?} (peak RMS {:.4})",
        result.gate.decision, result.gate.peak_rms
    );
    println!("raw transcript: {}", result.raw_text);
    println!("text: {}", result.text);
    println!(
        "STT: {} ({}) | cleanup: {:?}",
        result.model_id, result.model_family, result.cleanup_status
    );
    println!("processing: {} ms", result.processing_time_ms);
}

struct Recording {
    stream: Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    errors: Arc<Mutex<Vec<String>>>,
    sample_rate: u32,
    channels: u16,
    started: Instant,
}

impl Recording {
    fn start() -> Result<Self> {
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

    fn finish(self) -> Result<AudioBuffer> {
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
}

fn append_samples<I>(sink: &Arc<Mutex<Vec<f32>>>, samples: I)
where
    I: IntoIterator<Item = f32>,
{
    if let Ok(mut sink) = sink.lock() {
        sink.extend(samples.into_iter().map(|sample| sample.clamp(-1.0, 1.0)));
    }
}
