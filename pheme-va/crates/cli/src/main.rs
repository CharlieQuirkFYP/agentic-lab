use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use va_core::TranscriptionStatus;

mod model;
mod recorder;
mod tui;

const DEFAULT_MODEL_ID: &str = "whisper-large-v3-turbo";
const DEFAULT_MANIFEST: &str = "models/manifest.toml";

#[derive(Debug, Parser)]
#[command(
    name = "pheme-va",
    about = "Record and transcribe speech with the Pheme VA"
)]
struct Cli {
    /// Model ID from the model manifest.
    #[arg(long, global = true, env = "PHEME_VA_STT_MODEL")]
    stt_model: Option<String>,
    /// Model manifest describing paths and runtime requirements.
    #[arg(long, global = true, env = "PHEME_VA_MODEL_MANIFEST")]
    model_manifest: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Open the standalone speech bench or connected voice console.
    Tui {
        /// Connect through Go. Without this flag, use the standalone local STT bench.
        #[arg(long, num_args = 0..=1, default_missing_value = "http://127.0.0.1:8080")]
        server_url: Option<String>,
        /// Maximum duration for one microphone recording.
        #[arg(long)]
        max_seconds: Option<u32>,
        /// Microphone/WAV language code. Omit for language detection.
        #[arg(long)]
        language: Option<String>,
        /// Comma-separated words/phrases to bias transcription toward.
        #[arg(long, value_delimiter = ',')]
        dictionary: Option<Vec<String>>,
        /// Audio directory used by the WAV browser.
        #[arg(long)]
        audio_directory: Option<PathBuf>,
        /// Always open the first-run model and audio setup.
        #[arg(long)]
        reconfigure: bool,
    },
    /// Transcribe an existing WAV file through the same core pipeline.
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
    let Cli {
        stt_model,
        model_manifest,
        command,
    } = Cli::parse();
    match command {
        Command::Tui {
            server_url,
            max_seconds,
            language,
            dictionary,
            audio_directory,
            reconfigure,
        } => tui::run(tui::TuiOptions {
            server_url,
            model_id: stt_model,
            model_manifest,
            audio_directory,
            max_seconds,
            language,
            dictionary,
            reconfigure,
        }),
        Command::Transcribe {
            input,
            language,
            dictionary,
            max_seconds,
        } => run_transcribe(
            &input,
            stt_model.as_deref().unwrap_or(DEFAULT_MODEL_ID),
            model_manifest
                .as_deref()
                .unwrap_or_else(|| Path::new(DEFAULT_MANIFEST)),
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

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn server_flag_is_explicit_and_bare_flag_uses_loopback_default() {
        let Cli {
            command: Command::Tui { server_url, .. },
            ..
        } = Cli::try_parse_from(["cli", "tui"]).unwrap()
        else {
            panic!("expected tui")
        };
        assert!(server_url.is_none());
        let Cli {
            command: Command::Tui { server_url, .. },
            ..
        } = Cli::try_parse_from(["cli", "tui", "--server-url"]).unwrap()
        else {
            panic!("expected tui")
        };
        assert_eq!(server_url.as_deref(), Some("http://127.0.0.1:8080"));
        let Cli {
            command: Command::Tui { server_url, .. },
            ..
        } = Cli::try_parse_from(["cli", "tui", "--server-url", "http://localhost:9000"]).unwrap()
        else {
            panic!("expected tui")
        };
        assert_eq!(server_url.as_deref(), Some("http://localhost:9000"));
    }
}
