use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Axis, Block, Borders, Cell, Chart, Dataset, Gauge, List, ListItem, Paragraph, Row, Table, Tabs,
    Wrap,
};
use ratatui::Frame;

use super::app::{App, Screen, TelemetryTab};
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
        Line::from("  2. Show which models can run in this binary"),
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
            let status = if entry.selectable() {
                "ready"
            } else {
                entry.status()
            };
            let color = if entry.selectable() {
                Color::Green
            } else {
                Color::Yellow
            };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{}  ", entry.manifest.id)),
                Span::styled(status, Style::default().fg(color)),
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
                if entry.adapter_compiled {
                    "compiled"
                } else {
                    "not compiled"
                }
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
                if !matches!(entry.manifest.family.as_str(), "whisper" | "zipformer") {
                    "Unsupported model family; add a matching adapter before selecting it."
                        .to_owned()
                } else if !entry.artifacts_available() {
                    "Download or place the required artifacts, then press [r].".to_owned()
                } else if !entry.adapter_compiled {
                    "[Enter] build adapter and restart automatically. Requires source checkout, Cargo and native toolchain; may download build dependencies/runtime packages. Settings retained; in-memory results/logs/history reset. [Esc] back.".to_owned()
                } else {
                    "[Enter] load this candidate".to_owned()
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
            "[↑/↓] select  [Enter] load/build  [r] rescan  [Esc] back",
        )),
        chunks[2],
    );
    draw_error_line(frame, app, outer);
}

fn draw_loading(frame: &mut Frame<'_>, app: &App) {
    if let Some(build) = &app.build {
        let chunks = base_layout(frame.area());
        draw_header(frame, app, chunks[0], "BUILDING ADAPTER");
        frame.render_widget(
            Paragraph::new(format!(
                "Building adapter for {}...\n\nCargo is running in the background. The first build may take several minutes.\n\n[t] view compiler progress in Logs.\n\nOn success the TUI restarts with the selected model. Settings are retained; in-memory results and history reset.\n\nOn failure or cancellation, the current model remains available.",
                build.model_id
            ))
            .block(Block::default().borders(Borders::ALL).title("ADAPTER BUILD"))
            .wrap(Wrap { trim: false }),
            chunks[1],
        );
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
    let titles = ["[1] Overview", "[2] Metrics", "[3] Graphs", "[4] Logs"]
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
        TelemetryTab::Graphs => draw_graphs(frame, app, chunks[2]),
        TelemetryTab::Logs => draw_logs(frame, app, chunks[2]),
    }
    let footer = if app.filter_editing {
        format!(
            "Live filter: {}_  [Enter/Esc] finish  [Backspace] delete",
            app.filter_query
        )
    } else {
        format!("[1-4] tab  [ and ] run  [j/k ↑/↓] {}  [f /] filter  [x] reset\n[Esc] back  [q] quit{}  Filter: {}",
            if matches!(app.telemetry_tab, TelemetryTab::Metrics | TelemetryTab::Graphs) { "series" } else { "scroll" },
            if app.telemetry_tab == TelemetryTab::Logs { "  [c] clear logs" } else { "" },
            if app.filter_query.is_empty() { "(none)" } else { &app.filter_query })
    };
    frame.render_widget(
        Paragraph::new(footer).style(Style::default().fg(Color::Yellow)),
        chunks[3],
    );
}

