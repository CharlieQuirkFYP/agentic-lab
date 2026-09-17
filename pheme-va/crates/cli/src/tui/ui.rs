use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Axis, Block, Borders, Cell, Chart, Dataset, Gauge, List, ListItem, Paragraph, Row, Table, Tabs,
    Wrap,
};
use ratatui::Frame;

use super::app::{AdapterAvailability, App, Screen, TelemetryTab};
use super::folder::FolderEntryKind;
use super::logs::format_timestamp;
use super::model_catalog::ModelCatalog;
use super::telemetry::TelemetryStore;

const ACCENT: Color = Color::Cyan;
const MIN_WIDTH: u16 = 80;
const MIN_HEIGHT: u16 = 24;

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        frame.render_widget(
            Paragraph::new(format!(
                "Terminal too small for the Pheme VA TUI.\nResize to at least {MIN_WIDTH}x{MIN_HEIGHT}.\nCurrent size: {}x{}",
                area.width, area.height
            ))
            .block(Block::default().borders(Borders::ALL).title("PHEME VA"))
            .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    match app.screen {
        Screen::Welcome => draw_welcome(frame, app),
        Screen::Picker => draw_picker(frame, app),
        Screen::Loading => draw_loading(frame, app),
        Screen::Bench => draw_bench(frame, app),
        Screen::Folder => draw_folder(frame, app),
        Screen::Recording => draw_recording(frame, app),
        Screen::Processing => draw_processing(frame, app),
        Screen::Telemetry => draw_telemetry(frame, app),
        Screen::DirectoryInput => draw_directory_input(frame, app),
        Screen::Help => draw_help(frame, app),
        Screen::Error => draw_error(frame, app),
    }
}

fn draw_welcome(frame: &mut Frame<'_>, app: &App) {
    let area = centered(frame.area(), 82, 72);
    let body = vec![
        Line::from(Span::styled(
            "Local speech transcription test bench",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("The setup will:"),
        Line::from("  1. Load the model manifest"),
        Line::from("  2. Show which adapters are compiled, cached, or need preparation"),
        Line::from("  3. Let you choose a model ID"),
        Line::from("  4. Configure a local WAV folder and microphone workflow"),
        Line::from("  5. Open the transcription test bench"),
        Line::from(""),
        Line::from("Audio remains on this machine when using the local CLI."),
        Line::from(""),
        Line::from(Span::styled(
            "[Enter] continue    [Esc] exit",
            Style::default().fg(Color::Yellow),
        )),
    ];
    let title = if app.config_path.exists() {
        " PHEME VA / START "
    } else {
        " PHEME VA / FIRST-RUN SETUP "
    };
    frame.render_widget(
        Paragraph::new(Text::from(body))
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_picker(frame: &mut Frame<'_>, app: &App) {
    let outer = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(outer);
    draw_header(frame, app, chunks[0], "SELECT SPEECH MODEL");
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
        .split(chunks[1]);

    let items = app
        .catalog
        .entries
        .iter()
        .map(|entry| {
            let availability = app.adapter_availability(entry);
            let color = if availability.is_ready() {
                Color::Green
            } else {
                Color::Yellow
            };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{}  ", entry.manifest.id)),
                Span::styled(availability.label(), Style::default().fg(color)),
            ]))
        })
        .collect::<Vec<_>>();
    let mut state = ratatui::widgets::ListState::default();
    if !app.catalog.entries.is_empty() {
        state.select(Some(app.catalog_index.min(app.catalog.entries.len() - 1)));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("MANIFEST ENTRIES"),
            )
            .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
            .highlight_symbol("> "),
        body[0],
        &mut state,
    );

    let details = if let Some(entry) = app.catalog.entry(app.catalog_index) {
        let missing = if entry.missing_paths.is_empty() {
            "none".to_owned()
        } else {
            ModelCatalog::missing_summary(entry)
        };
        vec![
            Line::from(Span::styled(
                entry.manifest.id.clone(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("Family:       {}", entry.manifest.family)),
            Line::from(format!(
                "Runtime:      {}",
                entry.manifest.runtime.as_deref().unwrap_or("not specified")
            )),
            Line::from(format!(
                "Revision:     {}",
                entry
                    .manifest
                    .revision
                    .as_deref()
                    .unwrap_or("not specified")
            )),
            Line::from(format!("Model:        {}", entry.model_path.display())),
            Line::from(format!(
                "Size:         {}",
                entry
                    .manifest
                    .model_size
                    .as_deref()
                    .unwrap_or("not specified")
            )),
            Line::from(format!(
                "Adapter:      {}",
                app.adapter_availability(entry).label()
            )),
            Line::from(format!(
                "Artifacts:    {}",
                if entry.artifacts_available() {
                    "available"
                } else {
                    "missing"
                }
            )),
            Line::from(format!("Missing:      {missing}")),
            Line::from(format!(
                "Timestamps:   {}",
                yes_no(entry.manifest.timestamps)
            )),
            Line::from(format!(
                "Streaming:    {}",
                yes_no(entry.manifest.streaming)
            )),
            Line::from(""),
            Line::from(
                match app.adapter_availability(entry) {
                    AdapterAvailability::Unsupported => {
                        "Unsupported model family; add a matching adapter before selecting it."
                            .to_owned()
                    }
                    AdapterAvailability::ArtifactMissing => {
                        "[Enter] download the verified model artifacts automatically. [r] rescan."
                            .to_owned()
                    }
                    AdapterAvailability::Compiled => "[Enter] load this candidate".to_owned(),
                    AdapterAvailability::Cached => {
                        "[Enter] use the validated cached adapter; a quick launcher restart may occur.".to_owned()
                    }
                    AdapterAvailability::Checking => {
                        "[Enter] load or prepare this adapter; cache check is still running.".to_owned()
                    }
                    AdapterAvailability::Prepare => {
                        "[Enter] prepare adapter and restart automatically. Requires source checkout, Cargo and native toolchain; may download build dependencies/runtime packages. Settings and saved run reports retained; live metrics/logs reset. [Esc] back.".to_owned()
                    }
                },
            ),
        ]
    } else if let Some(error) = app.catalog.error.as_deref() {
        vec![
            Line::from(Span::styled(
                "Manifest error",
                Style::default().fg(Color::Red),
            )),
            Line::from(""),
            Line::from(error.to_owned()),
        ]
    } else {
        vec![Line::from("No model entries found.")]
    };
    frame.render_widget(
        Paragraph::new(Text::from(details))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("MODEL DETAILS"),
            )
            .wrap(Wrap { trim: false }),
        body[1],
    );
    frame.render_widget(
        Paragraph::new(Line::from(
            "[↑/↓] select  [Enter] load/use cache/prepare  [r] rescan  [Esc] back",
        )),
        chunks[2],
    );
    draw_error_line(frame, app, outer);
}

fn draw_loading(frame: &mut Frame<'_>, app: &App) {
    if let Some(download) = &app.download {
        let chunks = base_layout(frame.area());
        draw_header(frame, app, chunks[0], "DOWNLOADING MODEL");
        let percent = download.progress.percent.unwrap_or(0);
        let bar = progress_bar(percent, 32);
        frame.render_widget(Paragraph::new(format!("Downloading model {}\n\n{} {:>3}%\n\n{}\n\nThe verified partial file is retained if you cancel.\n\n[Esc] cancel download", download.model_id, bar, percent, download.progress.message)).block(Block::default().borders(Borders::ALL).title("MODEL DOWNLOAD")).wrap(Wrap { trim: false }), chunks[1]);
        draw_footer(
            frame,
            app,
            chunks[2],
            "[Esc] cancel download  [t] logs  [q] quit",
        );
        return;
    }
    if let Some(build) = &app.build {
        let chunks = base_layout(frame.area());
        draw_header(frame, app, chunks[0], "PREPARING BACKEND");
        let panels = Layout::vertical([Constraint::Length(5), Constraint::Min(3)]).split(chunks[1]);
        frame.render_widget(
            Paragraph::new(format!(
                "Model: {}\nChecking cached backend; Cargo runs only when needed.\nSaved run reports survive restart. Settings are retained. Current model stays available on failure.",
                build.model_id,
            ))
            .block(Block::default().borders(Borders::ALL).title("BACKEND PREPARATION"))
            .wrap(Wrap { trim: false }),
            panels[0],
        );
        draw_build_output(frame, app, panels[1]);
        draw_footer(
            frame,
            app,
            chunks[2],
            "[t] metrics/logs  [Esc] cancel build  [q] quit",
        );
        return;
    }
    let chunks = base_layout(frame.area());
    draw_header(frame, app, chunks[0], "LOADING MODEL");
    let model_id = app
        .pending_model_id
        .as_deref()
        .unwrap_or(&app.config.selected_stt_model);
    let entry = app.catalog.entry_by_id(model_id);
    let details = vec![
        Line::from(Span::styled(
            format!("Model ID:  {model_id}"),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "Family:    {}",
            entry
                .map(|entry| entry.manifest.family.as_str())
                .unwrap_or("unknown")
        )),
        Line::from(format!(
            "Runtime:   {}",
            entry
                .and_then(|entry| entry.manifest.runtime.as_deref())
                .unwrap_or("not specified")
        )),
        Line::from(""),
        Line::from("Loading candidate engine in the inference worker..."),
        Line::from("The terminal remains responsive while the model is loaded."),
        Line::from(""),
        Line::from(Span::styled(
            "[t] view metrics and logs",
            Style::default().fg(Color::Yellow),
        )),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(details))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("MODEL OPERATION"),
            )
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
    draw_footer(frame, app, chunks[2], "[t] metrics/logs  [q] quit");
}

