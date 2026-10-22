use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use metrics::{MetricsBatcher, MetricsConfig, MetricsContext, MetricsHub, MetricsSubscription};
use serde::Serialize;
use va_core::{AudioBuffer, Engine, TranscriptionResult};

use crate::config::{Configuration, SCHEMA_VERSION};
use crate::{BridgeError, BACKEND_ERROR, BUSY, CLOSED, PANIC, UNAVAILABLE};

pub(crate) const METRIC_CAPACITY: usize = 512;
type Loader = fn(&Configuration) -> Result<Engine, BridgeError>;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Unavailable,
    Idle,
    Loading,
    Running,
    Closing,
    Closed,
}

pub(crate) struct State {
    pub phase: Phase,
    pub engine: Option<Engine>,
    pub config: Option<Configuration>,
    pub active_run: Option<String>,
    pub last_failure: Option<BridgeError>,
}

pub struct PhemeVaHandle {
    pub(crate) state: Mutex<State>,
    pub(crate) batcher: Arc<MetricsBatcher>,
    hub: Arc<MetricsHub>,
    _subscription: MetricsSubscription,
    loader: Loader,
}

pub struct PhemeVaObserver {
    pub(crate) batcher: Arc<MetricsBatcher>,
}

#[derive(Serialize)]
pub(crate) struct Snapshot {
    schema_version: u32,
    state: Phase,
    ready: bool,
    capabilities: Capabilities,
    configuration: Option<Configuration>,
    active_run: Option<String>,
    last_failure: Option<BridgeError>,
}

#[derive(Serialize)]
struct Capabilities {
    whisper: bool,
    whisper_metal: bool,
    whisper_coreml: bool,
    metrics_output: bool,
}

#[derive(Serialize)]
pub(crate) struct RunResult {
    pub schema_version: u32,
    pub run_id: String,
    pub result: TranscriptionResult,
}

impl PhemeVaHandle {
    pub(crate) fn new() -> Self {
        Self::with_loader(load_engine)
    }

    pub(crate) fn with_loader(loader: Loader) -> Self {
        let hub = Arc::new(MetricsHub::new());
        let batcher = Arc::new(MetricsBatcher::with_capacity(METRIC_CAPACITY));
        let subscription = hub.subscribe(Arc::clone(&batcher));
        Self {
            state: Mutex::new(State {
                phase: Phase::Unavailable,
                engine: None,
                config: None,
                active_run: None,
                last_failure: None,
            }),
            batcher,
            hub,
            _subscription: subscription,
            loader,
        }
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let state = self.lock();
        Snapshot {
            schema_version: SCHEMA_VERSION,
            state: state.phase,
            ready: state.phase == Phase::Idle,
            capabilities: Capabilities {
                whisper: cfg!(feature = "whisper"),
                whisper_metal: cfg!(feature = "whisper-metal"),
                whisper_coreml: cfg!(feature = "whisper-coreml"),
                metrics_output: true,
            },
            configuration: state.config.clone(),
            active_run: state.active_run.clone(),
            last_failure: state.last_failure.clone(),
        }
    }

    pub(crate) fn configure(&self, configuration: Configuration) -> Result<(), BridgeError> {
        let configuration = configuration.validate()?;
        let mut state = self.lock();
        ensure_idle(&state)?;
        let reload = state.engine.is_none()
            || state.config.as_ref().map_or(true, |old| {
                old.model_path != configuration.model_path || old.threads != configuration.threads
            });
        if !reload {
            *state.engine.as_mut().expect("checked engine").config_mut() =
                configuration.engine_config();
            state.config = Some(configuration);
            state.last_failure = None;
            return Ok(());
        }
        state.phase = Phase::Loading;
        let old_engine = state.engine.take();
        // The effective configuration is absent until a replacement loads successfully.
        state.config = None;
        state.last_failure = None;
        drop(state);
        let mut lease = Lease {
            handle: self,
            engine: None,
            run_id: None,
            metrics_enabled: false,
        };
        drop(old_engine);
        match (self.loader)(&configuration) {
            Ok(engine) => {
                lease.engine = Some(engine);
                self.lock().config = Some(configuration);
                Ok(())
            }
            Err(error) => {
                self.lock().last_failure = Some(error.clone());
                Err(error)
            }
        }
    }