fn draw_overview(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let series = app.telemetry.series(&app.filter_query);
    let mut lines = vec![Line::from(format!(
        "{} matching series / {} retained events / {} logs (all runs)",
        series.len(),
        app.telemetry.events().count(),
        app.logs.len()
    ))];
    if let Some(model) = &app.active_model {
        let active = format!(
            "Active model (not selected-run metadata): {} / {} / {} / {}",
            model.id,
            model.family,
            model.runtime.as_deref().unwrap_or("unknown runtime"),
            model.backend
        );
        if active
            .to_ascii_lowercase()
            .contains(&app.filter_query.trim().to_ascii_lowercase())
        {
            lines.push(Line::from(active));
        }
    }
    let mut category = "";
    for series in &series {
        if category != series.category() {
            category = series.category();
            lines.push(Line::from(Span::styled(
                category,
                Style::default().fg(ACCENT),
            )));
        }
        let event = series.latest();
        lines.push(Line::from(format!(
            "{} [{:?} / {} / {}]",
            event.name,
            event.scope,
            event.source,
            TelemetryStore::unit_text(event)
        )));
        lines.push(Line::from(format!(
            "  {} ({} samples)",
            TelemetryStore::value_text(event),
            series.samples.len()
        )));
    }
    if series.is_empty() {
        lines.push(Line::from(
            "No matching metrics in the selected run. [x] reset filter.",
        ));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(format!(
                "OVERVIEW / run: {}",
                app.telemetry.selected_run().unwrap_or("none")
            )))
            .scroll((app.scroll, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_metrics(frame: &mut Frame<'_>, app: &App, area: Rect) {
    draw_series_browser(frame, app, area, false);
}

fn draw_graphs(frame: &mut Frame<'_>, app: &App, area: Rect) {
    draw_series_browser(frame, app, area, true);
}

fn draw_series_browser(frame: &mut Frame<'_>, app: &App, area: Rect, graph_focus: bool) {
    let series = app.telemetry.series(&app.filter_query);
    let selected = app.telemetry.series_index(&series);
    let outer = Block::default().borders(Borders::ALL).title(format!(
        "{} / run: {} / {} series",
        if graph_focus { "GRAPHS" } else { "METRICS" },
        app.telemetry.selected_run().unwrap_or("none"),
        series.len()
    ));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    if series.is_empty() {
        frame.render_widget(
            Paragraph::new("No matching metrics in the selected run. [x] reset filter."),
            inner,
        );
        return;
    }
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if graph_focus || inner.height < 20 {
                4
            } else {
                7
            }),
            Constraint::Min(6),
        ])
        .split(inner);
    let items = series
        .iter()
        .map(|series| {
            let event = series.latest();
            ListItem::new(vec![
                Line::from(format!("{} / {}", series.category(), event.name)),
                Line::from(format!(
                    "  {:?} / {} / {} = {}",
                    event.scope,
                    event.source,
                    TelemetryStore::unit_text(event),
                    TelemetryStore::value_text(event)
                )),
            ])
        })
        .collect::<Vec<_>>();
    let mut state = ratatui::widgets::ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(
        List::new(items)
            .highlight_symbol("> ")
            .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        panes[0],
        &mut state,
    );
    let selected = &series[selected];
    let event = selected.latest();
    let points = selected.history(app.telemetry.run_origin_ms());
    let details = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(3)])
        .split(panes[1]);
    let omitted = selected.samples.len() - points.len();
    let reason = selected
        .samples
        .iter()
        .rev()
        .find(|event| event.value.is_none() || event.unavailable_reason.is_some())
        .map(|event| TelemetryStore::value_text(event));
    frame.render_widget(
        Paragraph::new(format!(
            "{} [{:?} / {} / {}]\nLatest: {} | {} numeric, {} omitted{}",
            event.name,
            event.scope,
            event.source,
            TelemetryStore::unit_text(event),
            TelemetryStore::value_text(event),
            points.len(),
            omitted,
            reason
                .map(|reason| format!(" | {reason}"))
                .unwrap_or_default()
        ))
        .wrap(Wrap { trim: false }),
        details[0],
    );
    if points.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "No numeric history. Unavailable and non-numeric values are not plotted.",
            )
            .wrap(Wrap { trim: false }),
            details[1],
        );
        return;
    }
    let xmax = selected
        .samples
        .last()
        .map(|event| {
            event
                .timestamp_ms
                .saturating_sub(app.telemetry.run_origin_ms()) as f64
                / 1000.0
        })
        .unwrap_or(1.0)
        .max(0.001);
    let ymin = points
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let ymax = points
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let padding = if ymin == ymax {
        (ymin.abs() * 0.05).max(0.001)
    } else {
        0.0
    };
    let bounds = [ymin - padding, ymax + padding];
    // Scatter preserves timestamp gaps and never implies interpolation across unavailable samples.
    frame.render_widget(
        Chart::new(vec![Dataset::default()
            .data(&points)
            .marker(ratatui::symbols::Marker::Braille)
            .style(Style::default().fg(ACCENT))])
        .x_axis(
            Axis::default()
                .title("seconds since first retained run event")
                .bounds([0.0, xmax])
                .labels(["0".to_owned(), format!("{xmax:.3}")]),
        )
        .y_axis(
            Axis::default()
                .title(TelemetryStore::unit_text(event))
                .bounds(bounds)
                .labels([format!("{:.3}", bounds[0]), format!("{:.3}", bounds[1])]),
        ),
        details[1],
    );
}

