use std::path::Path;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::model::ModelPurpose;

use super::app::{AdapterAvailability, App, Screen};
use super::connected::{TurnStatus, DEFAULT_SERVER_URL};
use super::download::script_download_id;
use super::editor::Editor;
use super::model_actions::{self, Confirmation, RoleSelection};
use super::model_catalog::{CatalogEntry, ModelCatalog};

pub fn header(frame: &mut Frame<'_>, app: &App, area: Rect, title: &str) {
    let mode = if let Some(target) = app
        .voice
        .target
        .as_deref()
        .filter(|_| matches!(app.screen, Screen::Web | Screen::WebEditor))
    {
        format!(
            "{} / SERVER {target}",
            if app.voice.online {
                "CONNECTED"
            } else {
                "DISCONNECTED"
            }
        )
    } else {
        "LOCAL CHAT".into()
    };
    let active =
        if matches!(app.screen, Screen::Web | Screen::WebEditor) && app.voice.connected_mode() {
            if let Some(snapshot) = app.voice.snapshot.as_ref() {
                let readiness = |ready| {
                    if !app.voice.online {
                        "stale"
                    } else if ready {
                        "ready"
                    } else {
                        "not ready"
                    }
                };
                let stt_ready = readiness(snapshot.stt.ready);
                let reply_ready = readiness(snapshot.reply.ready);
                let busy = if snapshot.busy { " | BUSY" } else { "" };
                let labels = format!("Active STT:  [{stt_ready}] | Reply:  [{reply_ready}]{busy}");
                let name_width = (area.width as usize).saturating_sub(labels.width()) / 2;
                format!(
                    "Active STT: {} [{stt_ready}] | Reply: {} [{reply_ready}]{busy}",
                    compact_name(&snapshot.stt.name, name_width),
                    compact_name(&snapshot.reply.name, name_width)
                )
            } else {
                "Active STT: unavailable | Reply: unavailable".into()
            }
        } else {
            let reply = app.local_runtime.as_ref().map(|r| r.voice.reply_status());
            format!(
                "Active STT: {} | Reply: {} [{}]",
                app.active_model
                    .as_ref()
                    .map(|m| m.id.as_str())
                    .unwrap_or("unavailable"),
                reply.as_ref().map(|r| r.name.as_str()).unwrap_or("loading"),
                if reply.as_ref().is_some_and(|r| r.ready) {
                    "ready"
                } else {
                    "not ready"
                }
            )
        };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!("PHEME VA / {title}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(active),
            Line::from(compact_name(&mode, usize::from(area.width))),
        ]),
        area,
    );
}

fn compact_name(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.into();
    }
    let mut output = String::new();
    for grapheme in text.graphemes(true) {
        if output.width() + grapheme.width() + 1 > width {
            break;
        }
        output.push_str(grapheme);
    }
    if width > 0 {
        output.push('…');
    }
    output
}

pub(super) fn workspace_guide(app: &App) -> String {
    let mut items = Vec::new();
    if app.screen != Screen::Web {
        items.push("[w] Web");
    }
    if !matches!(app.screen, Screen::Models | Screen::Loading) {
        items.push("[m] Models");
    }
    if app.screen != Screen::Telemetry {
        items.push("[t] Telemetry");
    }
    if !matches!(app.screen, Screen::Bench | Screen::ServerTests) {
        items.push("[b] Chat");
    }
    items.push("[q] Quit");
    if matches!(app.screen, Screen::Models | Screen::Loading) && app.build.is_some() {
        items.push("[Esc] Cancel build");
    } else if app.screen != Screen::Telemetry || (!app.run_detail && !app.metric_detail) {
        items.push("[Esc] Back");
    }
    items.join("  ")
}

// Consecutive command lines are one shortcut group; prose keeps its own rows.
fn footer_lines(text: &str, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width);
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut items = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.contains(']') {
            let starts = line
                .match_indices('[')
                .filter_map(|(index, _)| line[index..].contains(']').then_some(index))
                .collect::<Vec<_>>();
            for (position, start) in starts.iter().enumerate() {
                let end = starts.get(position + 1).copied().unwrap_or(line.len());
                items.push(line[*start..end].trim_end_matches([' ', '|']).trim());
            }
        } else {
            pack_shortcuts(&mut lines, &mut items, width);
            lines.extend(wrap_footer_line(line, width));
        }
    }
    pack_shortcuts(&mut lines, &mut items, width);
    lines
}

fn pack_shortcuts(lines: &mut Vec<Line<'static>>, items: &mut Vec<&str>, width: usize) {
    let mut row = Vec::new();
    let mut used = 0;
    for item in items.drain(..) {
        let cells = item.width();
        if !row.is_empty() && used + 2 + cells > width {
            lines.push(spread_shortcuts(&row, width));
            row.clear();
            used = 0;
        }
        if cells > width {
            lines.extend(wrap_footer_line(item, width));
        } else {
            used += cells + if row.is_empty() { 0 } else { 2 };
            row.push(item);
        }
    }
    if !row.is_empty() {
        lines.push(spread_shortcuts(&row, width));
    }
}

fn spread_shortcuts(items: &[&str], _width: usize) -> Line<'static> {
    Line::from(items.join("  "))
}

fn wrap_footer_line(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut row = String::new();
    for word in text.split_whitespace() {
        if !row.is_empty() && row.width() + 1 + word.width() > width {
            lines.push(Line::from(std::mem::take(&mut row)));
        }
        if !row.is_empty() {
            row.push(' ');
        }
        for grapheme in word.graphemes(true) {
            if row.width() + grapheme.width() > width && !row.is_empty() {
                lines.push(Line::from(std::mem::take(&mut row)));
            }
            row.push_str(grapheme);
        }
    }
    if !row.is_empty() || lines.is_empty() {
        lines.push(Line::from(row));
    }
    lines
}

pub(super) fn footer_height(text: &str, width: u16) -> u16 {
    footer_lines(text, width).len().min(usize::from(u16::MAX)) as u16
}

pub(super) fn draw_shortcut_footer(frame: &mut Frame<'_>, area: Rect, text: &str) {
    frame.render_widget(
        Paragraph::new(footer_lines(text, area.width)).style(Style::default().fg(Color::Yellow)),
        area,
    );
}

fn layout(frame: &mut Frame<'_>, app: &App, title: &str, footer: &str) -> Rect {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(1),
        Constraint::Length(footer_height(footer, frame.area().width).max(3)),
    ])
    .split(frame.area());
    header(frame, app, chunks[0], title);
    let status = app.error_message.as_deref().unwrap_or(&app.status_message);
    frame.render_widget(
        Paragraph::new(status)
            .style(Style::default().fg(if app.error_message.is_some() {
                Color::Red
            } else {
                Color::Gray
            }))
            .wrap(Wrap { trim: false }),
        chunks[2],
    );
    draw_shortcut_footer(frame, chunks[3], footer);
    chunks[1]
}