fn draw_bench(frame: &mut Frame<'_>, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(frame.area());
    draw_header(frame, app, chunks[0], "TEST BENCH");

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(chunks[1]);
    let source_lines = vec![
        Line::from(Span::styled("INPUT SOURCE", Style::default().fg(ACCENT))),
        Line::from(""),
        Line::from("[l] Live microphone"),
        Line::from("[f] Select WAV file"),
        Line::from(""),
        Line::from("Folder:"),
        Line::from(app.folder.directory.display().to_string()),
        Line::from(format!("{} WAV file(s)", app.folder.wav_count())),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(source_lines))
            .block(Block::default().borders(Borders::ALL).title("SOURCE"))
            .wrap(Wrap { trim: true }),
        top[0],
    );

    let transcript = match app.result.as_ref() {
        Some(result)
            if result.result.status.as_str() == "speech" && !result.result.text.is_empty() =>
        {
            result.result.text.clone()
        }
        Some(result) => format!(
            "Status: {}\n\nNo final speech transcript was produced.\n\nGate: {:?}",
            result.result.status.as_str(),
            result.result.gate.decision
        ),
        None => "No result yet.\n\nChoose a source to test the selected model.".to_owned(),
    };
    frame.render_widget(
        Paragraph::new(transcript)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("FINAL TRANSCRIPT"),
            )
            .scroll((app.scroll, 0))
            .wrap(Wrap { trim: false }),
        top[1],
    );

    let detail_lines = if let Some(result) = app.result.as_ref() {
        vec![
            Line::from(format!(
                "Status: {}    Source: {}    Run: {}",
                result.result.status.as_str(),
                result.source,
                result.run_id
            )),
            Line::from(format!(
                "Audio: {:.2} s    Processing: {} ms    Language: {}",
                result.audio_duration_seconds,
                result.result.processing_time_ms,
                result.result.language.as_deref().unwrap_or("detected")
            )),
            Line::from(format!(
                "Gate: {:?}    Peak RMS: {:.4}    Recovery: {}",
                result.result.gate.decision,
                result.result.gate.peak_rms,
                yes_no(result.result.recovery_attempted)
            )),
        ]
    } else {
        vec![Line::from(format!(
            "Status: {}    Source: {}    Run: {}",
            if app.current_request.is_some() {
                "busy"
            } else {
                "idle"
            },
            app.current_source.as_deref().unwrap_or("none"),
            app.current_run.as_deref().unwrap_or("—")
        ))]
    };
    frame.render_widget(
        Paragraph::new(Text::from(detail_lines))
            .block(Block::default().borders(Borders::ALL).title("RUN DETAILS"))
            .wrap(Wrap { trim: false }),
        chunks[2],
    );

    let run_id = app.current_run.as_deref();
    let compact = [
        ("normalization", "audio_normalization_duration_ms"),
        ("gate", "speech_gate_duration_ms"),
        ("transcription", "transcription_duration_ms"),
        ("end-to-end", "end_to_end_request_duration_ms"),
    ]
    .iter()
    .map(|(label, name)| format!("{label}: {}", compact_metric(&app.telemetry, name, run_id)))
    .collect::<Vec<_>>()
    .join("   ");
    let compact = format!("{compact}\n{}", monitor_line(app));
    frame.render_widget(
        Paragraph::new(compact)
            .block(Block::default().borders(Borders::ALL).title("METRICS"))
            .wrap(Wrap { trim: false }),
        chunks[3],
    );
    draw_footer(
        frame,
        app,
        chunks[4],
        "[r] retry  [n] next file  [f] folder  [l] live  [m] model  [t] metrics/logs  [j/k] scroll  [?] help  [q] quit",
    );
    draw_error_line(frame, app, frame.area());
}

fn draw_folder(frame: &mut Frame<'_>, app: &App) {
    let chunks = base_layout(frame.area());
    draw_header(frame, app, chunks[0], "SELECT WAV FILE");
    let items = app
        .folder
        .entries
        .iter()
        .map(|entry| {
            let prefix = match entry.kind {
                FolderEntryKind::Parent => ".. ",
                FolderEntryKind::Directory => "▸ ",
                FolderEntryKind::Wav => "  ",
            };
            ListItem::new(format!("{prefix}{}", entry.name))
        })
        .collect::<Vec<_>>();
    let mut state = ratatui::widgets::ListState::default();
    if !app.folder.entries.is_empty() {
        state.select(Some(app.folder.selected));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", app.folder.directory.display())),
            )
            .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
            .highlight_symbol("> "),
        chunks[1],
        &mut state,
    );
    let message = app
        .folder
        .error
        .as_deref()
        .unwrap_or("Only WAV files are shown. Directories can be entered.");
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(message),
            Line::from(""),
            Line::from(format!("{} WAV file(s)", app.folder.wav_count())),
        ]))
        .block(Block::default().borders(Borders::ALL).title("FOLDER"))
        .wrap(Wrap { trim: false }),
        side_panel(chunks[1]),
    );
    draw_footer(
        frame,
        app,
        chunks[2],
        "[↑/↓] move  [Enter] transcribe/open  [d] directory  [r] refresh  [Esc] back",
    );
}

