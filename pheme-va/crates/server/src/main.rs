use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use metrics::{
    MetricBatch, MetricsBatcher, MetricsConfig, MetricsContext, MetricsHub, MetricsSubscription,
    Stage, SysinfoResourceSampler,
};
use serde::{Deserialize, Serialize};
use va_core::{analyze_with_metrics, Engine, RuleBasedIncidentAnalyzer};

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
    #[arg(long, env = "PHEME_VA_METRICS_ENABLED", default_value_t = false)]
    metrics_enabled: bool,
    #[arg(long, env = "PHEME_VA_INCIDENT_METRICS", default_value_t = false)]
    incident_metrics: bool,
    #[arg(long, env = "PHEME_VA_RESOURCE_SAMPLING", default_value_t = false)]
    resource_sampling: bool,
}

#[derive(Clone)]
struct AppState {
    engine: Arc<Mutex<Engine>>,
    metrics_hub: Arc<MetricsHub>,
    metrics_batcher: Arc<MetricsBatcher>,
    _metrics_subscription: Arc<MetricsSubscription>,
    metrics_config: MetricsConfig,
    resource_sampler: Arc<Mutex<SysinfoResourceSampler>>,
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
    let metrics_hub = Arc::new(MetricsHub::new());
    let metrics_batcher = Arc::new(MetricsBatcher::new());
    let metrics_subscription = Arc::new(metrics_hub.subscribe(Arc::clone(&metrics_batcher)));
    let state = AppState {
        engine: Arc::new(Mutex::new(engine)),
        metrics_hub,
        metrics_batcher,
        _metrics_subscription: metrics_subscription,
        metrics_config: MetricsConfig {
            enabled: args.metrics_enabled,
            incident_active: args.incident_metrics,
            resource_sampling: args.resource_sampling,
        },
        resource_sampler: Arc::new(Mutex::new(SysinfoResourceSampler::new())),
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
        .route("/v1/metrics/batches", get(metrics_batches))
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
    use va_core::{DictionaryHints, EngineConfig, RuleBasedFormatter};
    use whispercpp::{WhisperConfig, WhisperTranscriber};

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

    let metrics = request_metrics(&state, &headers, false);
    let metrics_for_sampling = metrics.clone();
    let resource_sampler = Arc::clone(&state.resource_sampler);
    let engine = Arc::clone(&state.engine);
    let result = tokio::task::spawn_blocking(move || {
        sample_resources(&resource_sampler, &metrics_for_sampling);
        let result = (|| {
            let mut engine = engine
                .lock()
                .map_err(|_| anyhow!("engine lock was poisoned"))?;
            let audio = va_core::AudioBuffer::from_wav(&body)
                .map_err(|error| anyhow!(error.to_string()))?;
            engine
                .transcribe_with_metrics(audio, metrics)
                .map_err(|error| anyhow!(error.to_string()))
        })();
        sample_resources(&resource_sampler, &metrics_for_sampling);
        result
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

async fn analyze(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AnalyzeRequest>,
) -> Response {
    let transcript = request.transcript.trim();
    if transcript.is_empty() || transcript.chars().count() > 16_000 {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "A non-empty transcript of at most 16000 characters is required.",
        );
    }
    let metrics = request_metrics(&state, &headers, true);
    sample_resources(&state.resource_sampler, &metrics);
    let request_timer = metrics.start_stage(Stage::EndToEndRequest);
    let mut analyzer = RuleBasedIncidentAnalyzer;
    let result = analyze_with_metrics(&mut analyzer, transcript, &metrics);
    let response = match result {
        Ok(report) => {
            metrics.record_workflow_outcome(true);
            Json(report).into_response()
        }
        Err(_) => {
            metrics.record_workflow_outcome(false);
            api_error(
                StatusCode::BAD_GATEWAY,
                "invalid_model_response",
                "Analysis runtime returned an invalid response.",
            )
        }
    };
    sample_resources(&state.resource_sampler, &metrics);
    request_timer.finish();
    response
}

async fn metrics_batches(State(state): State<AppState>) -> Json<Vec<MetricBatch>> {
    Json(state.metrics_batcher.drain())
}

fn sample_resources(sampler: &Arc<Mutex<SysinfoResourceSampler>>, metrics: &MetricsContext) {
    if let Ok(mut sampler) = sampler.lock() {
        metrics.sample_resources(&mut *sampler);
    }
}

fn request_metrics(state: &AppState, headers: &HeaderMap, incident_route: bool) -> MetricsContext {
    let mut config = state.metrics_config.clone();
    if let Some(active) = header_bool(headers, "x-incident-active") {
        config.incident_active = active;
    } else if incident_route {
        config.incident_active = true;
    }

    MetricsContext::new(
        header_string(headers, "x-run-id").unwrap_or_else(new_run_id),
        header_string(headers, "x-experiment-id"),
        config,
        Arc::clone(&state.metrics_hub),
    )
}

fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn header_bool(headers: &HeaderMap, name: &str) -> Option<bool> {
    header_string(headers, name).and_then(|value| match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    })
}

fn new_run_id() -> String {
    static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(0);
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!(
        "run_{timestamp_ms}_{}",
        NEXT_RUN_ID.fetch_add(1, Ordering::Relaxed)
    )
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
