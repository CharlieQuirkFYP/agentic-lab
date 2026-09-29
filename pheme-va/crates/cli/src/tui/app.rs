use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;

use anyhow::Result;
use metrics::MetricEvent;
use ratatui::crossterm::event::{Event, KeyCode};
use va_core::{AudioBuffer, TranscriptionResult};

use crate::model;
use crate::recorder::Recording;

use super::config::{self, TuiConfig};
use super::download::{DownloadOutcome, DownloadTask};
use super::events::{ModelLoadRequest, RequestId, WorkerCommand, WorkerEvent};
use super::folder::FolderState;
use super::logs::LogStore;
use super::model_catalog::ModelCatalog;
use super::rebuild::{BuildTask, CacheProbeTask, CacheStatus};
use super::telemetry::{SeriesKey, TelemetryStore};
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
    Runs,
    Logs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterAvailability {
    Unsupported,
    ArtifactMissing,
    Compiled,
    Cached,
    Checking,
    Prepare,
}

impl AdapterAvailability {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported model family",
            Self::ArtifactMissing => "artifact missing",
            Self::Compiled => "compiled",
            Self::Cached => "cached",
            Self::Checking => "checking cache",
            Self::Prepare => "prepare on selection",
        }
    }

    pub fn is_ready(self) -> bool {
        matches!(self, Self::Compiled | Self::Cached)
    }
}