fn draw_recording(frame: &mut Frame<'_>, app: &App) {
    let chunks = base_layout(frame.area());
    draw_header(frame, app, chunks[0], "LIVE MICROPHONE");
    let elapsed = app
        .recording
        .as_ref()
        .map(|recording| recording.elapsed().as_secs_f64())
        .unwrap_or(0.0);
    let max_seconds = app.config.max_seconds.max(1) as f64;
    let ratio = (elapsed / max_seconds).min(1.0);
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Length(3),
            Constraint::Min(3),
        ])
        .split(chunks[1]);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from("Device: default input device"),
            Line::from(Span::styled(
                "Status: RECORDING",
                Style::default().fg(Color::Red),
            )),
            Line::from(format!(
                "Duration: {} / {}",
                format_duration(elapsed),
                format_duration(max_seconds)
            )),
            Line::from(monitor_line(app)),
            Line::from("Speak now."),
        ]))
        .block(Block::default().borders(Borders::ALL).title("CAPTURE")),
        body[0],
    );
    frame.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::ALL).title("TIME"))
            .gauge_style(Style::default().fg(Color::Red))
            .ratio(ratio),
        body[1],
    );
    frame.render_widget(
        Paragraph::new("Final transcription is produced after recording stops. Partial words are unavailable because the current models are not streaming transcribers.")
            .block(Block::default().borders(Borders::ALL).title("NOTE"))
            .wrap(Wrap { trim: false }),
        body[2],
    );
    draw_footer(
        frame,
        app,
        chunks[2],
        "[Enter/Space] stop  [Esc] discard  [q] quit",
    );
}

fn draw_processing(frame: &mut Frame<'_>, app: &App) {
    let chunks = base_layout(frame.area());
    draw_header(frame, app, chunks[0], "PROCESSING");
    let source = app.current_source.as_deref().unwrap_or("unknown source");
    let run = app.current_run.as_deref().unwrap_or("—");
    let body = vec![
        Line::from(format!("Source: {source}")),
        Line::from(format!("Run:    {run}")),
        Line::from(""),
        Line::from("Reading audio and running speech recognition..."),
        Line::from(monitor_line(app)),
        Line::from("The final result will appear when decoding completes."),
        Line::from(""),
        Line::from(Span::styled(
            "[t] view live metrics and logs",
            Style::default().fg(Color::Yellow),
        )),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(body))
            .block(Block::default().borders(Borders::ALL).title("REQUEST"))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
    draw_footer(
        frame,
        app,
        chunks[2],
        "[t] metrics/logs  [j/k] scroll  [q] quit",
    );
}

fn draw_telemetry(frame: &mut Frame<'_>, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(frame.area());
    draw_header(frame, app, chunks[0], "METRICS & LOGS");
    let titles = ["[1] Overview", "[2] Metrics", "[3] Runs", "[4] Logs"]
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>();
    frame.render_widget(
        Tabs::new(titles)
            .select(app.telemetry_tab.index())
            .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
            .divider(" "),
        chunks[1],
    );
    match app.telemetry_tab {
        TelemetryTab::Overview => draw_overview(frame, app, chunks[2]),
        TelemetryTab::Metrics => draw_metrics(frame, app, chunks[2]),
        TelemetryTab::Runs => {
            if app.run_detail {
                draw_run_report(frame, app, chunks[2]);
            } else {
                draw_runs(frame, app, chunks[2]);
            }
        }
        TelemetryTab::Logs => draw_logs(frame, app, chunks[2]),
    }
    let footer = if app.clear_runs_pending {
        "Clear ALL saved runs and transcripts? [y] confirm  [Esc] cancel".to_owned()
    } else if app.filter_editing {
        format!(
            "Search: {}_  [Enter/Esc] finish  [Backspace] delete",
            app.filter_query
        )
    } else if app.run_detail {
        format!("[j/k ↑/↓] metric  [Enter] detail  [Esc] {}  [n/N] run\n[PgUp/PgDn] {} scroll  [/] highlight  [c] clear all runs",
            if app.historical_detail { "report" } else { "runs" },
            if app.historical_detail { "detail" } else { "report" })
    } else {
        format!("[1-4] tab  [j/k ↑/↓] select/scroll  [Enter] open  [/] search  [x] clear  [Esc] back{}  Search: {}",
            if app.telemetry_tab == TelemetryTab::Logs { "  [c] clear logs" } else if app.telemetry_tab == TelemetryTab::Runs { "  [c] clear all runs" } else { "" },
            if app.filter_query.is_empty() { "(none)" } else { &app.filter_query })
    };
    frame.render_widget(
        Paragraph::new(footer).style(Style::default().fg(Color::Yellow)),
        chunks[3],
    );
}

fn draw_overview(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(3),
        Constraint::Length(4),
        Constraint::Length(5),
    ])
    .split(area);
    let model = app
        .active_model
        .as_ref()
        .map(|model| {
            format!(
                "{} / {} / {}",
                model.id,
                model.backend,
                model.runtime.as_deref().unwrap_or("runtime unknown")
            )
        })
        .unwrap_or_else(|| "No active model".into());
    let state = format!(
        "State: {} | Run: {}",
        app.status_message,
        app.current_run.as_deref().unwrap_or("none")
    );
    draw_text_panel(
        frame,
        app,
        chunks[0],
        &format!(
            "MODEL / CURRENT STATE / {} events / {} logs",
            app.telemetry.events().count(),
            app.logs.len()
        ),
        &[model, state],
        0,
    );
    let report = app
        .current_run
        .as_deref()
        .and_then(|run| app.telemetry.report(run));
    let transcript = report
        .map(|report| report.transcript.as_str())
        .filter(|text| !text.is_empty())
        .unwrap_or("No final transcript yet.");
    draw_text_panel(
        frame,
        app,
        chunks[1],
        "TRANSCRIPT / CURRENT RUN",
        &[transcript.into()],
        app.scroll,
    );
    let value = |name| {
        app.current_run
            .as_deref()
            .map(|run| compact_metric(&app.telemetry, name, Some(run)))
            .unwrap_or_else(|| "—".into())
    };
    draw_text_panel(
        frame,
        app,
        chunks[2],
        "PERFORMANCE / CURRENT RUN",
        &[
            format!(
                "Normalize {} | Gate {}",
                value("audio_normalization_duration_ms"),
                value("speech_gate_duration_ms")
            ),
            format!(
                "Transcribe {} | Total {}",
                value("transcription_duration_ms"),
                value("end_to_end_request_duration_ms")
            ),
        ],
        0,
    );
    let unavailable = app
        .telemetry
        .series("")
        .iter()
        .filter(|item| short_value(item.latest()) == "n/a")
        .count();
    let value = |name| compact_metric(&app.telemetry, name, None);
    draw_text_panel(
        frame,
        app,
        chunks[3],
        "RESOURCES / LATEST MONITOR SAMPLES",
        &[
            format!(
                "Process CPU {} | RAM {}",
                value("process_cpu_percent"),
                compact_metric_scoped(
                    &app.telemetry,
                    "ram_usage_bytes",
                    Some(metrics::MetricScope::Process),
                    None
                )
            ),
            format!(
                "GPU {} | Temp {} | Battery {}",
                value("gpu_usage_percent"),
                value("temperature_celsius"),
                value("battery_drain_percent")
            ),
            format!("{unavailable} series unavailable | [2] Metrics, Enter for reasons"),
        ],
        0,
    );
}