fn draw_logs(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let logs = app.logs.filtered(&app.filter_query);
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
                Cell::from(entry.component.clone()),
                Cell::from(entry.message.clone()),
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
        Line::from("[1] overview  [2] metric series  [3] history graphs  [4] logs"),
        Line::from("[j/k or ↑/↓] select series (Metrics/Graphs), otherwise scroll"),
        Line::from("[ and ] previous/next run  [f or /] live shared filter  [x] reset"),
        Line::from("[Enter/Esc] finish filter  [c] clear logs (Logs only)  [Esc] back"),
        Line::from("Filter matches category/name/scope/source/unit/run/value/reason; history stays intact."),
        Line::from("Graphs show retained numeric samples only; gaps are not zero or interpolated."),
        Line::from(""),
        Line::from(Span::styled("Model availability", Style::default().fg(ACCENT))),
        Line::from("A manifest artifact is not active until its adapter is compiled and the engine loads successfully."),
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
    if event.value.is_none() {
        return format!(
            "unavailable ({})",
            event.unavailable_reason.as_deref().unwrap_or("no reason")
        );
    }
    format!(
        "{} {}",
        TelemetryStore::value_text(event),
        TelemetryStore::unit_text(event)
    )
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
            TelemetryTab::Overview,
            TelemetryTab::Metrics,
            TelemetryTab::Graphs,
            TelemetryTab::Logs,
        ] {
            app.telemetry_tab = tab;
            let text = rendered(&app, 100, 30);
            assert!(!text.contains("hide_memory"));
            assert!(!text.contains("hide_log"));
            assert!(!text.contains("wrong_run"));
            if tab == TelemetryTab::Logs {
                assert!(text.contains("keep_log"));
            } else {
                assert!(text.contains("selected-run"));
                assert!(text.contains("keep_cpu"));
            }
        }
    }

    #[test]
    fn history_and_unavailable_reason_render_at_minimum_size() {
        let mut app = super::super::app::tests::test_app();
        app.telemetry_tab = TelemetryTab::Graphs;
        let mut event = super::super::app::tests::metric("run-1", "cpu");
        app.telemetry.push(event.clone());
        let text = rendered(&app, 80, 24);
        assert!(text.contains("seconds since first retained run event"));
        assert!(text.contains("0.250"));
        assert!(text.contains("[x] reset"));
        event.value = None;
        event.unavailable_reason = Some("sensor offline".into());
        event.timestamp_ms += 1000;
        app.telemetry.push(event);
        let text = rendered(&app, 80, 24);
        assert!(text.contains("sensor offline"));
        assert!(text.contains("1 numeric, 1 omitted"));
        app.telemetry = TelemetryStore::default();
        let mut unavailable = super::super::app::tests::metric("run-2", "power");
        unavailable.value = None;
        unavailable.unavailable_reason = Some("no power sensor".into());
        app.telemetry.push(unavailable);
        let text = rendered(&app, 80, 24);
        assert!(text.contains("no power sensor"));
        assert!(text.contains("No numeric history"));
        app.filter_query = "no matches".into();
        assert!(rendered(&app, 80, 24).contains("No matching metrics"));
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