    pub(crate) fn begin_run(&self) -> Result<Lease<'_>, BridgeError> {
        let mut state = self.lock();
        ensure_idle(&state)?;
        let engine = state
            .engine
            .take()
            .ok_or_else(|| BridgeError::new(UNAVAILABLE, "configure and load a model first"))?;
        let run_id = next_run_id();
        state.phase = Phase::Running;
        state.active_run = Some(run_id.clone());
        state.last_failure = None;
        let metrics_enabled = state.config.as_ref().is_some_and(|c| c.metrics_enabled);
        Ok(Lease {
            handle: self,
            engine: Some(engine),
            run_id: Some(run_id),
            metrics_enabled,
        })
    }

    pub(crate) fn close(&self) -> Result<(), BridgeError> {
        let mut state = self.lock();
        if matches!(
            state.phase,
            Phase::Running | Phase::Loading | Phase::Closing
        ) {
            state.phase = Phase::Closing;
            return Err(BridgeError::new(
                BUSY,
                "closing; wait for outstanding calls before freeing",
            ));
        }
        state.phase = Phase::Closed;
        state.config = None;
        let engine = state.engine.take();
        drop(state);
        drop(engine);
        Ok(())
    }
}

/// Owns the engine while a blocking operation runs, leaving snapshots/drains responsive.
pub(crate) struct Lease<'a> {
    handle: &'a PhemeVaHandle,
    engine: Option<Engine>,
    run_id: Option<String>,
    metrics_enabled: bool,
}

impl Lease<'_> {
    pub(crate) fn run_id(&self) -> Option<String> {
        self.run_id.clone()
    }

    pub(crate) fn max_audio_seconds(&self) -> u32 {
        self.engine
            .as_ref()
            .expect("run engine")
            .config()
            .max_audio_seconds
            .unwrap_or(120)
    }

    pub(crate) fn transcribe(&mut self, audio: AudioBuffer) -> Result<RunResult, BridgeError> {
        let run_id = self.run_id.clone().expect("admitted run");
        let context = MetricsContext::new(
            run_id.clone(),
            None,
            MetricsConfig {
                enabled: self.metrics_enabled,
                incident_active: false,
                resource_sampling: false,
            },
            Arc::clone(&self.handle.hub),
        );
        let result = self
            .engine
            .as_mut()
            .expect("run engine")
            .transcribe_with_metrics(audio, context)
            .map_err(|error| {
                BridgeError::new(BACKEND_ERROR, error.to_string()).with_run(run_id.clone())
            })?;
        Ok(RunResult {
            schema_version: SCHEMA_VERSION,
            run_id,
            result,
        })
    }

    pub(crate) fn failure(&self, mut error: BridgeError) -> BridgeError {
        error.run_id.clone_from(&self.run_id);
        self.handle.lock().last_failure = Some(error.clone());
        error
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        let mut state = self.handle.lock();
        state.active_run = None;
        if std::thread::panicking() {
            state.last_failure = Some(
                BridgeError::new(PANIC, "Rust operation panicked; reload the model")
                    .with_optional_run(self.run_id.clone()),
            );
            state.config = None;
            state.phase = if state.phase == Phase::Closing {
                Phase::Closed
            } else {
                Phase::Unavailable
            };
        } else if state.phase == Phase::Closing {
            state.config = None;
            state.phase = Phase::Closed;
        } else {
            state.engine = self.engine.take();
            state.phase = if state.engine.is_some() {
                Phase::Idle
            } else {
                Phase::Unavailable
            };
        }
        // Drop a closed/panicked model outside the snapshot lock.
        drop(state);
    }
}

pub(crate) fn ensure_idle(state: &State) -> Result<(), BridgeError> {
    match state.phase {
        Phase::Loading | Phase::Running => Err(BridgeError::new(BUSY, "engine is busy")),
        Phase::Closing | Phase::Closed => {
            Err(BridgeError::new(CLOSED, "engine is closing or closed"))
        }
        _ => Ok(()),
    }
}

#[cfg(feature = "whisper")]
fn load_engine(config: &Configuration) -> Result<Engine, BridgeError> {
    let adapter = whispercpp::WhisperTranscriber::from_file(
        &config.model_path,
        whispercpp::WhisperConfig {
            threads: config.threads,
            ..whispercpp::WhisperConfig::default()
        },
    )
    .map_err(|error| BridgeError::new(BACKEND_ERROR, error.to_string()))?;
    Ok(Engine::with_config(adapter, config.engine_config()))
}

#[cfg(not(feature = "whisper"))]
fn load_engine(_config: &Configuration) -> Result<Engine, BridgeError> {
    Err(BridgeError::new(
        crate::UNSUPPORTED,
        "this library was built without Whisper",
    ))
}

fn next_run_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("ffi_{timestamp}_{}", NEXT.fetch_add(1, Ordering::Relaxed))
}