fn draw_text_panel(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    title: &str,
    lines: &[String],
    scroll: u16,
) {
    let text = lines
        .iter()
        .flat_map(|text| text.lines())
        .map(|line| highlighted_line(line, &app.filter_query, Style::default()))
        .collect::<Vec<_>>();
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
    let block = Block::default().borders(Borders::ALL).title(title);
    frame.render_widget(paragraph.scroll((scroll, 0)).block(block), area);
}

fn draw_metrics(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if app.metric_detail {
        draw_metric_detail(frame, app, area);
        return;
    }
    let series = app.telemetry.series("");
    let rows = app.telemetry.metric_rows("");
    let selected = app.telemetry.selected_metric_index(&rows);
    let rendered = rows
        .iter()
        .enumerate()
        .map(|(index, row)| match row {
            super::telemetry::MetricRow::Category(category) => Row::new(vec![
                Cell::from(format!("▸ {category}")),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
            ])
            .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
            super::telemetry::MetricRow::Metric(key) => {
                let item = series.iter().find(|item| item.key == *key);
                let stats = item.map(|item| item.stats());
                let style = if index == selected {
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                } else if !app.filter_query.is_empty()
                    && !item.is_some_and(|item| item.matches(&app.filter_query))
                {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                };
                let latest = item
                    .map(|item| {
                        let event = item.latest();
                        if event.value.is_none() || event.unavailable_reason.is_some() {
                            TelemetryStore::value_text(event)
                        } else {
                            stats
                                .as_ref()
                                .and_then(|stats| stats.latest)
                                .map(|value| format!("{value:.3}"))
                                .unwrap_or_else(|| TelemetryStore::value_text(event))
                        }
                    })
                    .unwrap_or_else(|| "—".to_owned());
                let min = stats
                    .as_ref()
                    .and_then(|stats| stats.minimum)
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "—".to_owned());
                let max = stats
                    .as_ref()
                    .and_then(|stats| stats.maximum)
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "—".to_owned());
                let avg = stats
                    .as_ref()
                    .and_then(|stats| stats.average)
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "—".to_owned());
                let count = stats
                    .map(|stats| stats.count.to_string())
                    .unwrap_or_else(|| "0".to_owned());
                Row::new(vec![
                    Cell::from(highlighted_line(
                        &format!("  {}", key.name),
                        &app.filter_query,
                        style,
                    )),
                    Cell::from(highlighted_line(&latest, &app.filter_query, style)),
                    Cell::from(highlighted_line(&min, &app.filter_query, style)),
                    Cell::from(highlighted_line(&max, &app.filter_query, style)),
                    Cell::from(highlighted_line(&avg, &app.filter_query, style)),
                    Cell::from(count),
                ])
                .style(style)
            }
        })
        .collect::<Vec<_>>();
    let mut state = ratatui::widgets::TableState::default().with_selected(Some(selected));
    frame.render_stateful_widget(
        Table::new(
            rendered,
            [
                Constraint::Percentage(38),
                Constraint::Length(13),
                Constraint::Length(13),
                Constraint::Length(13),
                Constraint::Length(13),
                Constraint::Length(7),
            ],
        )
        .header(
            Row::new(vec!["Category / Metric", "Now", "Min", "Max", "Avg", "N"])
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("LIVE METRICS / {}", live_context(app))),
        )
        .column_spacing(1),
        area,
        &mut state,
    );
}

fn draw_runs(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let reports = app.telemetry.reports().collect::<Vec<_>>();
    let selected = app.telemetry.selected_run();
    let rows = reports.iter().map(|report| {
        let style = if Some(report.run_id.as_str()) == selected {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        Row::new(vec![
            Cell::from(highlighted_line(&report.run_id, &app.filter_query, style)),
            Cell::from(highlighted_line(
                report.status_text(),
                &app.filter_query,
                style,
            )),
            Cell::from(highlighted_line(&report.source, &app.filter_query, style)),
            Cell::from(highlighted_line(&report.model_id, &app.filter_query, style)),
            Cell::from(format!("{:.2}s", report.audio_duration_seconds)),
        ])
        .style(style)
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(12),
                Constraint::Length(10),
                Constraint::Percentage(28),
                Constraint::Percentage(28),
                Constraint::Length(10),
            ],
        )
        .header(
            Row::new(vec!["Run", "Status", "Source", "Model", "Audio"])
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("RUNS / SELECT A RUN THEN PRESS ENTER"),
        ),
        area,
    );
}

fn draw_metric_detail(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(super::telemetry::MetricRow::Metric(key)) = app.telemetry.selected_metric("") else {
        frame.render_widget(
            Paragraph::new("Select a metric row first.")
                .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    };
    let series = app.telemetry.series("");
    let Some(series) = series.get(app.telemetry.series_index(&series)) else {
        return;
    };
    let points = series.history(app.telemetry.run_origin_ms());
    let title = format!("LIVE / {} / {}", key.category, key.name);
    if points.len() < 2 || points.first().map(|point| point.0) == points.last().map(|point| point.0)
    {
        draw_text_panel(
            frame,
            app,
            area,
            "LIVE METRIC DETAIL",
            &[
                title,
                format!("Scope: {:?} | Source: {}", key.scope, key.source),
                format!("Now: {}", TelemetryStore::value_text(series.latest())),
                if points.is_empty() {
                    "No numeric samples.".into()
                } else {
                    "Scalar sample: no time history to plot.".into()
                },
            ],
            app.scroll,
        );
        return;
    }
    let stats = series.stats();
    let ymin = points
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let ymax = points
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let pad = if ymin == ymax {
        (ymin.abs() * 0.05).max(0.001)
    } else {
        0.0
    };
    let xmax = points.last().map(|point| point.0).unwrap_or(1.0).max(0.001);
    let inner = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .inner(area);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title("LIVE METRIC DETAIL"),
        area,
    );
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(4)])
        .split(inner);
    frame.render_widget(
        Paragraph::new(format!(
            "Run: {}   Scope: {:?}   Source: {}   Unit: {}   LIVE\nNow {}  Min {}  Max {}  Avg {}  Samples {}",
            live_context(app),
            key.scope,
            key.source,
            TelemetryStore::unit_text(series.latest()),
            short_value(series.latest()),
            metric_number(stats.minimum, key.unit),
            metric_number(stats.maximum, key.unit),
            metric_number(stats.average, key.unit),
            stats.count
        )),
        parts[0],
    );
    frame.render_widget(
        Chart::new(vec![Dataset::default()
            .name(key.name.clone())
            .data(&points)
            .marker(ratatui::symbols::Marker::Braille)
            .style(Style::default().fg(ACCENT))])
        .x_axis(
            Axis::default()
                .bounds([0.0, xmax])
                .labels(["0s".to_owned(), format!("{xmax:.1}s")]),
        )
        .y_axis(
            Axis::default()
                .bounds([ymin - pad, ymax + pad])
                .labels([format!("{ymin:.2}"), format!("{ymax:.2}")]),
        ),
        parts[1],
    );
}

