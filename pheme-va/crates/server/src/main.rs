mod startup;
mod voice;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
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
use va_core::{analyze_with_metrics, RuleBasedIncidentAnalyzer};

#[derive(Debug, Parser)]
#[command(name = "server", about = "HTTP wrapper around the Pheme VA core")]
struct Args {
    #[arg(long, env = "PHEME_VA_WHISPER_MODEL", conflicts_with = "stt_model")]
    model: Option<PathBuf>,
    #[arg(long, conflicts_with = "model")]
    stt_model: Option<String>,
    #[arg(long, conflicts_with = "reply_path")]
    reply_model: Option<String>,
    #[arg(long, env = "PHEME_VA_REPLY_MODEL", conflicts_with = "reply_model")]
    reply_path: Option<PathBuf>,
    #[arg(long, default_value = "models/manifest.toml")]
    model_manifest: PathBuf,
    #[arg(long)]
    startup_choices: Option<PathBuf>,
    #[arg(long, conflicts_with = "prompt_files")]
    system_prompt: Option<PathBuf>,
    /// Trusted local role files, composed in flag order on this startup only.
    /// Relative paths use the host working directory, like --system-prompt.
    #[arg(long = "prompt-file", action = clap::ArgAction::Append, conflicts_with = "system_prompt")]
    prompt_files: Vec<PathBuf>,
    #[arg(long, default_value_t = 4)]
    threads: i32,
    #[arg(long, default_value_t = 0)]
    gpu_layers: u32,
    #[arg(long, default_value_t = 4096)]
    context_size: u32,
    #[arg(long, default_value_t = 512)]
    max_output_tokens: u32,
    #[arg(long, default_value_t = 0.2)]
    temperature: f32,
    #[arg(long, default_value_t = 12000)]
    max_input_chars: usize,
    #[arg(long, default_value_t = 4000)]
    max_output_chars: usize,
    #[arg(long, default_value_t = 6)]
    max_history_turns: usize,
    #[arg(long, default_value_t = 300)]
    max_review_seconds: u64,
    #[arg(long, default_value_t = 120)]
    max_generation_seconds: u64,
    #[arg(long, default_value_t = 180)]
    max_transcription_seconds: u64,
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
    runtime: va_runtime::AgentRuntime,
    metrics_batcher: Arc<MetricsBatcher>,
    _metrics_subscription: Arc<MetricsSubscription>,
}
impl std::ops::Deref for AppState {
    type Target = va_runtime::AgentRuntime;
    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}
impl std::ops::DerefMut for AppState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.runtime
    }
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
    let started = std::time::Instant::now();
    let runtimes = startup::load(&args)?;
    let metrics_hub = Arc::new(MetricsHub::new());
    let metrics_batcher = Arc::new(MetricsBatcher::new());
    let metrics_subscription = Arc::new(metrics_hub.subscribe(Arc::clone(&metrics_batcher)));
    let runtime = va_runtime::AgentRuntime::new(
        runtimes.engine,
        runtimes.reply,
        runtimes.prompt,
        voice::Limits {
            conversation: runtimes.conversation,
            review_timeout: std::time::Duration::from_secs(args.max_review_seconds),
            generation_timeout: std::time::Duration::from_secs(args.max_generation_seconds),
            transcription_timeout: std::time::Duration::from_secs(args.max_transcription_seconds),
            ..Default::default()
        },
        MetricsConfig {
            enabled: args.metrics_enabled,
            incident_active: args.incident_metrics,
            resource_sampling: args.resource_sampling,
        },
        metrics_hub,
    );
    let state = AppState {
        runtime,
        metrics_batcher,
        _metrics_subscription: metrics_subscription,
    };
    let startup_metrics = request_metrics(&state, &HeaderMap::new(), false);
    startup_metrics.record_model_timing(
        "stt_model_load_ms",
        runtimes.stt_load_ms,
        "server.startup",
    );
    startup_metrics.record_model_timing(
        "reply_model_load_ms",
        runtimes.reply_load_ms,
        "server.startup",
    );
    startup_metrics.record_model_timing(
        "server_startup_ms",
        Some(started.elapsed().as_secs_f64() * 1000.0),
        "server.startup",
    );
    let address: std::net::SocketAddr = args
        .bind
        .parse()
        .with_context(|| format!("invalid bind address {}", args.bind))?;
    if !address.ip().is_loopback() {
        eprintln!("WARNING: voice inspection/mutations expose a single development conversation; protect this listener with authentication and TLS at the Go/reverse-proxy boundary.");
    }
    let shutdown_state = state.clone();
    let app = router(state);

    println!("server listening on http://{address}");
    println!("POST audio/wav to /v1/transcribe");
    axum::serve(tokio::net::TcpListener::bind(address).await?, app)
        .with_graceful_shutdown(shutdown(shutdown_state))
        .await
        .context("Pheme VA server stopped")?;
    Ok(())
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/v1/transcribe", post(transcribe))
        .route("/v1/analyze", post(analyze))
        .route("/v1/metrics/batches", get(metrics_batches))
        .merge(voice::routes())
        .layer(DefaultBodyLimit::max(state.voice.limits.max_body_bytes))
        .with_state(state)
}

async fn shutdown(state: AppState) {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
    state.voice.shutdown().await;
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn ready(State(state): State<AppState>) -> Response {
    let ready = state.voice.stt_status().ready;
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

    let audio = match va_core::AudioBuffer::from_wav(&body) {
        Ok(audio) => audio,
        Err(error) => return map_engine_error(&error.to_string()),
    };
    let context = request_metrics(&state, &headers, false);
    match state.runtime.transcribe(audio, context).await {
        Ok(result) => Json(result).into_response(),
        Err(error) => voice::HttpError::from(error).into_response(),
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
    va_runtime::new_run_id()
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

#[cfg(test)]
mod args_tests {
    use super::*;

    #[test]
    fn repeatable_prompt_files_preserve_flag_order_and_host_relative_paths() {
        let args = Args::try_parse_from([
            "server",
            "--reply-model",
            "reply",
            "--prompt-file",
            "roles/second.txt",
            "--prompt-file",
            "roles/first.txt",
        ])
        .unwrap();
        assert_eq!(
            args.prompt_files,
            vec![
                PathBuf::from("roles/second.txt"),
                PathBuf::from("roles/first.txt")
            ]
        );
        assert!(args.system_prompt.is_none());
    }

    #[test]
    fn prompt_file_and_legacy_system_prompt_flags_are_mutually_exclusive() {
        for flags in [
            vec![
                "server",
                "--prompt-file",
                "first.txt",
                "--system-prompt",
                "second.txt",
            ],
            vec![
                "server",
                "--system-prompt",
                "first.txt",
                "--prompt-file",
                "second.txt",
            ],
            vec!["server", "--prompt-file"],
        ] {
            assert!(Args::try_parse_from(flags).is_err());
        }
        let args = Args::try_parse_from([
            "server",
            "--reply-path",
            "local.gguf",
            "--system-prompt",
            "role.txt",
        ])
        .unwrap();
        assert_eq!(args.system_prompt, Some(PathBuf::from("role.txt")));
        assert!(args.prompt_files.is_empty());
    }
}