fn panel(frame: &mut Frame<'_>, area: Rect, title: &str, text: String, scroll: u16) {
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        area,
    );
}

pub fn web(frame: &mut Frame<'_>, app: &App) {
    let footer = if app.voice.discard_editor {
        "Discard the retained local web editor?\n[y] Confirm  [Esc] Keep".into()
    } else {
        format!(
            "{}\n[e] Edit pending turn  [x] Discard editor  [p] Replay  [z] Stop voice",
            workspace_guide(app)
        )
    };
    let area = layout(
        frame,
        app,
        "WEB INSPECTION (silent; web owns playback)",
        &footer,
    );
    if !app.voice.connected_mode() {
        panel(frame, area, "WEB / NO INSPECTION TARGET", format!(
            "[b] Chat returns to local voice/typed Chat.\n[m] Models manages local artifacts and startup choices.\n\nInspect Web explicitly through Go:\n  cli tui --server-url\n  cli tui --server-url {DEFAULT_SERVER_URL}\n\nChat and Tests always use the local Rust runtime.\nThis URL is only for Web inspection and approval.\nLocal and server models/conversations are separate."
        ),0);
        return;
    }
    let Some(snapshot) = app.voice.snapshot.as_ref() else {
        panel(frame, area, "WAITING FOR INSPECTION", app.voice.connection_error.clone().unwrap_or_else(|| "Polling Go /api/v1/voice/inspect in the background. No web work is created by this view.".into()), 0);
        return;
    };
    let areas = if area.width >= 110 {
        Layout::horizontal([Constraint::Percentage(48), Constraint::Percentage(52)]).split(area)
    } else {
        Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)]).split(area)
    };
    let history = snapshot
        .history
        .iter()
        .map(|message| format!("{}: {}", message.role, message.content))
        .collect::<Vec<_>>()
        .join("\n\n");
    panel(
        frame,
        areas[0],
        "WEB CONVERSATION (not saved locally)",
        if history.is_empty() {
            "No completed conversation history.".into()
        } else {
            history
        },
        app.scroll,
    );
    let current = if let Some(turn) = snapshot.current_turn.as_ref() {
        format!("ID: {} | State: {}\nTranscript: {}\nApproved: {}\n\nReply: {}\n\nTimings (server): {}{}{}",
            turn.turn_id, turn.status.label(), turn.transcript, turn.approved_text.as_deref().unwrap_or("awaiting explicit approval"),
            if turn.reply.is_empty() { "not started" } else { &turn.reply }, turn.timings,
            turn.error.as_ref().map(|error| format!("\nError {}: {}", error.code,error.message)).unwrap_or_default(),
            snapshot.role.as_ref().map(|role| format!("\nRole: {} / SHA256 {}", role.name, role.sha256)).unwrap_or_default())
    } else {
        "No current web turn. TUI tests do not appear here.".into()
    };
    let current = if !app.voice.online {
        format!(
            "STALE SNAPSHOT — {}\n\n{current}",
            app.voice
                .connection_error
                .as_deref()
                .unwrap_or("disconnected")
        )
    } else {
        current
    };
    panel(frame, areas[1], "CURRENT WEB TURN", current, app.scroll);
}

pub fn web_editor(frame: &mut Frame<'_>, app: &App) {
    let area = layout(frame, app, "EDIT WEB TURN", "[Enter] Explicitly submit this WEB turn  [Esc] Close (retain text)  [arrows/Home/End] Move  [Alt+Enter] Newline");
    let Some(editor) = app.voice.editor.as_ref() else {
        return;
    };
    let chunks = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(4),
        Constraint::Length(3),
    ])
    .split(area);
    let current = app
        .voice
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.current_turn.as_ref());
    let stale = !current.is_some_and(|turn| {
        turn.turn_id == editor.turn_id && matches!(turn.status, TurnStatus::AwaitingReview)
    });
    panel(frame, chunks[0], &format!("WEB TURN {}", editor.turn_id), format!(
        "Original: {}\n{}\nThis approves the WEB request. The answer returns to the WEB, not a TUI test.", editor.original,
        if editor.accepted { "Approved by server — read only" } else if editor.pending { "Submission pending — no automatic retry" } else if stale { "STALE TURN — submission disabled; local text retained" } else if editor.buffer.dirty { "Unsaved local changes" } else { "Review before explicit Enter submission" }), 0);
    draw_editor(
        frame,
        chunks[1],
        "APPROVED QUESTION (max 8192 bytes)",
        &editor.buffer,
        !editor.accepted && !editor.pending,
    );
    panel(
        frame,
        chunks[2],
        "SUBMISSION",
        editor.error.clone().unwrap_or_else(|| {
            if editor.accepted {
                "Accepted; authoritative reply is visible in Web inspection. No local autoplay."
                    .into()
            } else {
                "Reply readiness gates Submit, not editing. Closing retains this turn-bound buffer."
                    .into()
            }
        }),
        0,
    );
}

pub fn tests(frame: &mut Frame<'_>, app: &App) {
    let tests = &app.voice.tests;
    let footer = if tests.editing {
        "[Enter] Run isolated reply test  [Esc] Finish editing  [arrows] Move  [Alt+Enter] Newline"
            .into()
    } else if tests.diagnostics {
        format!(
            "{}\n[d] Back to tests  [↑/↓ PgUp/PgDn] Scroll  [x] Cancel test  [z] Stop voice",
            workspace_guide(app)
        )
    } else {
        format!("{}\n[i/e] Edit  [l] Mic  [f] WAV  [Enter] Reply  [r] Retry  [x] Cancel test\n[d] Diagnostics  [p] Replay  [v] Voice  [z] Stop voice", workspace_guide(app))
    };
    let area = layout(frame, app, "LOCAL TESTS — ISOLATED FROM CHAT/WEB", &footer);
    if tests.diagnostics {
        let stt = tests.transcript.as_ref().map_or("No completed local STT result.".into(), |transcript| format!(
            "Model: {} / Backend: {}\nGate status: {}\nServer STT processing: {} ms\n\nRaw transcript:\n{}\n\nCleaned transcript:\n{}",
            transcript.model_id, transcript.stt_backend, transcript.status, transcript.processing_time_ms, transcript.raw_text, transcript.text));
        panel(frame, area, "ISOLATED TEST DIAGNOSTICS", format!(
            "Source: {}\nStage: {}\nClient last-operation total: {} (not resource/energy data)\nError: {}\n\n{}",
            tests.source, tests.stage, tests.elapsed_ms.map_or("unavailable".into(), |ms| format!("{ms} ms")), tests.error.as_deref().unwrap_or("none"), stt), app.scroll);
        return;
    }
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Percentage(40),
        Constraint::Min(3),
        Constraint::Length(4),
    ])
    .split(area);
    frame.render_widget(Paragraph::new(format!("Source: {} | Stage: {} | Voice: {} | Request: {}\nExplicit STT review before reply. No web history or submission.",
        tests.source,tests.stage,if tests.auto_voice { "local espeak on" } else { "off" },tests.request.map_or("none".into(),|id| id.to_string()))),chunks[0]);
    draw_editor(
        frame,
        chunks[1],
        "TEST QUESTION / TRANSCRIPT REVIEW",
        &tests.buffer,
        tests.editing,
    );
    panel(
        frame,
        chunks[2],
        "TEST REPLY (only here)",
        tests.reply.clone(),
        app.scroll,
    );
    let diagnostic = if let Some(error) = tests.error.as_ref() {
        format!("Error: {error}")
    } else if let Some(transcript) = tests.transcript.as_ref() {
        format!(
            "STT {} / {} / {} / {} ms\nRaw: {}\nClient operation total: {}",
            transcript.model_id,
            transcript.stt_backend,
            transcript.status,
            transcript.processing_time_ms,
            transcript.raw_text,
            tests.elapsed_ms.map_or("unavailable".into(), |ms| format!(
                "{ms} ms (not CPU/power)"
            ))
        )
    } else {
        format!(
            "Client operation total: {}. Server resource/energy measurements unavailable.",
            tests
                .elapsed_ms
                .map_or("unavailable".into(), |ms| format!("{ms} ms"))
        )
    };
    panel(frame, chunks[3], "DIAGNOSTICS / TIMINGS", diagnostic, 0);
}