fn draw_run_report(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(report) = app.telemetry.selected_report() else {
        frame.render_widget(
            Paragraph::new("No run selected.").block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    };
    let series = app.telemetry.series_for_run(&report.run_id);
    let selected = series
        .iter()
        .position(|item| Some(&item.key) == app.historical_metric.as_ref())
        .unwrap_or(0);
    if app.historical_detail {
        if let Some(item) = series.get(selected) {
            draw_historical_detail(frame, app, area, item, &report.run_id);
        }
        return;
    }
    // A full-width snapshot keeps all statistic columns readable on an 80-column terminal.
    let wide = area.width >= 140;
    let chunks = Layout::default()
        .direction(if wide {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints(if wide {
            [Constraint::Percentage(40), Constraint::Percentage(60)]
        } else {
            [Constraint::Percentage(50), Constraint::Percentage(50)]
        })
        .split(area);
    let lines = vec![
        format!("Run: {} / {}", report.run_id, report.status_text()),
        format!("Model: {}", report.model_id),
        format!("Family: {}", report.model_family),
        format!("Backend: {}", report.backend),
        format!(
            "Runtime: {}",
            report.runtime.as_deref().unwrap_or("unknown")
        ),
        format!(
            "Revision: {}",
            report.revision.as_deref().unwrap_or("unknown")
        ),
        format!("Started: {}", report_time(report.started_at_ms)),
        format!(
            "Finished: {}",
            report
                .finished_at_ms
                .map(report_time)
                .unwrap_or_else(|| "pending".into())
        ),
        format!("Source: {}", report.source),
        format!(
            "Language: {}",
            report.language.as_deref().unwrap_or("unknown")
        ),
        format!("Audio: {:.2} s", report.audio_duration_seconds),
        format!("Segments: {}", report.segment_count),
        format!("Samples: {}", report.samples().len()),
        format!("Gate: {}", report.gate_decision),
        format!("Error: {}", report.error.as_deref().unwrap_or("none")),
        "TRANSCRIPT".into(),
        if report.transcript.is_empty() {
            "No final transcript.".into()
        } else {
            report.transcript.clone()
        },
        "RAW TRANSCRIPT".into(),
        if report.raw_transcript.is_empty() {
            "No raw transcript.".into()
        } else {
            report.raw_transcript.clone()
        },
    ];
    draw_text_panel(
        frame,
        app,
        chunks[0],
        "REPORT / PgUp PgDn scroll",
        &lines,
        app.scroll,
    );
    if series.is_empty() {
        draw_text_panel(
            frame,
            app,
            chunks[1],
            "RUN SNAPSHOT",
            &["No retained metric samples for this run.".into()],
            0,
        );
        return;
    }
    let rows = series.iter().map(|item| {
        let stats = item.stats();
        let number = |value| metric_number(value, item.key.unit);
        let values = [
            short_value(item.latest()),
            number(stats.minimum),
            number(stats.maximum),
            number(stats.average),
            stats.count.to_string(),
        ];
        let name = format!(
            "{}\n{:?} / {}",
            item.key.name, item.key.scope, item.key.source
        );
        let style = if item.matches(&app.filter_query) {
            Style::default()
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut cells = vec![Cell::from(Text::from(
            name.lines()
                .map(|line| highlighted_line(line, &app.filter_query, style))
                .collect::<Vec<_>>(),
        ))];
        cells.extend(
            values
                .iter()
                .map(|value| Cell::from(highlighted_line(value, &app.filter_query, style))),
        );
        Row::new(cells).height(2)
    });
    let mut state = app.run_table.borrow_mut();
    state.select(Some(selected));
    frame.render_stateful_widget(
        Table::new(
            rows,
            [
                Constraint::Min(20),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(4),
            ],
        )
        .header(
            Row::new(["Metric / Scope / Source", "Now", "Min", "Max", "Avg", "N"])
                .style(Style::default().fg(ACCENT)),
        )
        .row_highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
        .highlight_symbol("> ")
        .block(Block::default().borders(Borders::ALL).title(format!(
            "RUN SNAPSHOT / {} / {}/{} / N=events",
            report.run_id,
            selected + 1,
            series.len()
        ))),
        chunks[1],
        &mut state,
    );
}

fn report_time(timestamp: u64) -> String {
    format!(
        "{} UTC (epoch {} ms)",
        format_timestamp(timestamp),
        timestamp
    )
}

fn draw_historical_detail(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    item: &super::telemetry::MetricSeries<'_>,
    run: &str,
) {
    let points = item.history(app.telemetry.run_origin_for(run));
    let stats = item.stats();
    let mut lines = vec![
        format!("Run: {run} / HISTORICAL"),
        format!("Metric: {}", item.key.name),
        format!(
            "Scope: {:?} | Source: {} | Unit: {}",
            item.key.scope,
            item.key.source,
            TelemetryStore::unit_text(item.latest())
        ),
        format!("Now: {}", TelemetryStore::value_text(item.latest())),
        format!(
            "Min {} | Max {} | Avg {} | N {} events",
            metric_number(stats.minimum, item.key.unit),
            metric_number(stats.maximum, item.key.unit),
            metric_number(stats.average, item.key.unit),
            stats.count
        ),
    ];
    for event in &item.samples {
        if short_value(event) == "n/a" {
            let reason = TelemetryStore::value_text(event);
            if !lines.contains(&reason) {
                lines.push(reason);
            }
        }
    }
    // A single scalar or status is not a time series. Never manufacture a second point.
    if matches!(
        item.key.unit,
        metrics::MetricUnit::Status | metrics::MetricUnit::Boolean
    ) || points.len() < 2
        || points.first().map(|point| point.0) == points.last().map(|point| point.0)
    {
        lines.push("No time history: scalar, status or unavailable samples (not plotted).".into());
        draw_text_panel(
            frame,
            app,
            area,
            "HISTORICAL METRIC DETAIL",
            &lines,
            app.historical_scroll,
        );
        return;
    }
    let chunks =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);
    draw_text_panel(
        frame,
        app,
        chunks[0],
        "HISTORICAL METRIC DETAIL / PgUp PgDn",
        &lines,
        app.historical_scroll,
    );
    let min = points
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let max = points
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let pad = if min == max {
        (min.abs() * 0.05).max(0.001)
    } else {
        (max - min) * 0.05
    };
    let xmax = points.last().map(|point| point.0).unwrap_or(1.0).max(0.001);
    frame.render_widget(
        Chart::new(vec![Dataset::default()
            .data(&points)
            .marker(ratatui::symbols::Marker::Braille)
            .style(Style::default().fg(ACCENT))])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("HISTORY / numeric samples only"),
        )
        .x_axis(
            Axis::default()
                .title("seconds since first retained run event")
                .bounds([0.0, xmax])
                .labels(["0s".into(), format!("{xmax:.2}s")]),
        )
        .y_axis(Axis::default().bounds([min - pad, max + pad]).labels([
            metric_number(Some(min), item.key.unit),
            metric_number(Some(max), item.key.unit),
        ])),
        chunks[1],
    );
}

fn live_context(app: &App) -> String {
    match (app.telemetry.active_run(), app.current_request.is_some()) {
        (Some(run), true) => format!("{run} BUSY"),
        (Some(run), false) => format!("{run} COMPLETE"),
        _ => "idle".to_owned(),
    }
}

fn monitor_line(app: &App) -> String {
    let value = |name| compact_metric(&app.telemetry, name, None);
    format!(
        "CPU {}   RAM {}   GPU {}   TEMP {}   BATTERY {}",
        value("process_cpu_percent"),
        compact_metric_scoped(
            &app.telemetry,
            "ram_usage_bytes",
            Some(metrics::MetricScope::Process),
            None
        ),
        value("gpu_usage_percent"),
        value("temperature_celsius"),
        value("battery_drain_percent")
    )
}

fn highlighted_line(text: &str, query: &str, base: Style) -> Line<'static> {
    let query = query.trim();
    if query.is_empty() {
        return Line::from(Span::styled(text.to_owned(), base));
    }
    let lower = text.to_ascii_lowercase();
    let needle = query.to_ascii_lowercase();
    let mut spans = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find(&needle) {
        let start = cursor + relative;
        if start > cursor {
            spans.push(Span::styled(text[cursor..start].to_owned(), base));
        }
        let end = start + needle.len();
        spans.push(Span::styled(
            text[start..end].to_owned(),
            base.fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        cursor = end;
    }
    if spans.is_empty() {
        spans.push(Span::styled(text.to_owned(), base));
    } else if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_owned(), base));
    }
    Line::from(spans)
}

