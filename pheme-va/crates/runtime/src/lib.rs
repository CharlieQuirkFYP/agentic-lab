//! In-process agent runtime shared by the TUI and the HTTP host.
//! No HTTP server, microphone, terminal, or model downloads live here.
pub mod loader;
pub mod voice;
use metrics::{MetricsConfig, MetricsContext, MetricsHub, SysinfoResourceSampler};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use va_core::{ConversationModel, Engine, LoadedPrompt};
pub use voice::{conversations, events, Limits, RuntimeError, RuntimeStatus, TurnInput};

#[derive(Clone, Copy, Debug)]
pub enum ErrorKind {
    InvalidInput,
    Conflict,
    NotFound,
    Expired,
    Unavailable,
    Capacity,
    Timeout,
    Internal,
}
#[derive(Clone, Default)]
pub struct RequestOptions {
    pub run_id: Option<String>,
    pub experiment_id: Option<String>,
    pub incident_active: Option<bool>,
    pub idempotency_key: Option<String>,
    pub audio_source: String,
}
#[derive(Clone)]
pub struct AgentRuntime {
    pub engine: Arc<Mutex<Option<Engine>>>,
    pub voice: Arc<voice::Voice>,
    pub metrics_hub: Arc<MetricsHub>,
    pub metrics_config: MetricsConfig,
    pub resource_sampler: Arc<Mutex<SysinfoResourceSampler>>,
}
impl AgentRuntime {
    pub fn new(
        engine: Option<Engine>,
        reply: Option<Box<dyn ConversationModel>>,
        prompt: Option<LoadedPrompt>,
        limits: Limits,
        config: MetricsConfig,
        hub: Arc<MetricsHub>,
    ) -> Self {
        let voice = voice::Voice::new(engine.as_ref(), reply, prompt, limits);
        Self {
            engine: Arc::new(Mutex::new(engine)),
            voice,
            metrics_hub: hub,
            metrics_config: config,
            resource_sampler: Arc::new(Mutex::new(SysinfoResourceSampler::new())),
        }
    }
    pub fn load_engine(
        &self,
        load: impl FnOnce() -> anyhow::Result<Engine>,
    ) -> anyhow::Result<(String, String)> {
        anyhow::ensure!(
            self.voice.idle(),
            "Local runtime is busy; wait for native work before loading a model."
        );
        let _lease = self.voice.begin_inference()?;
        let engine = load()?;
        anyhow::ensure!(engine.is_ready(), "candidate engine reported not ready");
        let info = (
            engine.model_family().to_owned(),
            engine.backend_name().to_owned(),
        );
        let status = RuntimeStatus {
            name: engine.backend_name().to_owned(),
            ready: true,
        };
        *self.engine.lock().unwrap_or_else(|p| p.into_inner()) = Some(engine);
        *self.voice.stt.lock().unwrap_or_else(|p| p.into_inner()) = status;
        Ok(info)
    }
    pub async fn shutdown(&self) {
        self.voice.shutdown().await;
    }
    pub async fn transcribe(
        &self,
        audio: va_core::AudioBuffer,
        context: MetricsContext,
    ) -> Result<va_core::TranscriptionResult, RuntimeError> {
        if !self.voice.stt_status().ready {
            return Err(voice::RuntimeError(
                ErrorKind::Unavailable,
                voice::Failure::new(
                    "runtime_unavailable",
                    "Transcription runtime is unavailable.",
                ),
            ));
        }
        let lease = self.voice.begin_inference()?;
        let engine = self.engine.clone();
        let sampler = self.resource_sampler.clone();
        let sampling = conversations::sample_during(self, &context);
        let task = tokio::task::spawn_blocking(move || {
            let _lease = lease;
            let _sampling = sampling;
            sample_resources(&sampler, &context);
            let result = engine
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_mut()
                .ok_or_else(|| "Transcription runtime is unavailable.".to_owned())?
                .transcribe_with_metrics(audio, context.clone())
                .map_err(|e| e.to_string());
            sample_resources(&sampler, &context);
            result
        });
        // Dropping this waiter never releases native work's lease or sampler.
        let result = tokio::time::timeout(self.voice.limits.transcription_timeout, task)
            .await
            .map_err(|_| {
                voice::RuntimeError(
                    ErrorKind::Timeout,
                    voice::Failure::new(
                        "transcription_timeout",
                        "Transcription timed out; runtime remains busy until native work settles.",
                    ),
                )
            })?;
        result
            .map_err(|_| {
                voice::RuntimeError(
                    ErrorKind::Internal,
                    voice::Failure::new("transcription_failed", "Transcription failed."),
                )
            })?
            .map_err(|message| {
                voice::RuntimeError(
                    ErrorKind::InvalidInput,
                    voice::Failure::new(
                        if message.contains("duration exceeds") {
                            "audio_too_long"
                        } else {
                            "transcription_failed"
                        },
                        "Transcription failed; check the audio and runtime.",
                    ),
                )
            })
    }
}
pub fn request_metrics(
    state: &AgentRuntime,
    options: &RequestOptions,
    incident: bool,
) -> MetricsContext {
    let mut config = state.metrics_config.clone();
    config.incident_active = options
        .incident_active
        .unwrap_or(incident || config.incident_active);
    MetricsContext::new(
        options.run_id.clone().unwrap_or_else(new_run_id),
        options.experiment_id.clone(),
        config,
        state.metrics_hub.clone(),
    )
}
pub fn sample_resources(sampler: &Arc<Mutex<SysinfoResourceSampler>>, context: &MetricsContext) {
    if let Ok(mut sampler) = sampler.lock() {
        context.sample_resources(&mut *sampler);
    }
}
pub fn new_run_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "run_{}_{}",
        conversations::now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