impl TelemetryTab {
    pub fn index(self) -> usize {
        match self {
            Self::Overview => 0,
            Self::Metrics => 1,
            Self::Runs => 2,
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
    pub metric_detail: bool,
    pub run_detail: bool,
    pub clear_runs_pending: bool,
    clear_runs_inflight: bool,
    history: Option<super::history::History>,
    saved_history_revision: u64,
    run_namespace: String,
    pub historical_metric: Option<SeriesKey>,
    pub historical_detail: bool,
    pub historical_scroll: u16,
    pub run_table: std::cell::RefCell<ratatui::widgets::TableState>,
    historical_run: Option<String>,
    pub filter_query: String,
    pub filter_editing: bool,
    pub directory_input: String,
    pub directory_input_error: Option<String>,
    pub scroll: u16,
    pub should_quit: bool,
    pub build: Option<BuildTask>,
    pub download: Option<DownloadTask>,
    pub restart: Option<(PathBuf, String)>,
    pub adapter_cache: HashMap<String, CacheStatus>,
    cache_probe: Option<CacheProbeTask>,
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
            metric_detail: false,
            run_detail: false,
            clear_runs_pending: false,
            clear_runs_inflight: false,
            history: None,
            saved_history_revision: 0,
            run_namespace: format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            historical_metric: None,
            historical_detail: false,
            historical_scroll: 0,
            run_table: Default::default(),
            historical_run: None,
            filter_query: String::new(),
            filter_editing: false,
            directory_input,
            directory_input_error: None,
            scroll: 0,
            should_quit: false,
            build: None,
            download: None,
            restart: None,
            adapter_cache: HashMap::new(),
            cache_probe: None,
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

    pub fn start_cache_probe(&mut self) {
        let families = self
            .catalog
            .entries
            .iter()
            .filter(|entry| matches!(entry.manifest.family.as_str(), "whisper" | "zipformer"))
            .map(|entry| entry.manifest.family.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        self.adapter_cache.clear();
        for family in &families {
            self.adapter_cache
                .insert(family.clone(), CacheStatus::Checking);
        }
        self.cache_probe = None;
        if families.is_empty() {
            return;
        }
        match CacheProbeTask::start(families.clone()) {
            Ok(probe) => self.cache_probe = Some(probe),
            Err(error) => {
                for family in families {
                    self.adapter_cache.insert(family, CacheStatus::Unavailable);
                }
                self.logs.warn(
                    "adapter-cache",
                    format!("could not start cache probe: {error:#}"),
                );
            }
        }
    }

    fn poll_cache_probe(&mut self) {
        let Some(mut probe) = self.cache_probe.take() else {
            return;
        };
        match probe.poll() {
            Ok(Some(results)) => {
                for (family, cached) in results {
                    self.adapter_cache.insert(
                        family,
                        if cached {
                            CacheStatus::Cached
                        } else {
                            CacheStatus::Missing
                        },
                    );
                }
                self.logs.info("adapter-cache", "validated adapter cache");
            }
            Ok(None) => self.cache_probe = Some(probe),
            Err(error) => {
                for status in self.adapter_cache.values_mut() {
                    if *status == CacheStatus::Checking {
                        *status = CacheStatus::Unavailable;
                    }
                }
                self.logs
                    .warn("adapter-cache", format!("cache probe failed: {error:#}"));
            }
        }
    }

    pub fn adapter_availability(
        &self,
        entry: &super::model_catalog::CatalogEntry,
    ) -> AdapterAvailability {
        if !model::family_supported(&entry.manifest.family) {
            AdapterAvailability::Unsupported
        } else if !entry.artifacts_available() {
            AdapterAvailability::ArtifactMissing
        } else if entry.adapter_compiled {
            AdapterAvailability::Compiled
        } else {
            match self
                .adapter_cache
                .get(&entry.manifest.family)
                .copied()
                .unwrap_or(CacheStatus::Checking)
            {
                CacheStatus::Cached => AdapterAvailability::Cached,
                CacheStatus::Checking => AdapterAvailability::Checking,
                CacheStatus::Missing | CacheStatus::Unavailable => AdapterAvailability::Prepare,
            }
        }
    }

    pub fn tick(&mut self) {
        self.drain_metrics();
        self.drain_worker_events();
        self.poll_cache_probe();
        self.poll_download();
        self.poll_build();
        self.sync_historical_selection();
        self.persist_history();
        self.poll_history();
        if self.recording.as_ref().is_some_and(|recording| {
            recording.elapsed().as_secs() >= self.config.max_seconds as u64
        }) {
            self.stop_recording();
            self.status_message = "maximum recording duration reached".to_owned();
        }
    }

    fn poll_download(&mut self) {
        let Some(download) = self.download.as_mut() else {
            return;
        };
        match download.poll() {
            Ok(Some(DownloadOutcome::Completed)) => {
                let model_id = download.model_id.clone();
                self.download.take();
                self.catalog.refresh();
                if let Some(entry) = self.catalog.entry_by_id(&model_id) {
                    if !entry.artifacts_available() {
                        self.error_message = Some(format!(
                            "download completed but artifacts are still missing: {}",
                            ModelCatalog::missing_summary(entry)
                        ));
                        self.screen = Screen::Picker;
                    } else if !entry.adapter_compiled {
                        self.start_adapter_build(&model_id);
                    } else {
                        self.send_load_model(
                            model_id,
                            self.active_model.as_ref().map(|model| model.id.clone()),
                        );
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.error_message = Some(format!("{error:#}"));
                self.logs.error("model-download", format!("{error:#}"));
                self.download.take();
                self.screen = Screen::Picker;
            }
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
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('f')
                    && self.screen == Screen::Telemetry
                    && !self.clear_runs_pending
                    && !self.clear_runs_inflight
                {
                    self.filter_editing = true;
                    self.scroll = 0;
                } else {
                    self.handle_key(key.code)?;
                }
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, code: KeyCode) -> Result<()> {
        if self.clear_runs_inflight {
            return Ok(());
        }
        if self.clear_runs_pending {
            match code {
                KeyCode::Char('y') => {
                    self.clear_runs_pending = false;
                    if !self.run_active() {
                        if let Some(history) = self.history.as_mut() {
                            self.clear_runs_inflight = true;
                            history.clear();
                            self.status_message = "clearing saved runs…".into();
                        }
                    }
                }
                KeyCode::Esc => {
                    self.clear_runs_pending = false;
                    self.status_message = "clear runs cancelled".into();
                }
                _ => {}
            }
            return Ok(());
        }
        if (self.build.is_some() || self.download.is_some())
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
                self.start_cache_probe();
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
        if self.download.is_some() {
            if code == KeyCode::Esc {
                if let Some(mut download) = self.download.take() {
                    download.cancel();
                }
                self.logs.info(
                    "model-download",
                    "download cancelled; partial files were retained",
                );
                self.screen = Screen::Picker;
            }
            return Ok(());
        }
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
                if let Some(run_id) = self.current_run.clone() {
                    self.end_run_context(&run_id);
                    self.telemetry.upsert_report(self.failed_report(
                        &run_id,
                        "live microphone",
                        "recording discarded".to_owned(),
                    ));
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
        if self.filter_editing {
            return self.handle_filter_key(code);
        }
        if code == KeyCode::Char('c') && self.telemetry_tab == TelemetryTab::Runs {
            if self.run_active() {
                self.status_message =
                    "cannot clear runs during an active request or recording".into();
            } else if self.history.is_some() {
                self.clear_runs_pending = true;
                self.status_message =
                    "Clear ALL saved runs and transcripts? [y] confirm · [Esc] cancel".into();
            } else {
                self.status_message = "run history storage is unavailable".into();
            }
            return Ok(());
        }
        if self.metric_detail {
            match code {
                KeyCode::Esc => self.metric_detail = false,
                KeyCode::Char('j') | KeyCode::Down => {
                    self.telemetry.move_metric(&self.filter_query, 1)
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.telemetry.move_metric(&self.filter_query, -1)
                }
                KeyCode::Char('f') | KeyCode::Char('/') => self.filter_editing = true,
                KeyCode::Char('x') => self.filter_query.clear(),
                KeyCode::Char('n') => self.telemetry.move_metric(&self.filter_query, 1),
                KeyCode::Char('N') => self.telemetry.move_metric(&self.filter_query, -1),
                _ => {}
            }
            return Ok(());
        }
        if self.run_detail {
            self.sync_historical_selection();
            match code {
                KeyCode::Esc if self.historical_detail => self.historical_detail = false,
                KeyCode::Esc => self.run_detail = false,
                KeyCode::Enter => {
                    self.historical_detail = self.historical_metric.is_some();
                    self.historical_scroll = 0;
                }
                KeyCode::Char('j') | KeyCode::Down => self.move_historical_metric(1),
                KeyCode::Char('k') | KeyCode::Up => self.move_historical_metric(-1),
                KeyCode::PageDown if self.historical_detail => {
                    self.historical_scroll = self.historical_scroll.saturating_add(5);
                }
                KeyCode::PageUp if self.historical_detail => {
                    self.historical_scroll = self.historical_scroll.saturating_sub(5);
                }
                KeyCode::PageDown => self.scroll = self.scroll.saturating_add(5),
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(5),
                KeyCode::Char('[') => self.cycle_run(-1),
                KeyCode::Char(']') => self.cycle_run(1),
                KeyCode::Char('f') | KeyCode::Char('/') => self.filter_editing = true,
                KeyCode::Char('x') => self.filter_query.clear(),
                KeyCode::Char('n') => self.cycle_run(1),
                KeyCode::Char('N') => self.cycle_run(-1),
                _ => {}
            }
            return Ok(());
        }
        let previous_tab = self.telemetry_tab;
        match code {
            KeyCode::Char('1') => self.telemetry_tab = TelemetryTab::Overview,
            KeyCode::Char('2') => self.telemetry_tab = TelemetryTab::Metrics,
            KeyCode::Char('3') => self.telemetry_tab = TelemetryTab::Runs,
            KeyCode::Char('4') => self.telemetry_tab = TelemetryTab::Logs,
            KeyCode::Char('j') | KeyCode::Down => self.move_telemetry(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_telemetry(-1),
            KeyCode::Enter if self.telemetry_tab == TelemetryTab::Metrics => {
                self.metric_detail = self.telemetry.selected_metric(&self.filter_query).is_some();
                self.scroll = 0;
            }
            KeyCode::Enter if self.telemetry_tab == TelemetryTab::Runs => {
                self.run_detail = self.telemetry.selected_report().is_some();
                self.sync_historical_selection();
                self.historical_detail = false;
                self.scroll = 0;
            }
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
            KeyCode::Char('n') => self.move_telemetry(1),
            KeyCode::Char('N') => self.move_telemetry(-1),
            KeyCode::Char('c') if self.telemetry_tab == TelemetryTab::Logs => self.logs.clear(),
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
        match self.telemetry_tab {
            TelemetryTab::Metrics => self.telemetry.move_metric(&self.filter_query, delta),
            TelemetryTab::Runs => self.cycle_run(delta),
            _ if delta > 0 => self.scroll_down(),
            _ => self.scroll_up(),
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

    fn start_adapter_build(&mut self, model_id: &str) {
        let Some(entry) = self.catalog.entry_by_id(model_id) else {
            self.error_message = Some(format!("model `{model_id}` is not in the catalog"));
            return;
        };
        match BuildTask::start(&entry.manifest.family, model_id.to_owned()) {
            Ok(build) => {
                self.build = Some(build);
                self.pending_model_id = Some(model_id.to_owned());
                self.error_message = None;
                self.status_message = format!("preparing backend for {model_id}");
                self.logs
                    .info("adapter-build", format!("preparing backend for {model_id}"));
                self.screen = Screen::Loading;
            }
            Err(error) => {
                self.error_message = Some(format!("{error:#}"));
                self.logs.error("adapter-build", format!("{error:#}"));
                self.screen = Screen::Picker;
            }
        }
    }

    fn start_model_download(&mut self, model_id: String) {
        let model_dir = self
            .config
            .model_manifest
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        match DownloadTask::start(model_id.clone(), model_dir) {
            Ok(download) => {
                self.download = Some(download);
                self.pending_model_id = Some(model_id.clone());
                self.error_message = None;
                self.status_message = format!("downloading model {model_id}");
                self.logs
                    .info("model-download", format!("downloading model {model_id}"));
                self.screen = Screen::Loading;
            }
            Err(error) => {
                self.error_message = Some(format!("{error:#}"));
                self.logs.error("model-download", format!("{error:#}"));
                self.screen = Screen::Picker;
            }
        }
    }

    fn choose_catalog_model(&mut self) {
        let Some(entry) = self.catalog.entry(self.catalog_index) else {
            self.error_message =
                Some("the model manifest contains no selectable entries".to_owned());
            return;
        };
        let model_id = entry.manifest.id.clone();
        if !matches!(entry.manifest.family.as_str(), "whisper" | "zipformer") {
            self.error_message = Some(format!(
                "{} uses unsupported model family `{}`",
                model_id, entry.manifest.family
            ));
            return;
        }
        if !entry.artifacts_available() {
            self.start_model_download(model_id);
            return;
        }
        if !entry.adapter_compiled {
            self.start_adapter_build(&model_id);
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
            self.catalog_index = index;
            let entry = &self.catalog.entries[index];
            if !entry.artifacts_available() {
                self.start_model_download(model_id);
                return;
            }
            if !entry.adapter_compiled {
                self.start_adapter_build(&model_id);
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

    fn begin_run(&mut self, run_id: String, source: String) -> bool {
        if self
            .worker_sender
            .send(WorkerCommand::BeginRun {
                run_id: run_id.clone(),
            })
            .is_err()
        {
            self.error_message = Some("inference worker is not available".to_owned());
            return false;
        }
        let now = epoch_millis();
        let model = self.active_model.clone().unwrap_or(ActiveModel {
            id: self.config.selected_stt_model.clone(),
            family: "unknown".to_owned(),
            backend: "unknown".to_owned(),
            runtime: None,
        });
        self.telemetry.set_active_run(Some(run_id.clone()));
        self.telemetry.upsert_report(super::telemetry::RunReport {
            run_id: run_id.clone(),
            model_id: model.id,
            model_family: model.family,
            backend: model.backend,
            runtime: model.runtime,
            revision: self
                .catalog
                .entry_by_id(&self.config.selected_stt_model)
                .and_then(|entry| entry.manifest.revision.clone()),
            source,
            status: "BUSY".to_owned(),
            transcript: String::new(),
            raw_transcript: String::new(),
            language: None,
            audio_duration_seconds: 0.0,
            gate_decision: "pending".to_owned(),
            segment_count: 0,
            started_at_ms: now,
            finished_at_ms: None,
            error: None,
            result: None,
            events: Vec::new(),
        });
        true
    }

    fn end_run_context(&mut self, run_id: &str) {
        let _ = self.worker_sender.send(WorkerCommand::EndRun {
            run_id: run_id.to_owned(),
        });
        self.telemetry.set_active_run(None);
    }

    fn failed_report(
        &self,
        run_id: &str,
        source: &str,
        error: String,
    ) -> super::telemetry::RunReport {
        let mut report = self.telemetry.report(run_id).cloned().unwrap_or_else(|| {
            let model = self.active_model.clone().unwrap_or(ActiveModel {
                id: self.config.selected_stt_model.clone(),
                family: "unknown".to_owned(),
                backend: "unknown".to_owned(),
                runtime: None,
            });
            super::telemetry::RunReport {
                run_id: run_id.to_owned(),
                model_id: model.id,
                model_family: model.family,
                backend: model.backend,
                runtime: model.runtime,
                revision: None,
                source: source.to_owned(),
                status: "ERROR".to_owned(),
                transcript: String::new(),
                raw_transcript: String::new(),
                language: None,
                audio_duration_seconds: 0.0,
                gate_decision: "unknown".to_owned(),
                segment_count: 0,
                started_at_ms: epoch_millis(),
                finished_at_ms: None,
                error: None,
                result: None,
                events: Vec::new(),
            }
        });
        report.status = "ERROR".to_owned();
        report.error = Some(error);
        report.finished_at_ms = Some(epoch_millis());
        report.events = self
            .telemetry
            .events_for(Some(run_id))
            .into_iter()
            .cloned()
            .collect();
        report
    }

    fn start_file(&mut self, path: PathBuf) {
        if self.current_request.is_some() {
            return;
        }
        let run_id = self.take_run_id();
        let request_id = self.take_request_id();
        let source = path.display().to_string();
        if !self.begin_run(run_id.clone(), source.clone()) {
            return;
        }
        if self
            .worker_sender
            .send(WorkerCommand::TranscribeWav {
                request_id,
                run_id: run_id.clone(),
                path: path.clone(),
            })
            .is_err()
        {
            self.end_run_context(&run_id);
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
        let run_id = self.take_run_id();
        if !self.begin_run(run_id.clone(), "live microphone".to_owned()) {
            return;
        }
        match Recording::start() {
            Ok(recording) => {
                self.recording = Some(recording);
                self.current_run = Some(run_id);
                self.current_source = Some("live microphone".to_owned());
                self.status_message = "recording".to_owned();
                self.error_message = None;
                self.screen = Screen::Recording;
            }
            Err(error) => {
                self.end_run_context(&run_id);
                self.telemetry.upsert_report(self.failed_report(
                    &run_id,
                    "live microphone",
                    error.to_string(),
                ));
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
                if let Some(run_id) = self.current_run.clone() {
                    self.end_run_context(&run_id);
                    self.telemetry.upsert_report(self.failed_report(
                        &run_id,
                        "live microphone",
                        error.to_string(),
                    ));
                }
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
        let reuse_run = self
            .current_run
            .as_deref()
            .and_then(|run| self.telemetry.report(run))
            .is_some_and(|report| report.status == "BUSY");
        let run_id = if reuse_run {
            self.current_run.clone().expect("checked above")
        } else {
            let run_id = self.take_run_id();
            if !self.begin_run(run_id.clone(), source.clone()) {
                return;
            }
            run_id
        };
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
            self.end_run_context(&run_id);
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
                self.drain_metrics();
                let result = *result;
                let mut report = self.telemetry.report(&run_id).cloned().unwrap_or_else(|| {
                    self.failed_report(&run_id, &source, "missing run context".to_owned())
                });
                report.status = result.status.as_str().to_owned();
                report.transcript = result.text.clone();
                report.raw_transcript = result.raw_text.clone();
                report.language = result.language.clone();
                report.audio_duration_seconds = audio_duration_seconds;
                report.gate_decision = format!("{:?}", result.gate.decision);
                report.segment_count = result.segments.len();
                report.finished_at_ms = Some(epoch_millis());
                report.error = None;
                report.events = self
                    .telemetry
                    .events_for(Some(&run_id))
                    .into_iter()
                    .cloned()
                    .collect();
                report.result = Some(result.clone());
                self.telemetry.upsert_report(report);
                self.telemetry.set_active_run(None);
                self.result = Some(ResultState {
                    run_id: run_id.clone(),
                    source,
                    audio_duration_seconds,
                    result,
                });
                self.current_run = Some(run_id.clone());
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
                self.current_run = Some(run_id.clone());
                self.current_source = Some(source.clone());
                self.current_request = None;
                self.drain_metrics();
                self.telemetry
                    .upsert_report(self.failed_report(&run_id, &source, error.clone()));
                self.telemetry.set_active_run(None);
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
        loop {
            let run_id = format!("run-{}-{:04}", self.run_namespace, self.next_run_id);
            self.next_run_id = self.next_run_id.wrapping_add(1);
            if self.telemetry.report(&run_id).is_none() {
                return run_id;
            }
        }
    }

    fn open_telemetry(&mut self) {
        self.previous_screen = self.screen;
        self.telemetry_tab = if self.build.is_some() || self.download.is_some() {
            TelemetryTab::Logs
        } else {
            TelemetryTab::Overview
        };
        self.scroll = 0;
        self.screen = Screen::Telemetry;
    }

    fn return_to_previous_or_bench(&mut self) {
        self.screen = if self.build.is_some() || self.download.is_some() {
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

    fn sync_historical_selection(&mut self) {
        let run = self.telemetry.selected_run().map(str::to_owned);
        if self.historical_run != run {
            self.historical_run = run.clone();
            self.historical_metric = None;
            self.historical_detail = false;
            self.historical_scroll = 0;
            self.scroll = 0;
            *self.run_table.borrow_mut() = Default::default();
        }
        let series = run
            .as_deref()
            .map(|run| self.telemetry.series_for_run(run))
            .unwrap_or_default();
        if !series
            .iter()
            .any(|item| Some(&item.key) == self.historical_metric.as_ref())
        {
            self.historical_metric = series.first().map(|item| item.key.clone());
            self.historical_detail = false;
            self.historical_scroll = 0;
        }
    }

    fn move_historical_metric(&mut self, delta: isize) {
        self.sync_historical_selection();
        let Some(run) = self.telemetry.selected_run() else {
            return;
        };
        let series = self.telemetry.series_for_run(run);
        if series.is_empty() {
            return;
        }
        let current = series
            .iter()
            .position(|item| Some(&item.key) == self.historical_metric.as_ref())
            .unwrap_or(0);
        let next = (current as isize + delta).clamp(0, series.len() as isize - 1) as usize;
        self.historical_metric = Some(series[next].key.clone());
        self.historical_scroll = 0;
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
        self.sync_historical_selection();
        self.scroll = 0;
    }

    pub fn attach_history(
        &mut self,
        history: super::history::History,
        reports: Vec<super::telemetry::RunReport>,
    ) {
        for report in reports.into_iter().rev() {
            self.telemetry.upsert_report(report);
        }
        self.saved_history_revision = self.telemetry.history_revision;
        self.history = Some(history);
    }

    fn run_active(&self) -> bool {
        self.current_request.is_some()
            || self.recording.is_some()
            || self.telemetry.active_run().is_some()
    }

    fn persist_history(&mut self) {
        if self.clear_runs_inflight
            || self.saved_history_revision == self.telemetry.history_revision
        {
            return;
        }
        if let Some(history) = self.history.as_mut() {
            history.save(
                self.telemetry
                    .reports()
                    .filter(|report| report.status != "BUSY")
                    .cloned()
                    .collect(),
            );
            self.saved_history_revision = self.telemetry.history_revision;
        }
    }

    fn poll_history(&mut self) {
        let Some(history) = self.history.as_mut() else {
            return;
        };
        history.pump();
        let outcomes = history.outcomes();
        for (clear, result) in outcomes {
            if clear {
                self.clear_runs_inflight = false;
            }
            match result {
                Ok(()) if clear => {
                    self.telemetry.clear_runs();
                    self.saved_history_revision = self.telemetry.history_revision;
                    self.result = None;
                    self.current_run = None;
                    self.current_source = None;
                    self.run_detail = false;
                    self.historical_run = None;
                    self.historical_metric = None;
                    self.historical_detail = false;
                    self.historical_scroll = 0;
                    self.scroll = 0;
                    *self.run_table.borrow_mut() = Default::default();
                    self.status_message = "all saved runs cleared".into();
                }
                Err(error) => {
                    self.status_message = error.clone();
                    self.error_message = Some(error.clone());
                    self.logs.error("history", error);
                }
                _ => {}
            }
        }
    }

    pub fn flush_history(&mut self) -> Result<()> {
        self.drain_metrics();
        self.drain_worker_events();
        self.persist_history();
        let result = self
            .history
            .as_mut()
            .map_or(Ok(()), |history| history.flush());
        self.poll_history();
        result
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

fn epoch_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
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

    #[test]
    fn adapter_availability_reports_a_validated_cache_separately() {
        let mut app = test_app();
        let manifest: crate::model::ModelEntry = toml::from_str(
            r#"
            id = "fixture"
            family = "whisper"
            model = "fixture.bin"
            "#,
        )
        .unwrap();
        let entry = super::super::model_catalog::CatalogEntry {
            manifest,
            model_path: PathBuf::from("fixture.bin"),
            missing_paths: Vec::new(),
            // Force the fixture to represent a featureless launcher regardless
            // of the features used to compile this test binary.
            adapter_compiled: false,
        };

        assert_eq!(
            app.adapter_availability(&entry),
            AdapterAvailability::Checking
        );
        app.adapter_cache
            .insert("whisper".into(), CacheStatus::Cached);
        assert_eq!(
            app.adapter_availability(&entry),
            AdapterAvailability::Cached
        );
        app.adapter_cache
            .insert("whisper".into(), CacheStatus::Missing);
        assert_eq!(
            app.adapter_availability(&entry),
            AdapterAvailability::Prepare
        );
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

    pub fn report(run: &str, events: Vec<MetricEvent>) -> super::super::telemetry::RunReport {
        super::super::telemetry::RunReport {
            run_id: run.into(),
            model_id: "test-model".into(),
            model_family: "test-family".into(),
            backend: "test-backend".into(),
            runtime: Some("local".into()),
            revision: Some("abc123".into()),
            source: "fixture.wav".into(),
            status: "speech".into(),
            transcript: "Keep this transcript".into(),
            raw_transcript: "raw words".into(),
            language: Some("en".into()),
            audio_duration_seconds: 2.5,
            gate_decision: "speech".into(),
            segment_count: 1,
            started_at_ms: 1000,
            finished_at_ms: Some(3500),
            error: None,
            result: None,
            events,
        }
    }

    #[test]
    fn history_restart_ids_and_confirmed_clear() {
        use super::super::history::{tests::TestPath, History};
        let path = TestPath::new();
        let mut app = test_app();
        let (history, reports) = History::open(path.file()).unwrap();
        app.attach_history(history, reports);
        let id = app.take_run_id();
        app.telemetry
            .upsert_report(report(&id, vec![metric(&id, "cpu")]));
        app.flush_history().unwrap();
        drop(app);

        let mut app = test_app();
        let (history, reports) = History::open(path.file()).unwrap();
        app.attach_history(history, reports);
        assert!(app.telemetry.report(&id).is_some());
        assert_ne!(app.take_run_id(), id);
        let collision = format!("run-{}-{:04}", app.run_namespace, app.next_run_id);
        app.telemetry.upsert_report(report(&collision, vec![]));
        assert_ne!(app.take_run_id(), collision);
        app.telemetry_tab = TelemetryTab::Runs;
        app.current_request = Some(1);
        app.handle_key(KeyCode::Char('c')).unwrap();
        assert!(!app.clear_runs_pending);
        app.current_request = None;
        app.handle_key(KeyCode::Char('c')).unwrap();
        assert!(app.clear_runs_pending);
        app.handle_key(KeyCode::Esc).unwrap();
        assert!(!app.clear_runs_pending);
        assert!(app.telemetry.report(&id).is_some());
        app.handle_key(KeyCode::Char('c')).unwrap();
        app.handle_key(KeyCode::Char('y')).unwrap();
        assert!(app.clear_runs_inflight);
        app.flush_history().unwrap();
        assert_eq!(app.telemetry.reports().count(), 0);
        assert!(app.telemetry.selected_run().is_none());
        assert!(!app.clear_runs_inflight);
        app.flush_history().unwrap();
        drop(app);
        assert!(History::open(path.file()).unwrap().1.is_empty());
    }

    #[test]
    fn failed_worker_report_drains_queued_metrics_and_is_saved_on_tick() {
        use super::super::history::{tests::TestPath, History};
        let path = TestPath::new();
        let mut app = test_app();
        let (history, reports) = History::open(path.file()).unwrap();
        app.attach_history(history, reports);
        app.current_request = Some(7);
        let (sender, receiver) = mpsc::channel();
        app.metric_receiver = receiver;
        sender.send(metric("failed-run", "queued sample")).unwrap();
        app.apply_worker_event(WorkerEvent::ProcessingFailed {
            request_id: 7,
            run_id: "failed-run".into(),
            source: "fixture.wav".into(),
            error: "test failure".into(),
        });
        app.tick();
        app.history.as_mut().unwrap().flush().unwrap();
        let (_, restored) = History::open(path.file()).unwrap();
        assert_eq!(restored[0].error.as_deref(), Some("test failure"));
        assert_eq!(restored[0].events[0].name, "queued sample");
    }

    #[test]
    fn failed_clear_keeps_visible_reports_and_does_not_claim_success() {
        use super::super::history::{tests::TestPath, History};
        let path = TestPath::new();
        let mut app = test_app();
        let (history, reports) = History::open(path.file()).unwrap();
        app.attach_history(history, reports);
        app.telemetry.upsert_report(report("run-1", vec![]));
        app.flush_history().unwrap();
        std::fs::remove_file(path.file()).unwrap();
        std::fs::create_dir(path.file()).unwrap();
        app.telemetry_tab = TelemetryTab::Runs;
        app.handle_key(KeyCode::Char('c')).unwrap();
        app.handle_key(KeyCode::Char('y')).unwrap();
        assert!(app.flush_history().is_err());
        assert!(app.telemetry.report("run-1").is_some());
        assert!(app.status_message.contains("history"));
        assert!(!app.status_message.contains("all saved runs cleared"));
    }

    #[test]
    fn report_navigation_preserves_selection_and_resets_on_run_change() {
        let mut app = test_app();
        app.telemetry.push(metric("live", "live_cpu"));
        let live = app.telemetry.selected_metric("");
        app.telemetry.upsert_report(report(
            "old",
            vec![metric("old", "alpha"), metric("old", "beta")],
        ));
        app.telemetry
            .upsert_report(report("other", vec![metric("other", "gamma")]));
        app.handle_key(KeyCode::Char('3')).unwrap();
        app.handle_key(KeyCode::Enter).unwrap();
        assert!(app.run_detail);
        assert!(!app.historical_detail);
        assert_eq!(app.historical_metric.as_ref().unwrap().name, "alpha");
        app.handle_key(KeyCode::Char('j')).unwrap();
        assert_eq!(app.historical_metric.as_ref().unwrap().name, "beta");
        app.handle_key(KeyCode::PageDown).unwrap();
        assert_eq!(app.scroll, 5);
        app.handle_key(KeyCode::Enter).unwrap();
        assert!(app.historical_detail);
        app.handle_key(KeyCode::PageDown).unwrap();
        assert_eq!(app.historical_scroll, 5);
        app.handle_key(KeyCode::Esc).unwrap();
        assert!(app.run_detail);
        assert!(!app.historical_detail);
        assert_eq!(app.scroll, 5);
        assert_eq!(app.historical_metric.as_ref().unwrap().name, "beta");
        app.handle_key(KeyCode::Esc).unwrap();
        app.handle_key(KeyCode::Enter).unwrap();
        assert_eq!(app.historical_metric.as_ref().unwrap().name, "beta");
        app.handle_key(KeyCode::Enter).unwrap();
        app.handle_key(KeyCode::Char(']')).unwrap();
        assert!(!app.historical_detail);
        assert_eq!(app.scroll, 0);
        assert_eq!(app.historical_metric.as_ref().unwrap().name, "gamma");
        assert_eq!(app.telemetry.selected_metric(""), live);
        app.telemetry.select_run(Some("evicted".into()));
        app.sync_historical_selection();
        assert!(app.historical_metric.is_none());
        assert!(!app.historical_detail);
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
    fn metric_selection_is_independent_from_historical_run_selection() {
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
        assert_eq!(app.telemetry.series_index(&app.telemetry.series("")), 1);
        assert_eq!(app.telemetry.selected_run(), None);
        app.scroll = 50;
        app.handle_key(KeyCode::Char('4')).unwrap();
        assert_eq!(app.scroll, 0);
    }
}
