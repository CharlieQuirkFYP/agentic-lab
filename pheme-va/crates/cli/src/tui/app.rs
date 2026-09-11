use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;

use anyhow::Result;
use metrics::MetricEvent;
use ratatui::crossterm::event::{Event, KeyCode};
use va_core::{AudioBuffer, TranscriptionResult};

use crate::recorder::Recording;

use super::config::{self, TuiConfig};
use super::events::{ModelLoadRequest, RequestId, WorkerCommand, WorkerEvent};
use super::folder::FolderState;
use super::logs::LogStore;
use super::model_catalog::ModelCatalog;
use super::rebuild::BuildTask;
use super::telemetry::TelemetryStore;
use super::TuiOptions;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    Welcome,
    Picker,
    Loading,
    Bench,
    Folder,
    Recording,
    Processing,
    Telemetry,
    DirectoryInput,
    Help,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelemetryTab {
    Overview,
    Metrics,
    Graphs,
    Logs,
}

impl TelemetryTab {
    pub fn index(self) -> usize {
        match self {
            Self::Overview => 0,
            Self::Metrics => 1,
            Self::Graphs => 2,
            Self::Logs => 3,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ActiveModel {
    pub id: String,
    pub family: String,
    pub backend: String,
    pub runtime: Option<String>,
}

#[derive(Debug)]
pub struct ResultState {
    pub run_id: String,
    pub source: String,
    pub audio_duration_seconds: f32,
    pub result: TranscriptionResult,
}

pub struct App {
    pub screen: Screen,
    pub previous_screen: Screen,
    pub telemetry_tab: TelemetryTab,
    pub config: TuiConfig,
    pub config_path: PathBuf,
    pub catalog: ModelCatalog,
    pub catalog_index: usize,
    pub active_model: Option<ActiveModel>,
    pub pending_model_id: Option<String>,
    pub folder: FolderState,
    pub recording: Option<Recording>,
    pub result: Option<ResultState>,
    pub last_file: Option<PathBuf>,
    pub last_audio: Option<AudioBuffer>,
    pub current_source: Option<String>,
    pub current_run: Option<String>,
    pub current_request: Option<RequestId>,
    pub next_request_id: RequestId,
    pub next_run_id: u64,
    pub status_message: String,
    pub error_message: Option<String>,
    pub telemetry: TelemetryStore,
    pub logs: LogStore,
    pub filter_query: String,
    pub filter_editing: bool,
    pub directory_input: String,
    pub directory_input_error: Option<String>,
    pub scroll: u16,
    pub should_quit: bool,
    pub build: Option<BuildTask>,
    pub restart: Option<(PathBuf, String)>,
    worker_sender: Sender<WorkerCommand>,
    worker_receiver: Receiver<WorkerEvent>,
    metric_receiver: Receiver<MetricEvent>,
    metric_dropped: Arc<AtomicU64>,
    startup_load: bool,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        options: &TuiOptions,
        config: TuiConfig,
        config_loaded: bool,
        catalog: ModelCatalog,
        worker_sender: Sender<WorkerCommand>,
        worker_receiver: Receiver<WorkerEvent>,
        metric_receiver: Receiver<MetricEvent>,
        metric_dropped: Arc<AtomicU64>,
    ) -> Self {
        let selected_index = catalog
            .selected_index(&config.selected_stt_model)
            .unwrap_or(0);
        let has_valid_saved_model = catalog
            .entry_by_id(&config.selected_stt_model)
            .is_some_and(|entry| entry.selectable());
        let onboarding = options.reconfigure
            || (!config_loaded && options.model_id.is_none())
            || catalog.error.is_some();
        let startup_load = !onboarding && (options.model_id.is_some() || has_valid_saved_model);
        let screen = if catalog.error.is_some() {
            Screen::Error
        } else if onboarding {
            Screen::Welcome
        } else if startup_load {
            Screen::Loading
        } else {
            Screen::Picker
        };
        let mut logs = LogStore::default();
        if let Some(error) = catalog.error.as_deref() {
            logs.error("manifest", error);
        } else {
            logs.info(
                "manifest",
                format!(
                    "loaded {} model entr{}",
                    catalog.entries.len(),
                    if catalog.entries.len() == 1 {
                        "y"
                    } else {
                        "ies"
                    }
                ),
            );
        }
        if !config_loaded {
            logs.info("onboarding", "no saved TUI configuration found");
        }
        let directory_input = config.audio_directory.display().to_string();

        Self {
            screen,
            previous_screen: Screen::Bench,
            telemetry_tab: TelemetryTab::Overview,
            folder: FolderState::new(config.audio_directory.clone()),
            config_path: config::config_path(),
            config,
            catalog,
            catalog_index: selected_index,
            active_model: None,
            pending_model_id: None,
            recording: None,
            result: None,
            last_file: None,
            last_audio: None,
            current_source: None,
            current_run: None,
            current_request: None,
            next_request_id: 1,
            next_run_id: 1,
            status_message: "ready".to_owned(),
            error_message: None,
            telemetry: TelemetryStore::default(),
            logs,
            filter_query: String::new(),
            filter_editing: false,
            directory_input,
            directory_input_error: None,
            scroll: 0,
            should_quit: false,
            build: None,
            restart: None,
            worker_sender,
            worker_receiver,
            metric_receiver,
            metric_dropped,
            startup_load,
        }
    }

    pub fn start_initial_load(&mut self) {
        if self.startup_load {
            let model_id = self.config.selected_stt_model.clone();
            self.send_load_model(model_id, None);
            self.startup_load = false;
        }
    }

    pub fn tick(&mut self) {
        self.drain_metrics();
        self.drain_worker_events();
        self.poll_build();
        if self.recording.as_ref().is_some_and(|recording| {
            recording.elapsed().as_secs() >= self.config.max_seconds as u64
        }) {
            self.stop_recording();
            self.status_message = "maximum recording duration reached".to_owned();
        }
    }

    fn poll_build(&mut self) {
        let Some(build) = self.build.as_mut() else {
            return;
        };
        match build.poll() {
            Ok(Some(binary)) => {
                self.restart = Some((binary, build.model_id.clone()));
                self.build.take();
                self.should_quit = true;
            }
            Ok(None) => {}
            Err(error) => {
                self.error_message = Some(format!("{error:#}"));
                self.logs.error("adapter-build", format!("{error:#}"));
                self.build.take();
                self.screen = Screen::Picker;
            }
        }
    }

    pub fn handle_terminal_event(&mut self, event: Event) -> Result<()> {
        if let Event::Key(key) = event {
            if key.kind != ratatui::crossterm::event::KeyEventKind::Release {
                self.handle_key(key.code)?;
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, code: KeyCode) -> Result<()> {
        if self.build.is_some()
            && !matches!(
                self.screen,
                Screen::Loading | Screen::Telemetry | Screen::Help
            )
        {
            self.screen = Screen::Loading;
        }
        if self.filter_editing {
            return self.handle_filter_key(code);
        }

        match code {
            KeyCode::Char('q') => {
                if let Some(recording) = self.recording.take() {
                    recording.discard();
                }
                self.should_quit = true;
                return Ok(());
            }
            KeyCode::Char('?') => {
                self.previous_screen = self.screen;
                self.screen = Screen::Help;
                self.scroll = 0;
                return Ok(());
            }
            KeyCode::Char('t') if self.screen != Screen::Telemetry => {
                self.open_telemetry();
                return Ok(());
            }
            _ => {}
        }

        match self.screen {
            Screen::Welcome => self.handle_welcome_key(code),
            Screen::Picker => self.handle_picker_key(code),
            Screen::Loading => self.handle_loading_key(code),
            Screen::Bench => self.handle_bench_key(code),
            Screen::Folder => self.handle_folder_key(code),
            Screen::Recording => self.handle_recording_key(code),
            Screen::Processing => self.handle_processing_key(code),
            Screen::Telemetry => self.handle_telemetry_key(code),
            Screen::DirectoryInput => self.handle_directory_key(code),
            Screen::Help => self.handle_help_key(code),
            Screen::Error => self.handle_error_key(code),
        }
    }

    fn handle_welcome_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Enter | KeyCode::Char('r') => {
                self.screen = Screen::Picker;
                self.error_message = None;
            }
            KeyCode::Esc => self.should_quit = true,
            _ => {}
        }
        Ok(())
    }

    fn handle_picker_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.move_catalog(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_catalog(1),
            KeyCode::Enter => self.choose_catalog_model(),
            KeyCode::Char('r') => {
                self.catalog.refresh();
                self.catalog_index = self
                    .catalog
                    .selected_index(&self.config.selected_stt_model)
                    .unwrap_or(0);
                self.logs.info("manifest", "rescanned model manifest");
            }
            KeyCode::Esc => {
                if self.active_model.is_some() {
                    self.screen = Screen::Bench;
                } else {
                    self.screen = Screen::Welcome;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_loading_key(&mut self, code: KeyCode) -> Result<()> {
        if self.build.is_some() {
            if code == KeyCode::Esc {
                self.build.take();
                self.logs
                    .info("adapter-build", "build cancelled; current model retained");
                self.screen = Screen::Picker;
            }
            return Ok(());
        }
        match code {
            KeyCode::Char('r') if self.current_request.is_none() => {
                if let Some(model_id) = self.pending_model_id.clone() {
                    self.send_load_model(
                        model_id,
                        self.active_model.as_ref().map(|model| model.id.clone()),
                    );
                }
            }
            KeyCode::Char('m') if self.current_request.is_none() => self.screen = Screen::Picker,
            KeyCode::Esc if self.current_request.is_none() => self.return_to_previous_or_bench(),
            _ => {}
        }
        Ok(())
    }

    fn handle_bench_key(&mut self, code: KeyCode) -> Result<()> {
        if self.current_request.is_some() {
            match code {
                KeyCode::Char('j') | KeyCode::Down => self.scroll_down(),
                KeyCode::Char('k') | KeyCode::Up => self.scroll_up(),
                _ => {}
            }
            return Ok(());
        }
        match code {
            KeyCode::Char('l') => self.start_recording(),
            KeyCode::Char('f') => {
                self.folder.refresh();
                self.screen = Screen::Folder;
                self.scroll = 0;
            }
            KeyCode::Char('n') => {
                if let Some(path) = self.folder.next_wav() {
                    self.start_file(path);
                } else {
                    self.error_message =
                        Some("no WAV files are available in this folder".to_owned());
                }
            }
            KeyCode::Char('r') => self.retry_last(),
            KeyCode::Char('m') => {
                self.catalog_index = self
                    .catalog
                    .selected_index(
                        self.active_model
                            .as_ref()
                            .map(|model| model.id.as_str())
                            .unwrap_or(&self.config.selected_stt_model),
                    )
                    .unwrap_or(0);
                self.screen = Screen::Picker;
            }
            KeyCode::Char('j') | KeyCode::Down => self.scroll_down(),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_up(),
            KeyCode::Esc => self.error_message = None,
            _ => {}
        }
        Ok(())
    }

    fn handle_folder_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.folder.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.folder.move_selection(1),
            KeyCode::Enter => {
                if let Some(path) = self.folder.enter_selected()? {
                    self.start_file(path);
                }
            }
            KeyCode::Char('d') => {
                self.directory_input = self.folder.directory.display().to_string();
                self.directory_input_error = None;
                self.screen = Screen::DirectoryInput;
            }
            KeyCode::Char('r') => self.folder.refresh(),
            KeyCode::Esc => self.screen = Screen::Bench,
            _ => {}
        }
        Ok(())
    }

    fn handle_recording_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Enter | KeyCode::Char(' ') => self.stop_recording(),
            KeyCode::Esc => {
                if let Some(recording) = self.recording.take() {
                    recording.discard();
                }
                self.status_message = "recording discarded".to_owned();
                self.screen = Screen::Bench;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_processing_key(&mut self, code: KeyCode) -> Result<()> {
        if matches!(code, KeyCode::Char('j') | KeyCode::Down) {
            self.scroll_down();
        } else if matches!(code, KeyCode::Char('k') | KeyCode::Up) {
            self.scroll_up();
        }
        Ok(())
    }

    fn handle_telemetry_key(&mut self, code: KeyCode) -> Result<()> {
        let previous_tab = self.telemetry_tab;
        match code {
            KeyCode::Char('1') => self.telemetry_tab = TelemetryTab::Overview,
            KeyCode::Char('2') => self.telemetry_tab = TelemetryTab::Metrics,
            KeyCode::Char('3') => self.telemetry_tab = TelemetryTab::Graphs,
            KeyCode::Char('4') => self.telemetry_tab = TelemetryTab::Logs,
            KeyCode::Char('j') | KeyCode::Down => self.move_telemetry(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_telemetry(-1),
            KeyCode::Char('[') => self.cycle_run(-1),
            KeyCode::Char(']') => self.cycle_run(1),
            KeyCode::Char('f') | KeyCode::Char('/') => {
                self.filter_editing = true;
                self.scroll = 0;
            }
            KeyCode::Char('x') => {
                self.filter_query.clear();
                self.scroll = 0;
            }
            KeyCode::Char('c') if self.telemetry_tab == TelemetryTab::Logs => {
                self.logs.clear();
            }
            KeyCode::Esc => {
                self.filter_editing = false;
                self.return_to_previous_or_bench();
            }
            _ => {}
        }
        if previous_tab != self.telemetry_tab {
            self.scroll = 0;
        }
        Ok(())
    }

    fn move_telemetry(&mut self, delta: isize) {
        if matches!(
            self.telemetry_tab,
            TelemetryTab::Metrics | TelemetryTab::Graphs
        ) {
            self.telemetry.move_series(&self.filter_query, delta);
        } else if delta > 0 {
            self.scroll_down();
        } else {
            self.scroll_up();
        }
    }

    fn handle_directory_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char(character) => self.directory_input.push(character),
            KeyCode::Backspace => {
                self.directory_input.pop();
            }
            KeyCode::Enter => {
                let path = PathBuf::from(self.directory_input.trim());
                match self.folder.set_directory(path) {
                    Ok(()) => {
                        self.config.audio_directory = self.folder.directory.clone();
                        if let Err(error) = config::save(&self.config) {
                            self.logs
                                .warn("config", format!("could not save audio directory: {error}"));
                        }
                        self.directory_input_error = None;
                        self.screen = Screen::Bench;
                    }
                    Err(error) => self.directory_input_error = Some(error.to_string()),
                }
            }
            KeyCode::Esc => {
                self.directory_input_error = None;
                self.screen = Screen::Folder;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_filter_key(&mut self, code: KeyCode) -> Result<()> {
        self.scroll = 0;
        match code {
            KeyCode::Char(character) => self.filter_query.push(character),
            KeyCode::Backspace => {
                self.filter_query.pop();
            }
            KeyCode::Enter | KeyCode::Esc => self.filter_editing = false,
            _ => {}
        }
        Ok(())
    }

    fn handle_help_key(&mut self, code: KeyCode) -> Result<()> {
        if matches!(code, KeyCode::Esc | KeyCode::Char('?')) {
            self.return_to_previous_or_bench();
        } else if matches!(code, KeyCode::Char('j') | KeyCode::Down) {
            self.scroll_down();
        } else if matches!(code, KeyCode::Char('k') | KeyCode::Up) {
            self.scroll_up();
        }
        Ok(())
    }

    fn handle_error_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char('r') => {
                if let Some(model_id) = self.pending_model_id.clone() {
                    self.send_load_model(
                        model_id,
                        self.active_model.as_ref().map(|model| model.id.clone()),
                    );
                }
            }
            KeyCode::Char('m') => self.screen = Screen::Picker,
            KeyCode::Esc => self.return_to_previous_or_bench(),
            _ => {}
        }
        Ok(())
    }

    fn move_catalog(&mut self, delta: isize) {
        if self.catalog.entries.is_empty() {
            self.catalog_index = 0;
            return;
        }
        let next = self.catalog_index as isize + delta;
        self.catalog_index = next.clamp(0, self.catalog.entries.len() as isize - 1) as usize;
        self.error_message = None;
    }

    fn choose_catalog_model(&mut self) {
        let Some(entry) = self.catalog.entry(self.catalog_index) else {
            self.error_message =
                Some("the model manifest contains no selectable entries".to_owned());
            return;
        };
        let model_id = entry.manifest.id.clone();
        if !entry.adapter_compiled
            && entry.artifacts_available()
            && matches!(entry.manifest.family.as_str(), "whisper" | "zipformer")
        {
            match BuildTask::start(&entry.manifest.family, model_id.clone()) {
                Ok(build) => {
                    self.build = Some(build);
                    self.pending_model_id = Some(model_id.clone());
                    self.error_message = None;
                    self.status_message = format!("building adapter for {model_id}");
                    self.logs.info("adapter-build", format!("building {model_id}; successful build restarts the TUI and resets in-memory history"));
                    self.screen = Screen::Loading;
                }
                Err(error) => {
                    self.error_message = Some(format!("{error:#}"));
                    self.logs.error("adapter-build", format!("{error:#}"));
                }
            }
            return;
        }
        if !entry.selectable() {
            self.error_message = Some(
                if !matches!(entry.manifest.family.as_str(), "whisper" | "zipformer") {
                    format!(
                        "{} uses unsupported model family `{}`",
                        model_id, entry.manifest.family
                    )
                } else {
                    format!(
                        "{} is missing required artifact(s): {}",
                        model_id,
                        ModelCatalog::missing_summary(entry)
                    )
                },
            );
            return;
        }
        self.error_message = None;
        let switch_from = self.active_model.as_ref().map(|model| model.id.clone());
        self.send_load_model(model_id, switch_from);
    }

    fn send_load_model(&mut self, model_id: String, switch_from: Option<String>) {
        if let Some(index) = self.catalog.selected_index(&model_id) {
            if !self.catalog.entries[index].adapter_compiled {
                self.catalog_index = index;
                self.screen = Screen::Picker;
                self.error_message = Some(
                    "Select this model with Enter to build its adapter and restart.".to_owned(),
                );
                return;
            }
        }
        let request_id = self.take_request_id();
        let command = WorkerCommand::LoadModel(ModelLoadRequest {
            request_id,
            model_id: model_id.clone(),
            manifest_path: self.config.model_manifest.clone(),
            max_seconds: self.config.max_seconds,
            language: self.config.language.clone(),
            dictionary: self.config.dictionary.clone(),
            switch_from,
        });
        if self.worker_sender.send(command).is_err() {
            self.error_message = Some("inference worker is not available".to_owned());
            self.screen = Screen::Error;
            return;
        }
        self.current_request = Some(request_id);
        self.pending_model_id = Some(model_id.clone());
        self.status_message = format!("loading {model_id}");
        self.screen = Screen::Loading;
        self.scroll = 0;
    }

    fn start_file(&mut self, path: PathBuf) {
        if self.current_request.is_some() {
            return;
        }
        let run_id = self.take_run_id();
        let request_id = self.take_request_id();
        let source = path.display().to_string();
        if self
            .worker_sender
            .send(WorkerCommand::TranscribeWav {
                request_id,
                run_id: run_id.clone(),
                path: path.clone(),
            })
            .is_err()
        {
            self.error_message = Some("inference worker is not available".to_owned());
            return;
        }
        self.last_file = Some(path);
        self.last_audio = None;
        self.current_source = Some(source);
        self.current_run = Some(run_id);
        self.current_request = Some(request_id);
        self.result = None;
        self.error_message = None;
        self.scroll = 0;
        self.screen = Screen::Processing;
    }

    fn start_recording(&mut self) {
        if self.current_request.is_some() || self.recording.is_some() {
            return;
        }
        match Recording::start() {
            Ok(recording) => {
                self.recording = Some(recording);
                self.status_message = "recording".to_owned();
                self.error_message = None;
                self.screen = Screen::Recording;
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                self.logs.error("recorder", error.to_string());
                self.screen = Screen::Bench;
            }
        }
    }

    fn stop_recording(&mut self) {
        let Some(recording) = self.recording.take() else {
            return;
        };
        let duration = recording.elapsed().as_secs_f32();
        match recording.finish() {
            Ok(audio) => {
                self.start_audio("live microphone".to_owned(), audio);
                self.status_message = format!("captured {duration:.1} seconds");
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                self.logs.error("recorder", error.to_string());
                self.screen = Screen::Bench;
            }
        }
    }

    fn start_audio(&mut self, source: String, audio: AudioBuffer) {
        if self.current_request.is_some() {
            return;
        }
        let run_id = self.take_run_id();
        let request_id = self.take_request_id();
        if self
            .worker_sender
            .send(WorkerCommand::TranscribeAudio {
                request_id,
                run_id: run_id.clone(),
                source: source.clone(),
                audio: audio.clone(),
            })
            .is_err()
        {
            self.error_message = Some("inference worker is not available".to_owned());
            return;
        }
        self.last_file = None;
        self.last_audio = Some(audio);
        self.current_source = Some(source);
        self.current_run = Some(run_id);
        self.current_request = Some(request_id);
        self.result = None;
        self.error_message = None;
        self.scroll = 0;
        self.screen = Screen::Processing;
    }

    fn retry_last(&mut self) {
        if let Some(path) = self.last_file.clone() {
            self.start_file(path);
        } else if let Some(audio) = self.last_audio.clone() {
            self.start_audio("live microphone".to_owned(), audio);
        } else {
            self.error_message = Some("there is no previous source to retry".to_owned());
        }
    }

    fn drain_worker_events(&mut self) {
        loop {
            let event = match self.worker_receiver.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.logs
                        .error("worker", "worker event channel disconnected");
                    self.error_message = Some("inference worker stopped unexpectedly".to_owned());
                    break;
                }
            };
            self.apply_worker_event(event);
        }
    }

    fn apply_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::ModelLoadStarted {
                request_id,
                model_id,
                switch_from,
            } if self.accepts_request(request_id) => {
                self.pending_model_id = Some(model_id.clone());
                self.telemetry
                    .select_run(Some(format!("model-load-{request_id}")));
                self.status_message = match switch_from.as_deref() {
                    Some(previous) => format!("switching from {previous} to {model_id}"),
                    None => format!("loading {model_id}"),
                };
                self.screen = Screen::Loading;
            }
            WorkerEvent::ModelReady {
                request_id,
                model_id,
                model_family,
                backend,
                load_duration_ms,
                switch_from,
            } if self.accepts_request(request_id) => {
                let runtime = self
                    .catalog
                    .entry_by_id(&model_id)
                    .and_then(|entry| entry.manifest.runtime.clone());
                self.active_model = Some(ActiveModel {
                    id: model_id.clone(),
                    family: model_family,
                    backend,
                    runtime,
                });
                self.config.selected_stt_model = model_id.clone();
                if let Err(error) = config::save(&self.config) {
                    self.logs
                        .warn("config", format!("could not save selected model: {error}"));
                }
                if let Some(previous) = switch_from.as_deref() {
                    self.logs.info(
                        "model-loader",
                        format!("switched from {previous} to {model_id} in {load_duration_ms} ms"),
                    );
                } else {
                    self.logs.info(
                        "model-loader",
                        format!("loaded {model_id} in {load_duration_ms} ms"),
                    );
                }
                self.pending_model_id = None;
                self.current_request = None;
                self.error_message = None;
                self.status_message = format!("model {model_id} ready");
                self.screen = Screen::Bench;
            }
            WorkerEvent::ModelLoadFailed {
                request_id,
                model_id,
                error,
                switch_from,
            } if self.accepts_request(request_id) => {
                self.pending_model_id = Some(model_id.clone());
                self.current_request = None;
                self.error_message = Some(match switch_from.as_deref() {
                    Some(previous) => {
                        format!("model switch failed; {previous} remains active: {error}")
                    }
                    None => error.clone(),
                });
                self.logs.error("model-loader", error);
                self.screen = Screen::Error;
            }
            WorkerEvent::ProcessingStarted {
                request_id,
                run_id,
                source,
            } if self.accepts_request(request_id) => {
                self.current_run = Some(run_id);
                self.current_source = Some(source.clone());
                self.status_message = format!("processing {source}");
                self.screen = Screen::Processing;
            }
            WorkerEvent::Result {
                request_id,
                run_id,
                source,
                audio_duration_seconds,
                result,
            } if self.accepts_request(request_id) => {
                self.result = Some(ResultState {
                    run_id: run_id.clone(),
                    source,
                    audio_duration_seconds,
                    result: *result,
                });
                self.current_run = Some(run_id.clone());
                self.telemetry.select_run(Some(run_id));
                self.current_request = None;
                self.error_message = None;
                self.status_message = "transcription complete".to_owned();
                self.scroll = 0;
                self.screen = Screen::Bench;
            }
            WorkerEvent::ProcessingFailed {
                request_id,
                run_id,
                source,
                error,
            } if self.accepts_request(request_id) => {
                self.current_run = Some(run_id);
                self.current_source = Some(source);
                self.current_request = None;
                self.error_message = Some(error.clone());
                self.logs.error("worker", error);
                self.screen = Screen::Bench;
            }
            WorkerEvent::Log(entry) => self.logs.push(entry),
            WorkerEvent::WorkerStopped => self.logs.warn("worker", "worker stopped"),
            _ => {}
        }
    }

