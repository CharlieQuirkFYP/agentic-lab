mod app;
mod config;
mod download;
mod events;
mod folder;
mod logs;
mod model_catalog;
mod native_logs;
mod rebuild;
mod telemetry;
mod ui;
mod worker;

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use metrics::MetricsHub;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;

use self::app::App;
use self::config::TuiConfig;
use self::events::WorkerCommand;
use self::model_catalog::ModelCatalog;
use self::telemetry::{MetricForwarder, METRIC_QUEUE_CAPACITY};
use self::worker::WorkerConfig;

pub struct TuiOptions {
    pub model_id: Option<String>,
    pub model_manifest: Option<PathBuf>,
    pub audio_directory: Option<PathBuf>,
    pub max_seconds: Option<u32>,
    pub language: Option<String>,
    pub dictionary: Option<Vec<String>>,
    pub reconfigure: bool,
}

pub fn run(options: TuiOptions) -> Result<()> {
    let config_loaded = config::load().context("could not load TUI configuration")?;
    let config_was_loaded = config_loaded.is_some();
    let mut config = config_loaded.unwrap_or_default();
    apply_options(&mut config, &options);
    let catalog = ModelCatalog::load(config.model_manifest.clone());
    let mut native_logs =
        native_logs::NativeLogs::start().context("could not capture native stderr")?;

    let metrics_hub = Arc::new(MetricsHub::new());
    let (metric_sender, metric_receiver) = mpsc::sync_channel(METRIC_QUEUE_CAPACITY);
    let metric_dropped = Arc::new(AtomicU64::new(0));
    let _metric_subscription = metrics_hub.subscribe(MetricForwarder::new(
        metric_sender,
        Arc::clone(&metric_dropped),
    ));

    let (worker_sender, worker_command_receiver) = mpsc::channel::<WorkerCommand>();
    let (worker_event_sender, worker_receiver) = mpsc::channel();
    let resource_context = Arc::new(Mutex::new(None));
    let resource_stop = Arc::new(AtomicBool::new(false));
    let resource_join = config.resource_sampling_enabled.then(|| {
        worker::spawn_resource_sampler(Arc::clone(&resource_context), Arc::clone(&resource_stop))
    });
    let worker_join = worker::spawn(
        worker_command_receiver,
        worker_event_sender,
        Arc::clone(&metrics_hub),
        WorkerConfig {
            metrics_enabled: config.metrics_enabled,
            resource_sampling_enabled: config.resource_sampling_enabled,
            resource_context,
        },
    );
    let terminal_shutdown_sender = worker_sender.clone();

    let mut app = App::new(
        &options,
        config,
        config_was_loaded,
        catalog,
        worker_sender,
        worker_receiver,
        metric_receiver,
        metric_dropped,
    );

    let mut terminal = match TerminalSession::enter() {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = terminal_shutdown_sender.send(WorkerCommand::Shutdown);
            let _ = worker_join.join();
            resource_stop.store(true, Ordering::Relaxed);
            if let Some(resource_join) = resource_join {
                let _ = resource_join.join();
            }
            return Err(error);
        }
    };
    app.start_initial_load();
    let loop_result = run_event_loop(&mut terminal.terminal, &mut app, &native_logs);
    app.build.take();
    let _ = terminal.terminal.show_cursor();
    app.shutdown_worker();
    let _ = worker_join.join();
    resource_stop.store(true, Ordering::Relaxed);
    if let Some(resource_join) = resource_join {
        let _ = resource_join.join();
    }
    drop(terminal);
    let capture_result = native_logs
        .finish(&mut app.logs)
        .context("could not restore native stderr");
    loop_result.and(capture_result)?;
    if let Some((binary, model_id)) = app.restart.take() {
        rebuild::restart(&binary, &app.config, &model_id)?;
    }
    Ok(())
}

fn apply_options(config: &mut TuiConfig, options: &TuiOptions) {
    if let Some(model_id) = options.model_id.as_ref() {
        config.selected_stt_model = model_id.clone();
    }
    if let Some(manifest) = options.model_manifest.as_ref() {
        config.model_manifest = manifest.clone();
    }
    if let Some(directory) = options.audio_directory.as_ref() {
        config.audio_directory = directory.clone();
    }
    if let Some(max_seconds) = options.max_seconds {
        config.max_seconds = max_seconds.max(1);
    }
    if options.language.is_some() {
        config.language = options.language.clone();
    }
    if let Some(dictionary) = options.dictionary.as_ref() {
        config.dictionary = dictionary.clone();
    }
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    native_logs: &native_logs::NativeLogs,
) -> Result<()> {
    while !app.should_quit {
        app.tick();
        native_logs.drain_into(&mut app.logs);
        terminal.draw(|frame| ui::draw(frame, app))?;
        if event::poll(Duration::from_millis(100)).context("could not read terminal input")? {
            let event = event::read().context("could not read terminal event")?;
            app.handle_terminal_event(event)?;
        }
    }
    Ok(())
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    _cleanup: TerminalCleanup,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode().context("could not enable terminal input")?;
        let cleanup = TerminalCleanup;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            drop(cleanup);
            return Err(error).context("could not enter the alternate screen");
        }
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).context("could not create ratatui terminal")?;
        terminal.clear().context("could not clear the TUI screen")?;
        terminal
            .hide_cursor()
            .context("could not hide the terminal cursor")?;
        Ok(Self {
            terminal,
            _cleanup: cleanup,
        })
    }
}

struct TerminalCleanup;

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}