fn draw_logs(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let logs = app.logs.filtered("");
    let header = Row::new(vec!["Time", "Level", "Component", "Message"])
        .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD));
    let rows = logs
        .iter()
        .skip(app.scroll as usize)
        .take(area.height.saturating_sub(4) as usize)
        .map(|entry| {
            let color = match entry.level {
                super::logs::LogLevel::Info => Color::White,
                super::logs::LogLevel::Warn => Color::Yellow,
                super::logs::LogLevel::Error => Color::Red,
            };
            Row::new(vec![
                Cell::from(format_timestamp(entry.timestamp_ms)),
                Cell::from(entry.level.as_str()).style(Style::default().fg(color)),
                Cell::from(highlighted_line(
                    &entry.component,
                    &app.filter_query,
                    Style::default(),
                )),
                Cell::from(highlighted_line(
                    &entry.message,
                    &app.filter_query,
                    Style::default(),
                )),
            ])
        });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(13),
                Constraint::Length(7),
                Constraint::Length(18),
                Constraint::Min(30),
            ],
        )
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" LOGS / all runs / {} entries ", logs.len())),
        )
        .column_spacing(1),
        area,
    );
}

fn draw_directory_input(frame: &mut Frame<'_>, app: &App) {
    let chunks = base_layout(frame.area());
    draw_header(frame, app, chunks[0], "CHANGE AUDIO DIRECTORY");
    let mut lines = vec![
        Line::from("Enter a directory containing WAV samples:"),
        Line::from(""),
        Line::from(Span::styled(
            format!("{}_", app.directory_input),
            Style::default().fg(ACCENT),
        )),
    ];
    if let Some(error) = app.directory_input_error.as_deref() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            error,
            Style::default().fg(Color::Red),
        )));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title("DIRECTORY"))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
    draw_footer(
        frame,
        app,
        chunks[2],
        "[Enter] apply  [Backspace] delete  [Esc] cancel",
    );
}

fn draw_help(frame: &mut Frame<'_>, app: &App) {
    let chunks = base_layout(frame.area());
    draw_header(frame, app, chunks[0], "HELP");
    let lines = vec![
        Line::from(Span::styled("Test bench", Style::default().fg(ACCENT))),
        Line::from("[l] record microphone    [f] select WAV    [n] next WAV"),
        Line::from("[r] retry source        [m] switch model  [t] telemetry"),
        Line::from("[j/k] scroll transcript/details   [?] help   [q] quit"),
        Line::from(""),
        Line::from(Span::styled("Telemetry", Style::default().fg(ACCENT))),
        Line::from("[1] overview  [2] live metrics  [3] runs/reports  [4] logs"),
        Line::from("[j/k or ↑/↓] select metrics/runs, otherwise scroll"),
        Line::from("[ and ] previous/next run  [f or /] live shared filter  [x] reset"),
        Line::from("[Enter/Esc] finish search  [c] clear logs / clear runs (confirm)  [Esc] back"),
        Line::from("Filter matches category/name/scope/source/unit/run/value/reason; history stays intact."),
        Line::from("Live graphs and run reports show numeric samples; gaps are not zero or interpolated."),
        Line::from(""),
        Line::from(Span::styled("Model availability", Style::default().fg(ACCENT))),
        Line::from("Compiled adapters load directly; validated cached adapters restart quickly when needed."),
        Line::from("Streaming partial words are unavailable for the current full-clip transcriber contract."),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title("KEYS"))
            .scroll((app.scroll, 0))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
    draw_footer(frame, app, chunks[2], "[Esc] back  [?] close  [q] quit");
}

fn draw_error(frame: &mut Frame<'_>, app: &App) {
    let chunks = base_layout(frame.area());
    draw_header(frame, app, chunks[0], "OPERATION ERROR");
    let mut lines = vec![
        Line::from(Span::styled(
            app.error_message
                .as_deref()
                .unwrap_or("an unknown operation error occurred"),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    if let Some(active) = app.active_model.as_ref() {
        lines.push(Line::from(format!(
            "Current active model remains: {}",
            active.id
        )));
    }
    if let Some(requested) = app.pending_model_id.as_deref() {
        lines.push(Line::from(format!("Requested model: {requested}")));
    }
    lines.extend([
        Line::from(""),
        Line::from("[r] retry  [m] choose another model  [Esc] return"),
    ]);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("RECOVERABLE ERROR"),
            )
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
    draw_footer(
        frame,
        app,
        chunks[2],
        "[r] retry  [m] model  [Esc] back  [q] quit",
    );
}

fn draw_header(frame: &mut Frame<'_>, app: &App, area: Rect, title: &str) {
    let model = app
        .active_model
        .as_ref()
        .map(|model| {
            format!(
                "{} / {} / {} {}",
                model.id,
                model.family,
                model.runtime.as_deref().unwrap_or("runtime unknown"),
                if app.current_request.is_some() {
                    "BUSY"
                } else {
                    "READY"
                }
            )
        })
        .unwrap_or_else(|| "no active model".to_owned());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" PHEME VA / {title} "),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(model, Style::default().fg(Color::White)),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn draw_footer(frame: &mut Frame<'_>, _app: &App, area: Rect, text: &str) {
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(Color::Yellow),
        )))
        .style(Style::default().fg(Color::Yellow)),
        area,
    );
}

fn draw_error_line(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if let Some(error) = app.error_message.as_deref() {
        let line = Paragraph::new(Line::from(Span::styled(
            format!(" Error: {error}"),
            Style::default().fg(Color::Red),
        )));
        let y = area.bottom().saturating_sub(1);
        frame.render_widget(line, Rect::new(area.x, y, area.width, 1));
    }
}

fn base_layout(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area)
}

fn side_panel(area: Rect) -> Rect {
    Rect::new(
        area.x + area.width.saturating_mul(56) / 100,
        area.y,
        area.width.saturating_mul(44) / 100,
        area.height,
    )
}

fn progress_bar(percent: u8, width: usize) -> String {
    let filled = width.saturating_mul(percent as usize) / 100;
    format!(
        "{}{}",
        "|".repeat(filled),
        ".".repeat(width.saturating_sub(filled))
    )
}

fn draw_build_output(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let logs = app.logs.filtered("");
    // Start at the most recent preparation request, not a previous build's output.
    let start = logs
        .iter()
        .rposition(|entry| entry.component == "adapter-build")
        .unwrap_or(0);
    let lines = logs[start..]
        .iter()
        .filter(|entry| matches!(entry.component.as_str(), "adapter-build" | "native-stderr"))
        .flat_map(|entry| entry.message.lines())
        .collect::<Vec<_>>();
    let visible = area.height.saturating_sub(2) as usize;
    let lines = lines
        .iter()
        .skip(lines.len().saturating_sub(visible))
        .map(|line| Line::from((*line).to_owned()))
        .collect::<Vec<_>>();
    let lines = if lines.is_empty() {
        vec![Line::from("Waiting for cache lookup / compiler output…")]
    } else {
        lines
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title("LIVE COMPILER OUTPUT / auto-follow / [t] full logs"),
        ),
        area,
    );
}