    fn drain_metrics(&mut self) {
        self.telemetry.drain(&self.metric_receiver);
        let dropped = self
            .metric_dropped
            .load(std::sync::atomic::Ordering::Relaxed);
        self.telemetry.observe_drops(dropped, &mut self.logs);
    }

    fn accepts_request(&self, request_id: RequestId) -> bool {
        self.current_request == Some(request_id)
    }

    fn take_request_id(&mut self) -> RequestId {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        request_id
    }

    fn take_run_id(&mut self) -> String {
        let run_id = format!("run-{:04}", self.next_run_id);
        self.next_run_id = self.next_run_id.saturating_add(1);
        run_id
    }

    fn open_telemetry(&mut self) {
        self.previous_screen = self.screen;
        self.telemetry_tab = if self.build.is_some() {
            TelemetryTab::Logs
        } else {
            TelemetryTab::Overview
        };
        self.scroll = 0;
        self.screen = Screen::Telemetry;
    }

    fn return_to_previous_or_bench(&mut self) {
        self.screen = if self.build.is_some() {
            Screen::Loading
        } else if self.previous_screen == Screen::Telemetry {
            Screen::Bench
        } else {
            self.previous_screen
        };
        self.scroll = 0;
    }

    fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    fn cycle_run(&mut self, delta: isize) {
        let runs = self.telemetry.run_ids();
        if runs.is_empty() {
            return;
        }
        let current = self
            .telemetry
            .selected_run()
            .and_then(|selected| runs.iter().position(|run| run == selected))
            .unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(runs.len() as isize) as usize;
        self.telemetry.select_run(runs.get(next).cloned());
        self.scroll = 0;
    }