fn draw_editor(frame: &mut Frame<'_>, area: Rect, title: &str, editor: &Editor, focused: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if focused { Color::Cyan } else { Color::Gray }));
    let inner = block.inner(area);
    let (lines, (col, row)) = editor.wrapped(inner.width as usize);
    let scroll = if focused {
        row.saturating_sub(inner.height.saturating_sub(1) as usize) as u16
    } else {
        0
    };
    frame.render_widget(
        Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
            .block(block)
            .scroll((scroll, 0)),
        area,
    );
    if focused && inner.width > 0 && inner.height > 0 {
        frame.set_cursor_position((
            inner.x + col.min(inner.width.saturating_sub(1) as usize) as u16,
            inner.y + (row - scroll as usize) as u16,
        ));
    }
}

pub fn models(frame: &mut Frame<'_>, app: &App) {
    let footer = model_footer(app);
    let area = layout(frame, app, "MODELS / LOCAL FILES", &footer);
    super::ui::draw_picker(frame, app, area);
}

fn model_footer(app: &App) -> String {
    if let Some(roles) = app.models.role_selection.as_ref() {
        return if roles.adding_path {
            "[Enter] Add local .txt path  [Esc] Cancel path  [Backspace] Delete\nPath input: shortcuts type text, not navigation\nApplied next host start only; active hosts unchanged".into()
        } else {
            "[↑/↓] File  [Space] Toggle  [a] Add path  [Enter] Save  [Esc] Cancel\nSelection order is the combined prompt order\nApplied next host start only; active hosts unchanged".into()
        };
    }
    if app.models.confirmation.is_some() {
        return "[y] Confirm local action  [Esc] Cancel  [PgUp/PgDn] Scroll\nNetwork/download or verified next-start choice only\nActive hosts unchanged; no model-management HTTP request".into();
    }
    if app.models.preview.is_some() || app.models.command {
        return "[PgUp/PgDn] Scroll  [Esc] Close\nReply files/roles apply next TUI/server startup\nActive hosts unchanged; no model-management HTTP request".into();
    }
    if app.models.filtering {
        return "[Enter/Esc] Finish  [Backspace] Delete\nSearch: type filter across all models\nNavigation shortcuts disabled while editing".into();
    }
    let reply = app
        .models
        .selected(&app.catalog)
        .is_some_and(|entry| entry.manifest.purpose() == Some(ModelPurpose::Reply));
    let enter = if reply { "Download" } else { "Download/load" };
    let extra = if app.download.is_some() {
        "  [x] Cancel"
    } else {
        ""
    };
    format!("{}\n[↑/↓] Select  [/] Search  [Enter] {enter}  [s] Startup  [r] Rescan\n[PgUp/PgDn] Details  [p] Preview  [g] Command{}{extra}",
        workspace_guide(app), if reply { "  [o] Roles" } else { "" })
}

pub(super) fn model_details(
    frame: &mut Frame<'_>,
    app: &App,
    entry: Option<&CatalogEntry>,
    area: Rect,
) {
    if let Some(roles) = app.models.role_selection.as_ref() {
        draw_roles(frame, app, roles, area);
        return;
    }
    if let Some(prompt) = app.models.preview.as_ref() {
        panel(
            frame,
            area,
            "COMBINED ROLE PREVIEW",
            format!(
                "Applied next host start only.\nSHA256: {}\n\n{}",
                prompt.sha256, prompt.text
            ),
            app.models.scroll,
        );
        return;
    }
    if app.models.command {
        panel(frame, area, "NEXT-START COMMAND", format!(
            "Active server unchanged. No hot-swap.\nNext STT: {}\nNext reply: {}\n\nRun from pheme-va/:\n{}\n\nSaved reply and roles also apply on the next TUI start; quit and relaunch for local Chat.\nSTT here is the next server choice; Enter on Models selects local STT.\nRestart loses in-memory web history. Finish or deliberately cancel web work before restarting.\nThis command does not establish server readiness.",
            app.config.server_stt_model.as_deref().unwrap_or("not chosen"),
            app.config.server_reply_model.as_deref().unwrap_or("not chosen"),
            model_actions::startup_command(&app.config)), app.models.scroll);
        return;
    }
    if let Some(confirmation) = app.models.confirmation.as_ref() {
        let (title, text) = match confirmation {
            Confirmation::Download(id) => {
                ("CONFIRM LOCAL DOWNLOAD", download_confirmation(app, id))
            }
            Confirmation::Choose(id) => {
                let guidance = if entry
                    .is_some_and(|e| e.manifest.purpose() == Some(ModelPurpose::Reply))
                {
                    "Reply and roles apply after TUI/server restart. Finish local Chat, quit and relaunch."
                } else {
                    "This STT choice applies on server restart. Enter on Models loads/prepares STT locally."
                };
                ("VERIFIED NEXT-START CHOICE", format!(
                    "Model: {id}\nArtifact bundle and selected roles checked locally.\n\n{guidance}\nActive models unchanged. Archives stay read-only after restart.\n\n[y] Save locally and show command\n[Esc] Cancel"))
            }
        };
        panel(frame, area, title, text, app.models.scroll);
        return;
    }
    let lines = if let Some(entry) = entry {
        model_info(app, entry)
    } else {
        vec![Line::from(app.catalog.error.as_ref().map_or_else(
            || "No matching model selected. Clear or change the filter.".into(),
            |error| format!("Manifest error: {error}"),
        ))]
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("MODEL DETAILS / PgUp/PgDn"),
            )
            .wrap(Wrap { trim: false })
            .scroll((
                if entry.is_some() {
                    app.models.scroll
                } else {
                    0
                },
                0,
            )),
        area,
    );
}