fn centered(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let width = (u32::from(area.width) * u32::from(width_percent.min(100)) / 100) as u16;
    let height = (u32::from(area.height) * u32::from(height_percent.min(100)) / 100) as u16;
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn compact_metric(store: &TelemetryStore, name: &str, run_id: Option<&str>) -> String {
    compact_metric_scoped(store, name, None, run_id)
}

fn compact_metric_scoped(
    store: &TelemetryStore,
    name: &str,
    scope: Option<metrics::MetricScope>,
    run_id: Option<&str>,
) -> String {
    let Some(event) = store.latest_value(name, scope, run_id) else {
        return "—".to_owned();
    };
    short_value(event)
}

fn short_value(event: &metrics::MetricEvent) -> String {
    if event.unavailable_reason.is_some() {
        return "n/a".into();
    }
    match &event.value {
        Some(metrics::MetricValue::Number(value)) if value.is_finite() => {
            metric_number(Some(*value), event.unit)
        }
        Some(metrics::MetricValue::Integer(value)) => {
            metric_number(Some(*value as f64), event.unit)
        }
        Some(metrics::MetricValue::Text(value)) => value.clone(),
        Some(metrics::MetricValue::Boolean(value)) => value.to_string(),
        _ => "n/a".into(),
    }
}

fn metric_number(value: Option<f64>, unit: metrics::MetricUnit) -> String {
    let Some(value) = value.filter(|value| value.is_finite()) else {
        return "—".into();
    };
    match unit {
        metrics::MetricUnit::Bytes => {
            let (scale, suffix) = if value.abs() >= 1024.0 * 1024.0 * 1024.0 {
                (1024.0 * 1024.0 * 1024.0, "GiB")
            } else if value.abs() >= 1024.0 * 1024.0 {
                (1024.0 * 1024.0, "MiB")
            } else if value.abs() >= 1024.0 {
                (1024.0, "KiB")
            } else {
                (1.0, "B")
            };
            format!("{:.1}{suffix}", value / scale)
        }
        metrics::MetricUnit::Milliseconds if value.abs() >= 60_000.0 => {
            format!("{:.2}min", value / 60_000.0)
        }
        metrics::MetricUnit::Milliseconds if value.abs() >= 1000.0 => {
            format!("{:.2}s", value / 1000.0)
        }
        metrics::MetricUnit::Count => format!("{value:.0}"),
        _ => format!("{value:.2}{}", super::telemetry::unit_text(unit)),
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn format_duration(seconds: f64) -> String {
    format!(
        "{:02}:{:02}.{:01}",
        (seconds as u64) / 60,
        (seconds as u64) % 60,
        (seconds * 10.0) as u64 % 10
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_panel_follows_current_build_output() {
        let mut app = super::super::app::tests::test_app();
        app.logs.info("native-stderr", "stale output");
        app.logs.info("adapter-build", "preparing model");
        for index in 0..30 {
            app.logs
                .info("native-stderr", format!("Compiling crate_{index}"));
        }
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 10)).unwrap();
        terminal
            .draw(|frame| draw_build_output(frame, &app, frame.area()))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("LIVE COMPILER OUTPUT"));
        assert!(text.contains("Compiling crate_29"));
        assert!(!text.contains("stale output"));
        assert!(!text.contains("Compiling crate_0"));
    }

    #[test]
    fn runs_clear_confirmation_is_visible() {
        let mut app = super::super::app::tests::test_app();
        app.telemetry_tab = TelemetryTab::Runs;
        app.clear_runs_pending = true;
        let text = rendered(&app, 80, 24);
        assert!(text.contains("Clear ALL saved runs and transcripts?"));
        assert!(text.contains("[y] confirm"));
    }

    #[test]
    fn picker_shows_validated_cached_adapter() {
        let mut app = super::super::app::tests::test_app();
        app.screen = Screen::Picker;
        let manifest: crate::model::ModelEntry = toml::from_str(
            r#"
            id = "fixture"
            family = "whisper"
            model = "fixture.bin"
            "#,
        )
        .unwrap();
        app.catalog
            .entries
            .push(super::super::model_catalog::CatalogEntry {
                manifest,
                model_path: "fixture.bin".into(),
                missing_paths: Vec::new(),
                // Force the fixture to represent a featureless launcher for the
                // same UI behavior under feature-enabled test builds.
                adapter_compiled: false,
            });
        app.adapter_cache
            .insert("whisper".into(), super::super::rebuild::CacheStatus::Cached);
        let text = rendered(&app, 100, 30);
        assert!(text.contains("fixture  cached"));
        assert!(text.contains("Adapter:      cached"));
        assert!(!text.contains("not compiled"));
    }

    fn rendered(app: &App, width: u16, height: u16) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn shared_filter_applies_to_all_tabs_and_selected_run_is_not_current_run() {
        let mut app = super::super::app::tests::test_app();
        app.current_run = Some("current-run".into());
        app.telemetry
            .push(super::super::app::tests::metric("selected-run", "keep_cpu"));
        app.telemetry.push(super::super::app::tests::metric(
            "selected-run",
            "hide_memory",
        ));
        app.telemetry
            .push(super::super::app::tests::metric("current-run", "wrong_run"));
        app.logs.info("test", "keep_log");
        app.logs.info("test", "hide_log");
        app.filter_query = "keep".into();
        for tab in [
            TelemetryTab::Metrics,
            TelemetryTab::Runs,
            TelemetryTab::Logs,
        ] {
            app.telemetry_tab = tab;
            let text = rendered(&app, 100, 30);
            assert!(
                text.contains("hide_memory")
                    || matches!(tab, TelemetryTab::Runs | TelemetryTab::Logs)
            );
            assert!(text.contains("hide_log") || tab != TelemetryTab::Logs);
            assert!(
                text.contains("wrong_run")
                    || matches!(tab, TelemetryTab::Runs | TelemetryTab::Logs)
            );
            if tab == TelemetryTab::Logs {
                assert!(text.contains("keep_log"));
            } else if tab != TelemetryTab::Runs {
                assert!(text.contains("keep_cpu") || tab == TelemetryTab::Overview);
            }
        }
    }

    fn press(app: &mut App, code: ratatui::crossterm::event::KeyCode) {
        app.handle_terminal_event(ratatui::crossterm::event::Event::Key(
            ratatui::crossterm::event::KeyEvent::new(
                code,
                ratatui::crossterm::event::KeyModifiers::NONE,
            ),
        ))
        .unwrap();
    }

    #[test]
    fn overview_is_compact_current_and_highlighted() {
        use super::super::app::tests::{metric, report, test_app};
        let mut app = test_app();
        app.current_run = Some("current".into());
        app.telemetry.upsert_report(report("current", vec![]));
        app.telemetry.upsert_report(report("old", vec![]));
        app.telemetry.select_run(Some("old".into()));
        app.active_model = Some(super::super::app::ActiveModel {
            id: "active-model".into(),
            family: "family".into(),
            backend: "backend".into(),
            runtime: None,
        });
        for name in [
            "gpu_usage_percent",
            "temperature_celsius",
            "battery_drain_percent",
        ] {
            let mut event = metric("monitor", name);
            event.value = None;
            event.unavailable_reason = Some("a long repeated sensor failure reason".into());
            app.telemetry.push(event);
        }
        app.filter_query = "Keep".into();
        let text = rendered(&app, 80, 24);
        for expected in [
            "MODEL / CURRENT STATE",
            "active-model",
            "Run: current",
            "Keep this transcript",
            "PERFORMANCE",
            "RESOURCES",
            "3 series unavailable",
        ] {
            assert!(text.contains(expected), "missing {expected}");
        }
        assert!(!text.contains("long repeated"));
        assert!(!text.contains("gpu_usage_percent"));
        let line = highlighted_line("Keep this transcript", &app.filter_query, Style::default());
        assert_eq!(line.spans[0].style.bg, Some(Color::Yellow));
    }

    #[test]
    fn report_snapshot_scrolls_all_frozen_metrics_and_opens_only_selected_history() {
        use super::super::app::tests::{metric, report, test_app};
        use ratatui::crossterm::event::KeyCode;
        let mut app = test_app();
        let mut events = Vec::new();
        for index in 0..30 {
            let name = format!("metric_{index:02}");
            events.push(metric("old", &name));
            let mut second = metric("old", &name);
            second.timestamp_ms = 2000;
            second.value = Some(metrics::MetricValue::Number(0.75));
            events.push(second);
        }
        app.telemetry.upsert_report(report("old", events));
        app.telemetry
            .push(metric("old", "late_event_not_in_report"));
        app.telemetry.push(metric("current", "wrong_run"));
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Enter);
        for (width, height) in [(80, 24), (160, 40)] {
            let text = rendered(&app, width, height);
            for expected in [
                "REPORT",
                "RUN SNAPSHOT",
                "Now",
                "Min",
                "Max",
                "Avg",
                "N",
                "Process",
                "metric_00",
                "0.25%",
                "0.75%",
                "0.50%",
            ] {
                assert!(text.contains(expected), "missing {expected} at {width}");
            }
            assert!(!text.contains("HISTORY /"));
            assert!(!text.contains("wrong_run"));
            assert!(!text.contains("late_event"));
        }
        press(&mut app, KeyCode::PageDown);
        let text = rendered(&app, 80, 24);
        assert!(text.contains("Revision: abc123"));
        assert!(text.contains("Started: 00:00:01.000 UTC"));
        assert!(text.contains("Finished: 00:00:03.500 UTC"));
        for _ in 0..29 {
            press(&mut app, KeyCode::Char('j'));
        }
        assert!(rendered(&app, 80, 24).contains("metric_29"));
        assert!(app.run_table.borrow().offset() > 0);
        press(&mut app, KeyCode::Enter);
        let text = rendered(&app, 80, 24);
        assert!(text.contains("Metric: metric_29"));
        assert!(text.contains("HISTORY / numeric samples only"));
        press(&mut app, KeyCode::Esc);
        assert!(rendered(&app, 80, 24).contains("metric_29"));
    }

    #[test]
    fn missing_latest_sample_keeps_numeric_history_without_faking_now() {
        use super::super::app::tests::{metric, report, test_app};
        use ratatui::crossterm::event::KeyCode;
        let mut app = test_app();
        let first = metric("old", "cpu");
        let mut second = first.clone();
        second.timestamp_ms = 2000;
        second.value = Some(metrics::MetricValue::Number(-0.25));
        let mut missing = first.clone();
        missing.timestamp_ms = 3000;
        missing.value = None;
        missing.unavailable_reason = Some("sensor disconnected".into());
        app.telemetry
            .upsert_report(report("old", vec![first, second, missing]));
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Enter);
        let text = rendered(&app, 80, 24);
        assert!(text.contains("n/a"));
        assert!(text.contains("-0.25%"));
        assert!(text.contains("0.25%"));
        assert!(!text.contains("sensor disconnected"));
        press(&mut app, KeyCode::Enter);
        assert!(rendered(&app, 80, 24).contains("HISTORY / numeric samples only"));
        press(&mut app, KeyCode::PageDown);
        assert!(rendered(&app, 80, 24).contains("sensor disconnected"));
    }

    #[test]
    fn scalar_status_and_unavailable_details_never_invent_graphs_or_zeroes() {
        use super::super::app::tests::{metric, report, test_app};
        use ratatui::crossterm::event::KeyCode;
        let mut app = test_app();
        for value in [
            None,
            Some(metrics::MetricValue::Text("ready".into())),
            Some(metrics::MetricValue::Number(42.0)),
        ] {
            let mut event = metric("old", "sensor");
            event.value = value;
            event.unavailable_reason = event.value.is_none().then(|| "sensor offline".into());
            app.telemetry
                .upsert_report(report("old", vec![event.clone()]));
            app.telemetry_tab = TelemetryTab::Runs;
            app.run_detail = false;
            press(&mut app, KeyCode::Enter);
            let text = rendered(&app, 80, 24);
            assert!(text.contains(&short_value(&event)));
            assert!(!text.contains("0.00%"));
            press(&mut app, KeyCode::Enter);
            let text = rendered(&app, 80, 24);
            assert!(text.contains("No time history"));
            assert!(!text.contains("HISTORY / numeric"));
            if event.value.is_none() {
                assert!(text.contains("sensor offline"));
            }
        }
        assert_eq!(
            metric_number(Some(1_048_576.0), metrics::MetricUnit::Bytes),
            "1.0MiB"
        );
        assert_eq!(
            metric_number(Some(1500.0), metrics::MetricUnit::Milliseconds),
            "1.50s"
        );
        assert_eq!(metric_number(None, metrics::MetricUnit::Bytes), "—");
    }

    #[test]
    fn history_and_unavailable_reason_render_at_minimum_size() {
        let mut app = super::super::app::tests::test_app();
        app.telemetry_tab = TelemetryTab::Metrics;
        app.metric_detail = true;
        let mut event = super::super::app::tests::metric("run-1", "cpu");
        app.telemetry.push(event.clone());
        let text = rendered(&app, 80, 24);
        assert!(text.contains("LIVE METRIC DETAIL"));
        assert!(text.contains("Now: 0.250"));
        assert!(text.contains("Scalar sample: no time history"));
        event.value = None;
        event.unavailable_reason = Some("sensor offline".into());
        app.telemetry = TelemetryStore::default();
        app.telemetry.push(event);
        let text = rendered(&app, 80, 24);
        assert!(text.contains("sensor offline"));
        assert!(text.contains("No numeric samples"));
    }

    #[test]
    fn centers_welcome_panel_at_requested_percentages() {
        assert_eq!(
            centered(Rect::new(0, 0, 120, 40), 82, 72),
            Rect::new(11, 6, 98, 28)
        );
        assert_eq!(
            centered(Rect::new(0, 0, 80, 24), 82, 72),
            Rect::new(7, 3, 65, 17)
        );
    }

    #[test]
    fn centered_panel_respects_offset_and_bounds() {
        let area = Rect::new(10, 20, 120, 40);
        assert_eq!(centered(area, 50, 50), Rect::new(40, 30, 60, 20));
        assert_eq!(centered(area, 200, 200), area);
        assert_eq!(
            centered(Rect::new(10, 20, 0, 0), 82, 72),
            Rect::new(10, 20, 0, 0)
        );
        assert_eq!(
            centered(Rect::new(0, 0, 1000, 1000), 82, 72),
            Rect::new(90, 140, 820, 720)
        );
    }

    #[test]
    fn centered_panel_renders_title_and_content() {
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Paragraph::new("Local speech transcription test bench")
                        .block(Block::default().borders(Borders::ALL).title("PHEME VA")),
                    centered(frame.area(), 82, 72),
                );
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("PHEME VA"));
        assert!(text.contains("Local speech transcription test bench"));
    }
}