    pub fn shutdown_worker(&self) {
        let _ = self.worker_sender.send(WorkerCommand::Shutdown);
    }
}

impl Default for App {
    fn default() -> Self {
        panic!("App requires runtime channels and configuration")
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::sync::mpsc;

    pub fn test_app() -> App {
        let options = TuiOptions {
            model_id: None,
            model_manifest: None,
            audio_directory: None,
            max_seconds: None,
            language: None,
            dictionary: None,
            reconfigure: false,
        };
        let catalog = ModelCatalog {
            manifest_path: PathBuf::new(),
            entries: Vec::new(),
            error: None,
        };
        let (sender, _) = mpsc::channel();
        let (_, worker_receiver) = mpsc::channel();
        let (_, metric_receiver) = mpsc::channel();
        let mut app = App::new(
            &options,
            TuiConfig::default(),
            true,
            catalog,
            sender,
            worker_receiver,
            metric_receiver,
            Arc::new(AtomicU64::new(0)),
        );
        app.screen = Screen::Telemetry;
        app
    }

    pub fn metric(run: &str, name: &str) -> MetricEvent {
        MetricEvent {
            schema_version: metrics::METRICS_SCHEMA_VERSION,
            experiment_id: None,
            run_id: run.into(),
            sequence: 0,
            timestamp_ms: 1000,
            name: name.into(),
            value: Some(metrics::MetricValue::Number(0.25)),
            unit: metrics::MetricUnit::Percent,
            scope: metrics::MetricScope::Process,
            source: "test-sensor".into(),
            unavailable_reason: None,
        }
    }

    #[test]
    fn filter_is_shared_live_and_quit_keys_are_text_while_editing() {
        let mut app = test_app();
        for tab in ['1', '2', '3', '4'] {
            app.handle_key(KeyCode::Char(tab)).unwrap();
            app.scroll = 20;
            app.handle_key(KeyCode::Char('/')).unwrap();
            app.handle_key(KeyCode::Char('q')).unwrap();
            assert!(!app.should_quit);
            assert_eq!(app.scroll, 0);
            assert_eq!(app.filter_query, "q");
            app.handle_key(KeyCode::Esc).unwrap();
            assert!(!app.filter_editing);
            assert_eq!(app.screen, Screen::Telemetry);
            app.handle_key(KeyCode::Char('x')).unwrap();
            assert!(app.filter_query.is_empty());
        }
        app.handle_key(KeyCode::Char('f')).unwrap();
        app.handle_key(KeyCode::Char('a')).unwrap();
        app.handle_key(KeyCode::Char('b')).unwrap();
        app.handle_key(KeyCode::Backspace).unwrap();
        app.handle_key(KeyCode::Enter).unwrap();
        app.handle_key(KeyCode::Char('2')).unwrap();
        assert_eq!(app.filter_query, "a");
    }

    #[test]
    fn series_keys_are_shared_between_metrics_and_graphs_and_runs_cycle() {
        let mut app = test_app();
        app.telemetry.push(metric("run-1", "alpha"));
        app.telemetry.push(metric("run-1", "beta"));
        app.telemetry.push(metric("run-2", "gamma"));
        app.handle_key(KeyCode::Char('2')).unwrap();
        app.handle_key(KeyCode::Down).unwrap();
        assert_eq!(app.telemetry.series_index(&app.telemetry.series("")), 1);
        app.handle_key(KeyCode::Char('3')).unwrap();
        assert_eq!(app.telemetry.series_index(&app.telemetry.series("")), 1);
        app.handle_key(KeyCode::Char('k')).unwrap();
        assert_eq!(app.telemetry.series_index(&app.telemetry.series("")), 0);
        app.handle_key(KeyCode::Char(']')).unwrap();
        assert_eq!(app.telemetry.selected_run(), Some("run-2"));
        app.handle_key(KeyCode::Char('[')).unwrap();
        assert_eq!(app.telemetry.selected_run(), Some("run-1"));
        app.scroll = 50;
        app.handle_key(KeyCode::Char('4')).unwrap();
        assert_eq!(app.scroll, 0);
    }
}