fn model_field(label: &str, value: impl std::fmt::Display) -> Line<'static> {
    Line::from(format!("{label:<16} {value}"))
}

fn model_info(app: &App, entry: &CatalogEntry) -> Vec<Line<'static>> {
    let manifest = &entry.manifest;
    let reply = manifest.purpose() == Some(ModelPurpose::Reply);
    let verified = app
        .models
        .verified_choice
        .as_ref()
        .is_some_and(|choice| choice.id == manifest.id);
    let artifacts = if !entry.artifacts_available() {
        "missing"
    } else if verified {
        "verified for startup choice"
    } else {
        "present (not verified)"
    };
    let chosen = match manifest.purpose() {
        Some(ModelPurpose::Transcript) => app.config.server_stt_model.as_ref(),
        Some(ModelPurpose::Reply) => app.config.server_reply_model.as_ref(),
        None => None,
    } == Some(&manifest.id);
    let server = model_actions::startup_supported(manifest).map_or_else(
        |error| format!("unsupported: {error}"),
        |_| "supported family; inspect readiness".into(),
    );
    let mut lines = vec![
        Line::from(Span::styled(
            manifest.id.clone(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        model_field(
            "Purpose:",
            super::ui::model_purpose_label(manifest.purpose()),
        ),
        model_field("Artifacts:", artifacts),
        model_field(
            "Missing:",
            if entry.missing_paths.is_empty() {
                "none".into()
            } else {
                ModelCatalog::missing_summary(entry)
            },
        ),
    ];
    if reply {
        lines.extend([
            Line::from("Local reply runtime; applies on next TUI startup."),
            Line::from("Files do not establish runtime readiness."),
        ]);
    }
    lines.extend([
        model_field("Standalone:", app.adapter_availability(entry).label()),
        model_field("Adapter support:", server),
        model_field(
            "Next start:",
            if chosen {
                "chosen (saved locally)"
            } else {
                "not chosen"
            },
        ),
    ]);
    if reply {
        let roles =
            model_actions::role_paths_for(manifest, &app.catalog.manifest_path, &app.config)
                .map_or_else(
                    |error| format!("unavailable: {error}"),
                    |paths| {
                        paths
                            .iter()
                            .enumerate()
                            .map(|(index, path)| format!("{}. {}", index + 1, filename(path)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    },
                );
        lines.extend([
            model_field(
                "Role choice:",
                if app.config.reply_role_files.contains_key(&manifest.id) {
                    "saved per model"
                } else {
                    "manifest default"
                },
            ),
            model_field(
                "Selected roles:",
                if roles.is_empty() {
                    "none".into()
                } else {
                    roles
                },
            ),
        ]);
    }
    lines.extend([
        model_field("Family:", &manifest.family),
        model_field("Runtime:", manifest.runtime.as_deref().unwrap_or("not specified")),
        model_field("Revision:", manifest.revision.as_deref().unwrap_or("not specified")),
        model_field("Size:", manifest.model_size.as_deref().unwrap_or("unavailable")),
        model_field("Bytes:", manifest.model_size_bytes.map_or("unavailable".into(), |bytes| bytes.to_string())),
        model_field("Quantization:", manifest.quantization.as_deref().unwrap_or("not recorded")),
        model_field("Languages:", if manifest.languages.is_empty() { "not recorded".into() } else { manifest.languages.join(", ") }),
        model_field("License:", manifest.license.as_deref().unwrap_or("not recorded")),
        model_field("Source:", manifest.repository.as_deref().unwrap_or("unavailable")),
        model_field("SHA256:", manifest.sha256.as_deref().unwrap_or("unavailable")),
        model_field("Model path:", entry.model_path.display()),
        model_field("Download:", script_download_id(manifest).unwrap_or_else(|error| format!("unavailable: {error}"))),
        model_field("Next STT:", app.config.server_stt_model.as_deref().unwrap_or("not chosen")),
        model_field("Next reply:", app.config.server_reply_model.as_deref().unwrap_or("not chosen")),
        model_field("Timestamps:", manifest.timestamps),
        model_field("Streaming:", manifest.streaming),
        Line::from(""),
        Line::from("Highlighting does not activate a model. [s] verifies artifacts and roles before confirming a startup choice."),
    ]);
    if !entry.artifacts_available() {
        lines.push(Line::from("[Enter] requests a confirmed local download; pinned checksums are verified by the script."));
    } else if !reply {
        lines.push(Line::from(match app.adapter_availability(entry) {
            AdapterAvailability::Compiled => "[Enter] loads this standalone STT candidate.",
            AdapterAvailability::Cached => "[Enter] uses the validated cached STT adapter; a launcher restart may occur.",
            AdapterAvailability::Prepare | AdapterAvailability::Checking => "[Enter] prepares the standalone adapter. Requires source, Cargo and native toolchain; build dependencies may need network access. Settings/reports survive restart; live telemetry resets.",
            _ => "No supported standalone STT adapter for this entry.",
        }));
    } else {
        lines.push(Line::from("Active inference is unchanged by local file management. Use [s] for a verified next-start choice."));
    }
    lines
}

fn download_confirmation(app: &App, id: &str) -> String {
    let Some(entry) = app.catalog.entry_by_id(id) else {
        return "Catalog entry no longer available".into();
    };
    let root = app
        .catalog
        .manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let mut lines = vec![
        Line::from("Network access to the pinned source. Script verifies checksums and resumes valid partial files."),
        Line::from("Local only: no remote installation or active model change."),
        Line::from(""),
        model_field("Model:", id),
        model_field("Target:", script_download_id(&entry.manifest).unwrap_or_else(|error| error.to_string())),
        model_field("Root:", root.display()),
        model_field("Destination:", entry.model_path.display()),
        model_field("Size:", entry.manifest.model_size.as_deref().unwrap_or("unavailable")),
        model_field("Expected bytes:", entry.manifest.model_size_bytes.map_or("unavailable".into(), |bytes| bytes.to_string())),
        model_field("License:", entry.manifest.license.as_deref().unwrap_or("not recorded — review source license")),
        model_field("Source:", entry.manifest.repository.as_deref().unwrap_or("unavailable")),
        model_field("Revision:", entry.manifest.revision.as_deref().unwrap_or("unavailable")),
        model_field("SHA256:", entry.manifest.sha256.as_deref().unwrap_or("unavailable")),
        model_field("Available disk:", available_disk(root).map_or("unavailable".into(), |bytes| format!("{bytes} bytes"))),
    ];
    lines.push(Line::from("\n[y] Confirm network/download\n[Esc] Cancel"));
    lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

fn filename(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn draw_roles(frame: &mut Frame<'_>, app: &App, roles: &RoleSelection, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("ROLE FILES / NEXT START ONLY");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let order_height = (roles.selected_files.len() + 3).clamp(4, 6) as u16;
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(order_height),
        Constraint::Length(if roles.error.is_some() { 2 } else { 0 }),
    ])
    .split(inner);
    let default = app
        .catalog
        .entry_by_id(&roles.model_id)
        .and_then(|entry| entry.manifest.system_prompt.as_deref())
        .map_or("none recorded".into(), filename);
    frame.render_widget(
        Paragraph::new(format!(
            "Model: {}\nDirectory: {}\nDefault: {default}",
            roles.model_id,
            roles.directory.display()
        )),
        sections[0],
    );
    if roles.adding_path {
        let editor = Editor {
            text: roles.path_input.clone(),
            cursor: roles.path_input.len(),
            dirty: false,
        };
        draw_editor(
            frame,
            sections[1],
            "ADD EXISTING LOCAL .txt PATH",
            &editor,
            true,
        );
    } else {
        let width = usize::from(sections[1].width.saturating_sub(4));
        let items = roles
            .entries
            .iter()
            .map(|path| {
                let order = roles
                    .selected_files
                    .iter()
                    .position(|selected| selected == path);
                let label = format!(
                    "{} {}",
                    order.map_or("[ ]".into(), |index| format!("[✓ {}]", index + 1)),
                    filename(path)
                );
                ListItem::new(
                    Editor::new(label)
                        .wrapped(width)
                        .0
                        .into_iter()
                        .map(Line::from)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        state.select((roles.cursor < roles.entries.len()).then_some(roles.cursor));
        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("LOCAL .txt FILES"),
                )
                .highlight_symbol("> ")
                .highlight_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            sections[1],
            &mut state,
        );
        if roles.entries.is_empty() {
            let inner = Block::default().borders(Borders::ALL).inner(sections[1]);
            frame.render_widget(
                Paragraph::new("No local .txt files. [a] Add an existing path.")
                    .wrap(Wrap { trim: false }),
                inner,
            );
        }
    }
    let selected = if roles.selected_files.is_empty() {
        "No files selected.".into()
    } else {
        roles
            .selected_files
            .iter()
            .enumerate()
            .map(|(index, path)| format!("{}. {}", index + 1, filename(path)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    panel(frame, sections[2], "SELECTION ORDER", selected, 0);
    if let Some(error) = roles.error.as_ref() {
        frame.render_widget(
            Paragraph::new(error.as_str())
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: false }),
            sections[3],
        );
    }
}

fn available_disk(path: &std::path::Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: path is NUL terminated and stat points to writable storage.
        if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } == 0 {
            // SAFETY: statvfs initialized the structure on success.
            let stat = unsafe { stat.assume_init() };
            return Some(stat.f_bavail.saturating_mul(stat.f_frsize));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn buffer(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, app))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn text(buffer: &ratatui::buffer::Buffer, area: Rect) -> String {
        (area.y..area.bottom())
            .map(|y| {
                (area.x..area.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn columns(width: u16, height: u16) -> (Rect, Rect) {
        let columns = Layout::horizontal([Constraint::Percentage(44), Constraint::Percentage(56)])
            .split(Rect::new(0, 3, width, height - 7));
        (columns[0], columns[1])
    }

    fn model_app() -> App {
        let mut app = super::super::app::tests::test_app();
        app.screen = Screen::Models;
        app.catalog.manifest_path = "models/manifest.toml".into();
        let manifest: crate::model::ModelEntry = toml::from_str(
            r#"
            id = "reply-fixture"
            family = "qwen"
            purpose = "reply"
            runtime = "llama.cpp"
            model = "reply/fixture.gguf"
            system_prompt = "roles/incident-reporting.txt"
            revision = "fixture-revision"
            model_size = "1.5B"
            model_size_bytes = 123456
            quantization = "Q4_K_M"
            languages = ["English", "Chinese"]
            license = "Apache-2.0"
            repository = "https://example.invalid/model"
            sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        "#,
        )
        .unwrap();
        app.catalog.entries.push(CatalogEntry {
            manifest,
            model_path: "models/reply/fixture.gguf".into(),
            missing_paths: vec![],
            adapter_compiled: false,
        });
        app.models.group_mut().highlighted = Some("reply-fixture".into());
        app
    }

    fn roles() -> RoleSelection {
        let directory = std::path::PathBuf::from("models/roles");
        let entries = ["incident-reporting.txt", "safety.txt", "summary.txt"]
            .into_iter()
            .map(|name| directory.join(name))
            .collect::<Vec<_>>();
        RoleSelection {
            model_id: "reply-fixture".into(),
            directory,
            selected_files: vec![entries[0].clone()],
            entries,
            cursor: 0,
            adding_path: false,
            path_input: String::new(),
            error: None,
        }
    }

    #[test]
    fn standalone_web_dispatch_uses_alphabet_footer_not_header_navigation() {
        let mut app = super::super::app::tests::test_app();
        app.screen = Screen::Web;
        for (width, height) in [(80, 24), (120, 32)] {
            let buffer = buffer(&app, width, height);
            let rendered = text(&buffer, buffer.area);
            let header = text(&buffer, Rect::new(0, 0, width, 3));
            let footer = text(&buffer, Rect::new(0, height - 3, width, 3));
            assert!(!footer.contains("[w]"), "{footer}");
            for label in ["[m] Models", "[t] Telemetry", "[b] Chat", "[Esc] Back"] {
                assert!(footer.contains(label), "{footer}");
                assert!(!header.contains(label), "{header}");
            }
            for label in ["1 Web", "2 Tests", "3 Models", "4 Telemetry", "MAIN"] {
                assert!(!rendered.contains(label), "{rendered}");
            }
            assert!(rendered.contains("WEB / NO INSPECTION TARGET"));
            assert!(rendered.contains("[b] Chat returns to local voice/typed Chat"));
        }
    }

    #[test]
    fn connected_views_keep_header_and_alphabet_guidance_at_both_sizes() {
        let mut app = model_app();
        app.voice.target = Some(DEFAULT_SERVER_URL.into());
        app.voice.inspect(Ok(super::super::voice::tests::snapshot(
            "web",
            "partial reply",
        )));
        for (width, height) in [(80, 24), (120, 32)] {
            for screen in [
                Screen::Web,
                Screen::ServerTests,
                Screen::Models,
                Screen::Telemetry,
            ] {
                app.screen = screen;
                let buffer = buffer(&app, width, height);
                let rendered = text(&buffer, buffer.area);
                let header = text(&buffer, Rect::new(0, 0, width, 3));
                let footer = text(&buffer, Rect::new(0, height - 3, width, 3));
                for (page, label, key) in [
                    (Screen::Web, "[w] Web", "[w]"),
                    (Screen::Models, "[m] Models", "[m]"),
                    (Screen::Telemetry, "[t] Telemetry", "[t]"),
                    (Screen::ServerTests, "[b] Chat", "[b]"),
                ] {
                    if screen == page {
                        assert!(!footer.contains(key), "{footer}");
                    } else {
                        assert!(footer.contains(label), "{footer}");
                    }
                    assert!(!header.contains(label), "{header}");
                }
                assert!(footer.contains("[Esc] Back"), "{footer}");
                for label in ["1 Web", "2 Tests", "3 Models", "4 Telemetry"] {
                    assert!(!rendered.contains(label), "{rendered}");
                }
                if screen == Screen::Web {
                    assert!(header.contains("STT: stt [ready]"), "{header}");
                    assert!(header.contains("Reply: reply [ready]"), "{header}");
                } else {
                    assert!(header.contains("LOCAL CHAT"), "{header}");
                }
                assert!(header.contains("PHEME VA /"), "{header}");
                if screen == Screen::Models {
                    assert!(rendered.contains("[b] Chat"), "{rendered}");
                    assert!(rendered.contains("[Enter] Download"), "{rendered}");
                }
                if screen == Screen::ServerTests {
                    for key in ["[Enter] Reply", "[x] Cancel test", "[z] Stop voice"] {
                        assert!(rendered.contains(key), "{rendered}");
                    }
                }
            }
        }
        app.screen = Screen::Models;
        let snapshot = app.voice.snapshot.as_mut().unwrap();
        snapshot.stt.name = "a-very-long-transcription-model-name".into();
        snapshot.reply.name = "a-very-long-reply-model-name".into();
        let buffer = buffer(&app, 80, 24);
        let active = text(&buffer, Rect::new(0, 1, 80, 1));
        assert!(
            !active.contains("a-very-long"),
            "remote state must not label local Models: {active}"
        );
        assert!(text(&buffer, Rect::new(0, 0, 80, 1)).contains("MODELS / LOCAL FILES"));
    }

    #[test]
    fn shortcuts_merge_command_lines_with_fixed_left_aligned_spacing() {
        let lines = footer_lines("[界] 开启  [e\u{301}] Café\n[🙂] Replay  [Esc] Back", 60);
        assert_eq!(lines.len(), 1);
        let row = lines[0].to_string();
        assert_eq!(row, "[界] 开启  [e\u{301}] Café  [🙂] Replay  [Esc] Back");
        assert!(row.starts_with("[界] 开启"), "{row}");
        assert!(row.ends_with("[Esc] Back"), "{row}");
        assert!(row.contains("[e\u{301}] Café"), "{row}");
        assert!(row.contains("[🙂] Replay"), "{row}");
        assert_eq!(footer_height(&row, 60), 1);

        let narrow = footer_lines("[界] 开启  [e\u{301}] Café\n[🙂] Replay  [Esc] Back", 24);
        assert_eq!(narrow.len(), 2);
        for line in narrow {
            assert!(line.width() <= 24, "{line}");
            assert!(!line.to_string().contains("   "));
        }
    }

    #[test]
    fn unicode_footer_renders_in_cells_and_keeps_shortcuts_flush_left() {
        let footer = "[界] 开启  [e\u{301}] Café\n[🙂] Replay  [Esc] Back";
        for width in [24, 60] {
            let height = footer_height(footer, width);
            let mut terminal = Terminal::new(TestBackend::new(width, height + 2)).unwrap();
            terminal
                .draw(|frame| {
                    frame.render_widget(Paragraph::new("BODY / STATUS"), Rect::new(0, 0, width, 1));
                    draw_shortcut_footer(frame, Rect::new(0, 2, width, height), footer);
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            assert!(text(buffer, Rect::new(0, 0, width, 1)).contains("BODY / STATUS"));
            assert!(text(buffer, Rect::new(0, 1, width, 1)).trim().is_empty());
            let last = text(buffer, Rect::new(0, height + 1, width, 1));
            assert!(last.trim_end().ends_with("[Esc] Back"), "{last}");
            let expected = footer_lines(footer, width).last().unwrap().to_string();
            let mut col = 0;
            for grapheme in expected.graphemes(true) {
                assert_eq!(buffer[(col, height + 1)].symbol(), grapheme);
                col += UnicodeWidthStr::width(grapheme) as u16;
            }
            for x in col..width {
                assert_eq!(buffer[(x, height + 1)].symbol(), " ");
            }
            assert_eq!(buffer[(1, 2)].symbol(), "界");
            assert!(buffer
                .content
                .iter()
                .any(|cell| cell.symbol() == "e\u{301}"));
            assert!(buffer.content.iter().any(|cell| cell.symbol() == "🙂"));
        }
    }

    #[test]
    fn footer_prose_stays_separate_and_long_items_wrap_without_losing_graphemes() {
        let footer =
            "[y] Confirm  [Esc] Cancel\nRunning server unchanged\n[Enter] Save  [Space] Toggle";
        let lines = footer_lines(footer, 80);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].to_string(), "Running server unchanged");
        assert!(lines[0].to_string().ends_with("[Esc] Cancel"));
        assert!(lines[2].to_string().ends_with("[Space] Toggle"));

        let long = "[Esc] Close this 界界界界界界界界 e\u{301}ditor without losing text";
        let lines = footer_lines(long, 16);
        assert!(lines.len() > 1);
        assert!(lines[0].to_string().starts_with("[Esc]"));
        assert!(lines.iter().all(|line| line.width() <= 16));
        let compact = |text: &str| text.split_whitespace().collect::<String>();
        let wrapped = lines.iter().map(ToString::to_string).collect::<String>();
        assert_eq!(compact(&wrapped), compact(long));
        assert!(footer_lines(long, 0).is_empty());
    }

    #[test]
    fn server_tests_keep_every_hint_at_minimum_size_including_diagnostics() {
        let mut app = model_app();
        app.screen = Screen::ServerTests;
        app.voice.target = Some(DEFAULT_SERVER_URL.into());
        for diagnostics in [false, true] {
            app.voice.tests.diagnostics = diagnostics;
            let buffer = buffer(&app, 80, 24);
            let footer = text(&buffer, Rect::new(0, 21, 80, 3));
            let keys: &[&str] = if diagnostics {
                &[
                    "[d] Back to tests",
                    "[↑/↓ PgUp/PgDn] Scroll",
                    "[x] Cancel test",
                    "[z] Stop voice",
                ]
            } else {
                &[
                    "[i/e] Edit",
                    "[l] Mic",
                    "[f] WAV",
                    "[Enter] Reply",
                    "[r] Retry",
                    "[x] Cancel test",
                    "[d] Diagnostics",
                    "[p] Replay",
                    "[v] Voice",
                    "[z] Stop voice",
                ]
            };
            for hint in keys.iter().copied().chain([
                "[w] Web",
                "[m] Models",
                "[t] Telemetry",
                "[q] Quit",
                "[Esc] Back",
            ]) {
                assert!(footer.contains(hint), "missing {hint}: {footer}");
            }
            assert!(!footer.contains("[b]"), "{footer}");
            assert!(text(&buffer, Rect::new(0, 3, 80, 17)).contains("DIAGNOSTICS"));
        }
    }

    #[test]
    fn nested_web_and_test_editors_keep_focus_and_only_contextual_guidance() {
        let mut app = model_app();
        app.voice.target = Some(DEFAULT_SERVER_URL.into());
        app.voice.inspect(Ok(super::super::voice::tests::snapshot(
            "web",
            "partial reply",
        )));
        app.voice.open_editor().unwrap();
        for screen in [Screen::WebEditor, Screen::ServerTests] {
            app.screen = screen;
            app.voice.tests.editing = screen == Screen::ServerTests;
            app.voice.tests.buffer = Editor::new("typed 界 e\u{301}ditor".into());
            for (width, height) in [(80, 24), (120, 32)] {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| super::super::ui::draw(frame, &app))
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let footer = text(buffer, Rect::new(0, height - 3, width, 3));
                for key in ["[w]", "[m]", "[b]", "[t]", "[q]", "[Esc] Back"] {
                    assert!(!footer.contains(key), "{footer}");
                }
                assert!(footer.contains("[Enter]"), "{footer}");
                assert!(footer.contains("[Alt+Enter] Newline"), "{footer}");
                assert!(
                    footer.contains(if screen == Screen::WebEditor {
                        "[Esc] Close (retain text)"
                    } else {
                        "[Esc] Finish editing"
                    }),
                    "{footer}"
                );
                let cursor = terminal.get_cursor_position().unwrap();
                assert!(cursor.y >= 3 && cursor.y < height - 4, "{cursor:?}");
                assert!(cursor.x > 0 && cursor.x < width - 1, "{cursor:?}");
            }
        }
        app.screen = Screen::Web;
        app.voice.discard_editor = true;
        let buffer = buffer(&app, 80, 24);
        let footer = text(&buffer, Rect::new(0, 21, 80, 3));
        assert!(
            footer.contains("Discard the retained local web editor?"),
            "{footer}"
        );
        assert!(footer.contains("[y] Confirm"), "{footer}");
        assert!(footer.contains("[Esc] Keep"), "{footer}");
        assert!(!footer.contains("[Esc] Back"), "{footer}");
    }

    #[test]
    fn models_keep_all_actions_without_self_navigation_and_preserve_modal_hints() {
        let mut app = model_app();
        for (width, height) in [(80, 24), (120, 32)] {
            let rendered = buffer(&app, width, height);
            let footer = text(&rendered, Rect::new(0, height - 3, width, 3));
            for hint in [
                "[w] Web",
                "[t] Telemetry",
                "[b] Chat",
                "[q] Quit",
                "[Esc] Back",
                "[↑/↓] Select",
                "[/] Search",
                "[Enter] Download",
                "[s] Startup",
                "[r] Rescan",
                "[PgUp/PgDn] Details",
                "[p] Preview",
                "[g] Command",
                "[o] Roles",
            ] {
                assert!(footer.contains(hint), "missing {hint}: {footer}");
            }
            assert!(!footer.contains("[m]"), "{footer}");
        }
        for (width, height) in [(80, 24), (120, 32)] {
            app.models.filtering = true;
            let rendered = buffer(&app, width, height);
            let footer = text(&rendered, Rect::new(0, height - 3, width, 3));
            assert!(footer.contains("[Enter/Esc] Finish"), "{footer}");
            assert!(footer.contains("[Backspace] Delete"), "{footer}");
            assert!(
                footer.contains("Navigation shortcuts disabled while editing"),
                "{footer}"
            );
            for key in ["[w]", "[m]", "[b]", "[t]", "[q]", "[Esc] Back"] {
                assert!(!footer.contains(key), "{footer}");
            }
            app.models.filtering = false;
            app.models.confirmation = Some(Confirmation::Download("reply-fixture".into()));
            let rendered = buffer(&app, width, height);
            let footer = text(&rendered, Rect::new(0, height - 3, width, 3));
            for hint in [
                "[y] Confirm local action",
                "[Esc] Cancel",
                "[PgUp/PgDn] Scroll",
                "Network/download or verified next-start choice only",
                "Active hosts unchanged; no model-management HTTP request",
            ] {
                assert!(footer.contains(hint), "missing {hint}: {footer}");
            }
            assert!(!footer.contains("[Esc] Back"), "{footer}");
            assert!(!footer.contains("[w]"), "{footer}");
            app.models.confirmation = None;
            app.models.command = true;
            let rendered = buffer(&app, width, height);
            let footer = text(&rendered, Rect::new(0, height - 3, width, 3));
            assert!(footer.contains("[Esc] Close"), "{footer}");
            assert!(!footer.contains("[Esc] Back"), "{footer}");
            app.models.command = false;
        }
    }

    #[test]
    fn models_metadata_is_label_aligned_and_scrollable() {
        let mut app = model_app();
        app.config.server_reply_model = Some("reply-fixture".into());
        let info = model_info(&app, &app.catalog.entries[0])
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        for (label, value) in [
            ("Family:", "qwen"),
            ("Purpose:", "Reasoning & reply"),
            ("Runtime:", "llama.cpp"),
            ("Revision:", "fixture-revision"),
            ("Size:", "1.5B"),
            ("Bytes:", "123456"),
            ("Quantization:", "Q4_K_M"),
            ("Languages:", "English, Chinese"),
            ("License:", "Apache-2.0"),
            ("Source:", "https://example.invalid/model"),
            ("SHA256:", "aaaaaaaa"),
            ("Role choice:", "manifest default"),
            ("Selected roles:", "1. incident-reporting.txt"),
            ("Next start:", "chosen (saved locally)"),
        ] {
            assert!(info.contains(&format!("{label:<16} {value}")), "{info}");
        }
        app.config.reply_role_files.insert(
            "reply-fixture".into(),
            vec![
                "/local/roles/summary.txt".into(),
                "/local/roles/incident-reporting.txt".into(),
            ],
        );
        let saved_info = model_info(&app, &app.catalog.entries[0])
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            saved_info.contains("Role choice:     saved per model"),
            "{saved_info}"
        );
        assert!(
            saved_info.contains("Selected roles:  1. summary.txt, 2. incident-reporting.txt"),
            "{saved_info}"
        );
        for (width, height) in [(80, 24), (120, 32)] {
            let (_, details) = columns(width, height);
            app.models.scroll = 0;
            let first = buffer(&app, width, height);
            assert!(text(&first, details).contains("reply-fixture"));
            let mut scrolled = String::new();
            for scroll in (0..100).step_by(3) {
                app.models.scroll = scroll;
                let buffer = buffer(&app, width, height);
                scrolled.push_str(&text(&buffer, details));
                assert!(text(&buffer, Rect::new(0, 0, width, 3)).contains("PHEME VA / MODELS"));
            }
            for label in [
                "Bytes:",
                "SHA256:",
                "Source:",
                "License:",
                "Selected roles:",
            ] {
                assert!(scrolled.contains(label), "{scrolled}");
            }
        }
    }

    #[test]
    fn confirmations_preview_and_command_stay_in_the_right_column() {
        let mut app = model_app();
        for (width, height) in [(80, 24), (120, 32)] {
            let (list, details) = columns(width, height);
            for confirmation in [
                Confirmation::Download("reply-fixture".into()),
                Confirmation::Choose("reply-fixture".into()),
            ] {
                app.models.confirmation = Some(confirmation);
                let buffer = buffer(&app, width, height);
                assert!(text(&buffer, list).contains("MANIFEST ENTRIES"));
                let right = text(&buffer, details);
                assert!(
                    right.contains("CONFIRM LOCAL DOWNLOAD")
                        || right.contains("VERIFIED NEXT-START CHOICE"),
                    "{right}"
                );
                assert!(text(&buffer, buffer.area).contains("[y] Confirm local action"));
                assert!(text(&buffer, Rect::new(0, 0, width, 3)).contains("MODELS / LOCAL FILES"));
            }
            app.models.confirmation = None;
            app.models.preview = Some(crate::model::LoadedPrompt {
                path: "models/roles/incident-reporting.txt".into(),
                sha256: "fixture-sha".into(),
                text: "Combined prompt fixture".into(),
            });
            let preview_buffer = buffer(&app, width, height);
            assert!(text(&preview_buffer, list).contains("MANIFEST ENTRIES"));
            let right = text(&preview_buffer, details);
            assert!(right.contains("COMBINED ROLE PREVIEW"));
            assert!(right.contains("Combined prompt fixture"));
            app.models.preview = None;
            app.models.command = true;
            let buffer = buffer(&app, width, height);
            assert!(text(&buffer, list).contains("MANIFEST ENTRIES"));
            assert!(text(&buffer, details).contains("NEXT-START COMMAND"));
            app.models.command = false;
        }
    }

    #[test]
    fn roles_show_manifest_default_checkmarks_and_selection_order_at_both_sizes() {
        let mut app = model_app();
        app.models.role_selection = Some(roles());
        for (width, height) in [(80, 24), (120, 32)] {
            let (list, details) = columns(width, height);
            let buffer = buffer(&app, width, height);
            assert!(text(&buffer, list).contains("MANIFEST ENTRIES"));
            let right = text(&buffer, details);
            assert!(right.contains("Default: incident-reporting.txt"), "{right}");
            assert!(right.contains("[✓ 1] incident-reporting.txt"), "{right}");
            assert!(right.contains("1. incident-reporting.txt"), "{right}");
            let rendered = text(&buffer, buffer.area);
            for command in [
                "[Space] Toggle",
                "[a] Add path",
                "[Enter] Save",
                "[Esc] Cancel",
                "next host start only",
            ] {
                assert!(rendered.contains(command), "{rendered}");
            }
        }
        let roles = app.models.role_selection.as_mut().unwrap();
        roles.selected_files = vec![
            roles.entries[2].clone(),
            roles.entries[0].clone(),
            roles.entries[1].clone(),
        ];
        roles.cursor = 2;
        for (width, height) in [(80, 24), (120, 32)] {
            let buffer = buffer(&app, width, height);
            let right = text(&buffer, columns(width, height).1);
            assert!(right.contains("[✓ 1] summary.txt"), "{right}");
            assert!(right.contains("[✓ 2] incident-reporting.txt"), "{right}");
            assert!(right.contains("[✓ 3] safety.txt"), "{right}");
            let first = right.find("1. summary.txt").unwrap();
            let second = right.find("2. incident-reporting.txt").unwrap();
            let third = right.find("3. safety.txt").unwrap();
            assert!(first < second && second < third, "{right}");
        }
    }

    #[test]
    fn role_order_does_not_inherit_detail_scroll_or_advertise_paging() {
        let mut app = model_app();
        app.models.role_selection = Some(roles());
        app.models.scroll = 100;
        for (width, height) in [(80, 24), (120, 32)] {
            let buffer = buffer(&app, width, height);
            let right = text(&buffer, columns(width, height).1);
            assert!(right.contains("SELECTION ORDER"), "{right}");
            assert!(right.contains("1. incident-reporting.txt"), "{right}");
            let rendered = text(&buffer, buffer.area);
            assert!(!rendered.contains("PgUp"), "{rendered}");
            assert!(!rendered.contains("PgDn"), "{rendered}");
            assert!(rendered.contains("[↑/↓] File"), "{rendered}");
        }
    }

    #[test]
    fn role_path_input_is_separate_with_a_visible_cursor_and_retained_order() {
        let mut app = model_app();
        let mut roles = roles();
        roles.adding_path = true;
        roles.path_input = "/local/役割/".repeat(24) + "safety.txt";
        roles.error = Some("Path must name an existing .txt file".into());
        app.models.role_selection = Some(roles);
        for (width, height) in [(80, 24), (120, 32)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| super::super::ui::draw(frame, &app))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let (_, details) = columns(width, height);
            let right = text(buffer, details);
            assert!(right.contains("ADD EXISTING LOCAL .txt PATH"), "{right}");
            assert!(right.contains("safety.txt"), "{right}");
            assert!(right.contains("1. incident-reporting.txt"), "{right}");
            assert!(
                right.contains("Path must name an existing .txt file"),
                "{right}"
            );
            assert!(!right.contains("LOCAL .txt FILES"));
            assert!(text(buffer, buffer.area).contains("shortcuts type text, not navigation"));
            let footer = text(buffer, Rect::new(0, height - 3, width, 3));
            assert!(footer.contains("[Esc] Cancel path"), "{footer}");
            assert!(footer.contains("[Enter] Add local .txt path"), "{footer}");
            for key in ["[w]", "[m]", "[b]", "[t]", "[q]", "[Esc] Back"] {
                assert!(!footer.contains(key), "{footer}");
            }
            assert!(text(buffer, Rect::new(0, 0, width, 3)).contains("PHEME VA / MODELS"));
            let cursor = terminal.get_cursor_position().unwrap();
            assert!(cursor.x > details.x && cursor.x < details.right() - 1);
            assert!(cursor.y > details.y && cursor.y < details.bottom() - 1);
        }
    }
}
