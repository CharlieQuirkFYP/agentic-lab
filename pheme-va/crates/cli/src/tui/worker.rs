use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use metrics::{MetricSample, MetricScope, MetricUnit, MetricsConfig, MetricsContext, MetricsHub};
use va_core::{AudioBuffer, Engine};

use crate::model;

use super::events::{ModelLoadRequest, RequestId, RunId, WorkerCommand, WorkerEvent};
use super::logs::{LogEntry, LogLevel};

pub struct WorkerConfig {
    pub metrics_enabled: bool,
    pub resource_sampling_enabled: bool,
    pub resource_context: Arc<Mutex<Option<MetricsContext>>>,
}

pub fn spawn_resource_sampler(
    context: Arc<Mutex<Option<MetricsContext>>>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("pheme-va-tui-resource-sampler".to_owned())
        .spawn(move || {
            let mut collector =
                metrics::ResourceCollector::new(metrics::SysinfoResourceSampler::new());
            while !stop.load(Ordering::Relaxed) {
                let active_context = context.lock().ok().and_then(|guard| guard.clone());
                if let Some(active_context) = active_context {
                    collector.sample_and_record(&active_context);
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        })
        .expect("could not spawn Pheme VA TUI resource sampler")
}

pub fn spawn(
    receiver: Receiver<WorkerCommand>,
    sender: Sender<WorkerEvent>,
    metrics_hub: Arc<MetricsHub>,
    config: WorkerConfig,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("pheme-va-tui-worker".to_owned())
        .spawn(move || {
            let mut worker = Worker::new(sender, metrics_hub, config);
            worker.run(receiver);
        })
        .expect("could not spawn Pheme VA TUI worker")
}

struct Worker {
    sender: Sender<WorkerEvent>,
    metrics_hub: Arc<MetricsHub>,
    metrics_enabled: bool,
    resource_sampling_enabled: bool,
    engine: Option<Engine>,
    active_model_id: Option<String>,
    resource_context: Arc<Mutex<Option<MetricsContext>>>,
    idle_metrics: MetricsContext,
    active_metrics: Option<MetricsContext>,
}

impl Worker {
    fn new(
        sender: Sender<WorkerEvent>,
        metrics_hub: Arc<MetricsHub>,
        config: WorkerConfig,
    ) -> Self {
        Self {
            sender,
            metrics_hub: Arc::clone(&metrics_hub),
            metrics_enabled: config.metrics_enabled,
            resource_sampling_enabled: config.resource_sampling_enabled,
            engine: None,
            active_model_id: None,
            resource_context: config.resource_context,
            idle_metrics: MetricsContext::new(
                "idle",
                None,
                MetricsConfig {
                    enabled: config.metrics_enabled,
                    incident_active: false,
                    resource_sampling: config.resource_sampling_enabled,
                },
                Arc::clone(&metrics_hub),
            ),
            active_metrics: None,
        }
    }

    fn run(&mut self, receiver: Receiver<WorkerCommand>) {
        self.set_resource_context(Some(self.idle_metrics.clone()));
        self.emit_log(LogEntry::info("worker", "inference worker started"));
        while let Ok(command) = receiver.recv() {
            match command {
                WorkerCommand::LoadModel(request) => self.load_model(request),
                WorkerCommand::BeginRun { run_id } => self.begin_run(run_id),
                WorkerCommand::EndRun { run_id } => self.end_run(&run_id),
                WorkerCommand::TranscribeWav {
                    request_id,
                    run_id,
                    path,
                } => self.transcribe_wav(request_id, run_id, path),
                WorkerCommand::TranscribeAudio {
                    request_id,
                    run_id,
                    source,
                    audio,
                } => self.transcribe_audio(request_id, run_id, source, audio),
                WorkerCommand::Shutdown => break,
            }
        }
        self.emit_log(LogEntry::info("worker", "inference worker stopped"));
        let _ = self.sender.send(WorkerEvent::WorkerStopped);
    }

    fn begin_run(&mut self, run_id: RunId) {
        let metrics = self.context(&run_id);
        self.active_metrics = Some(metrics.clone());
        self.set_resource_context(Some(metrics));
    }

    fn end_run(&mut self, run_id: &str) {
        if self
            .active_metrics
            .as_ref()
            .is_some_and(|metrics| metrics.run_id() == run_id)
        {
            self.active_metrics = None;
            self.set_resource_context(Some(self.idle_metrics.clone()));
        }
    }

    fn restore_resource_monitor(&self) {
        self.set_resource_context(Some(
            self.active_metrics
                .clone()
                .unwrap_or_else(|| self.idle_metrics.clone()),
        ));
    }

    fn load_model(&mut self, request: ModelLoadRequest) {
        let ModelLoadRequest {
            request_id,
            model_id,
            manifest_path,
            max_seconds,
            language,
            dictionary,
            switch_from,
        } = request;
        let operation_id = format!("model-load-{request_id}");
        let metrics = self.context(&operation_id);
        let started = Instant::now();
        self.emit(WorkerEvent::ModelLoadStarted {
            request_id,
            model_id: model_id.clone(),
            switch_from: switch_from.clone(),
        });
        self.emit_log(LogEntry::new(
            LogLevel::Info,
            "model-loader",
            format!("loading model {model_id}"),
            None,
            Some(model_id.clone()),
        ));
        self.set_resource_context(Some(metrics.clone()));
        let candidate =
            model::create_engine(&model_id, &manifest_path, max_seconds, language, dictionary);
        self.restore_resource_monitor();
        let duration_ms = started.elapsed().as_millis();
        self.record_model_load_metrics(
            &metrics,
            duration_ms,
            candidate.is_ok(),
            switch_from.as_deref(),
            &model_id,
        );

        match candidate {
            Ok(candidate) if !candidate.is_ready() => {
                let error = "candidate engine reported not ready".to_owned();
                self.emit_model_load_failure(request_id, model_id, error, switch_from);
            }
            Ok(candidate) => {
                let family = candidate.model_family().to_owned();
                let backend = candidate.backend_name().to_owned();
                let old_model = self.active_model_id.clone();
                self.engine = Some(candidate);
                self.active_model_id = Some(model_id.clone());
                self.emit_log(LogEntry::new(
                    LogLevel::Info,
                    "model-loader",
                    format!("model {model_id} is ready"),
                    None,
                    Some(model_id.clone()),
                ));
                if old_model.is_some() {
                    self.emit_log(LogEntry::new(
                        LogLevel::Info,
                        "model-loader",
                        "replacement engine committed",
                        None,
                        Some(model_id.clone()),
                    ));
                }
                self.emit(WorkerEvent::ModelReady {
                    request_id,
                    model_id,
                    model_family: family,
                    backend,
                    load_duration_ms: duration_ms,
                    switch_from,
                });
            }
            Err(error) => {
                self.emit_model_load_failure(request_id, model_id, error.to_string(), switch_from);
            }
        }
    }

    fn emit_model_load_failure(
        &self,
        request_id: RequestId,
        model_id: String,
        error: String,
        switch_from: Option<String>,
    ) {
        self.emit_log(LogEntry::new(
            LogLevel::Error,
            "model-loader",
            format!("model load failed: {error}"),
            None,
            Some(model_id.clone()),
        ));
        self.emit(WorkerEvent::ModelLoadFailed {
            request_id,
            model_id,
            error,
            switch_from,
        });
    }

    fn transcribe_wav(&mut self, request_id: RequestId, run_id: RunId, path: PathBuf) {
        let source = path.display().to_string();
        self.emit(WorkerEvent::ProcessingStarted {
            request_id,
            run_id: run_id.clone(),
            source: source.clone(),
        });
        self.emit_log(LogEntry::new(
            LogLevel::Info,
            "worker",
            format!("reading WAV {source}"),
            Some(run_id.clone()),
            self.active_model_id.clone(),
        ));
        let result = std::fs::read(&path)
            .with_context(|| format!("could not read {}", path.display()))
            .and_then(|bytes| {
                AudioBuffer::from_wav(&bytes)
                    .with_context(|| format!("could not decode WAV {}", path.display()))
            });
        match result {
            Ok(audio) => self.transcribe_audio(request_id, run_id, source, audio),
            Err(error) => {
                self.emit_processing_failure(request_id, run_id.clone(), source, error.to_string());
                self.end_run(&run_id);
            }
        }
    }

    fn transcribe_audio(
        &mut self,
        request_id: RequestId,
        run_id: RunId,
        source: String,
        audio: AudioBuffer,
    ) {
        if self.engine.is_none() {
            self.emit_processing_failure(
                request_id,
                run_id.clone(),
                source,
                "no speech model is active".to_owned(),
            );
            self.end_run(&run_id);
            return;
        }

        let audio_duration_seconds = audio.duration_seconds();
        self.emit_log(LogEntry::new(
            LogLevel::Info,
            "worker",
            "transcription started",
            Some(run_id.clone()),
            self.active_model_id.clone(),
        ));
        let metrics = self
            .active_metrics
            .clone()
            .unwrap_or_else(|| self.context(&run_id));
        self.set_resource_context(Some(metrics.clone()));
        let result = self
            .engine
            .as_mut()
            .expect("engine checked above")
            .transcribe_with_metrics(audio, metrics);
        self.restore_resource_monitor();

        match result {
            Ok(result) => {
                self.emit_log(LogEntry::new(
                    LogLevel::Info,
                    "worker",
                    format!(
                        "transcription completed with status {}",
                        result.status.as_str()
                    ),
                    Some(run_id.clone()),
                    Some(result.model_id.clone()),
                ));
                self.emit(WorkerEvent::Result {
                    request_id,
                    run_id: run_id.clone(),
                    source,
                    audio_duration_seconds,
                    result: Box::new(result),
                });
                self.end_run(&run_id);
            }
            Err(error) => {
                self.emit_processing_failure(request_id, run_id.clone(), source, error.to_string());
                self.end_run(&run_id);
            }
        }
    }

    fn emit_processing_failure(
        &self,
        request_id: RequestId,
        run_id: RunId,
        source: String,
        error: String,
    ) {
        self.emit_log(LogEntry::new(
            LogLevel::Error,
            "worker",
            format!("transcription failed: {error}"),
            Some(run_id.clone()),
            self.active_model_id.clone(),
        ));
        self.emit(WorkerEvent::ProcessingFailed {
            request_id,
            run_id,
            source,
            error,
        });
    }

    fn context(&self, run_id: &str) -> MetricsContext {
        MetricsContext::new(
            run_id,
            None,
            MetricsConfig {
                enabled: self.metrics_enabled,
                incident_active: false,
                resource_sampling: self.resource_sampling_enabled,
            },
            Arc::clone(&self.metrics_hub),
        )
    }

    fn set_resource_context(&self, context: Option<MetricsContext>) {
        if let Ok(mut active_context) = self.resource_context.lock() {
            *active_context = context;
        }
    }

    fn record_model_load_metrics(
        &self,
        metrics: &MetricsContext,
        duration_ms: u128,
        success: bool,
        switch_from: Option<&str>,
        switch_to: &str,
    ) {
        metrics.record(MetricSample::number(
            "model_load_duration_ms",
            duration_ms as f64,
            MetricUnit::Milliseconds,
            MetricScope::Process,
            "cli.model_loader",
        ));
        metrics.record(MetricSample::text(
            "model_load_outcome",
            if success { "success" } else { "failure" },
            MetricUnit::Status,
            MetricScope::Process,
            "cli.model_loader",
        ));
        if let Some(previous) = switch_from {
            metrics.record(MetricSample::number(
                "model_switch_duration_ms",
                duration_ms as f64,
                MetricUnit::Milliseconds,
                MetricScope::Process,
                "cli.model_loader",
            ));
            metrics.record(MetricSample::text(
                "model_switch_outcome",
                if success { "success" } else { "failure" },
                MetricUnit::Status,
                MetricScope::Process,
                "cli.model_loader",
            ));
            metrics.record(MetricSample::text(
                "model_switch_from",
                previous,
                MetricUnit::Status,
                MetricScope::Process,
                "cli.model_loader",
            ));
            metrics.record(MetricSample::text(
                "model_switch_to",
                switch_to,
                MetricUnit::Status,
                MetricScope::Process,
                "cli.model_loader",
            ));
        }
    }

    fn emit(&self, event: WorkerEvent) {
        let _ = self.sender.send(event);
    }

    fn emit_log(&self, entry: LogEntry) {
        self.emit(WorkerEvent::Log(entry));
    }
}
