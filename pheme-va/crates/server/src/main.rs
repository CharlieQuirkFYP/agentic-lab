use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use serde::{Deserialize, Serialize};
use va_core::{Engine, IncidentAnalyzer, RuleBasedIncidentAnalyzer};

#[derive(Debug, Parser)]
#[command(name = "server", about = "HTTP wrapper around the Pheme VA core")]
struct Args {
    #[arg(long, env = "PHEME_VA_WHISPER_MODEL")]
    model: PathBuf,
    #[arg(long, default_value = "127.0.0.1:8000")]
    bind: String,
    #[arg(long, default_value_t = 120)]
    max_seconds: u32,
    #[arg(long)]
    language: Option<String>,
    #[arg(long, value_delimiter = ',')]
    dictionary: Vec<String>,
}

#[derive(Clone)]
struct AppState {
    engine: Arc<Mutex<Engine>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalyzeRequest {
    transcript: String,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Debug, Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: &'static str,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let engine = create_engine(
        &args.model,
        args.max_seconds,
        args.language,
        args.dictionary,
    )?;
    let state = AppState {
        engine: Arc::new(Mutex::new(engine)),
    };
    let address: std::net::SocketAddr = args
        .bind
        .parse()
        .with_context(|| format!("invalid bind address {}", args.bind))?;
    let app = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/v1/transcribe", post(transcribe))
        .route("/v1/analyze", post(analyze))
        .with_state(state);

    println!("server listening on http://{address}");
    println!("POST audio/wav to /v1/transcribe");
    axum::serve(tokio::net::TcpListener::bind(address).await?, app)
        .await
        .context("Pheme VA server stopped")?;
    Ok(())
}

#[cfg(feature = "whisper")]
fn create_engine(
    model: &Path,
    max_seconds: u32,
    language: Option<String>,
    dictionary: Vec<String>,
) -> Result<Engine> {
    use va_core::{
        DictionaryHints, EngineConfig, RuleBasedFormatter, WhisperConfig, WhisperTranscriber,
    };

    let transcriber = WhisperTranscriber::from_file(
        model,
        WhisperConfig {
            threads: std::thread::available_parallelism()
                .map(|threads| threads.get().min(8) as i32)
                .unwrap_or(4),
            use_gpu: cfg!(feature = "whisper-metal"),
            flash_attention: false,
        },
    )
    .with_context(|| format!("failed to load Whisper model {}", model.display()))?;
    let config = EngineConfig {
        max_audio_seconds: Some(max_seconds),
        language,
        dictionary: DictionaryHints::with_terms(dictionary),
        ..EngineConfig::default()
    };
    Ok(Engine::with_config(transcriber, config).with_cleaner(RuleBasedFormatter))
}

#[cfg(not(feature = "whisper"))]
fn create_engine(
    _model: &Path,
    _max_seconds: u32,
    _language: Option<String>,
    _dictionary: Vec<String>,
) -> Result<Engine> {
    Err(anyhow!(
        "this binary was built without Whisper support; run with `cargo run --release -p server --features whisper -- --model <path>`"
    ))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn ready(State(state): State<AppState>) -> Response {
    let ready = state
        .engine
        .lock()
        .map(|engine| engine.is_ready())
        .unwrap_or(false);
    if ready {
        Json(HealthResponse { status: "ready" }).into_response()
    } else {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
            "Analysis runtime is unavailable.",
        )
    }
}

async fn transcribe(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let content_type = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if content_type != "audio/wav" && content_type != "audio/x-wav" {
        return api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Content-Type must be audio/wav.",
        );
    }
    if body.is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "A non-empty WAV body is required.",
        );
    }

    let engine = Arc::clone(&state.engine);
    let result = tokio::task::spawn_blocking(move || {
        let mut engine = engine
            .lock()
            .map_err(|_| anyhow!("engine lock was poisoned"))?;
        engine
            .transcribe_wav(&body)
            .map_err(|error| anyhow!(error.to_string()))
    })
    .await;

    match result {
        Ok(Ok(transcription)) => Json(transcription).into_response(),
        Ok(Err(error)) => map_engine_error(&error.to_string()),
        Err(error) => {
            eprintln!("transcription task failed: {error}");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "transcription_failed",
                "Transcription failed.",
            )
        }
    }
}

async fn analyze(State(_state): State<AppState>, Json(request): Json<AnalyzeRequest>) -> Response {
    let transcript = request.transcript.trim();
    if transcript.is_empty() || transcript.chars().count() > 16_000 {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "A non-empty transcript of at most 16000 characters is required.",
        );
    }
    let mut analyzer = RuleBasedIncidentAnalyzer;
    match analyzer.analyze(transcript) {
        Ok(report) => Json(report).into_response(),
        Err(_) => api_error(
            StatusCode::BAD_GATEWAY,
            "invalid_model_response",
            "Analysis runtime returned an invalid response.",
        ),
    }
}

fn map_engine_error(message: &str) -> Response {
    if message.contains("duration exceeds") {
        api_error(
            StatusCode::BAD_REQUEST,
            "audio_too_long",
            "Audio exceeds the configured duration limit.",
        )
    } else if message.contains("not ready") || message.contains("could not load") {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
            "Analysis runtime is unavailable.",
        )
    } else {
        eprintln!("transcription failed: {message}");
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "transcription_failed",
            "Transcription failed.",
        )
    }
}

fn api_error(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: ErrorDetail { code, message },
        }),
    )
        .into_response()
}
