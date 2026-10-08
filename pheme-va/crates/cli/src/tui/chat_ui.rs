//! Conversation rendering uses the same retained snapshots as the detail views.
use super::{App, Detail};
use crate::tui::app::epoch_millis;
use crate::tui::workspace_ui::{draw_shortcut_footer, footer_height, header};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Block, Borders, Cell, Chart, Dataset, Paragraph, Row, Table, TableState, Wrap,
};
use ratatui::Frame;
use std::time::Instant;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use va_core::chat::{ChatPhase, ChatSnapshot};

impl App {
    pub fn draw_chat(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let narrow = area.width < 100;
        let footer = self.chat_footer();
        let footer_rows = footer_height(&footer, area.width);
        let rows = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(if narrow { 11 } else { 7 }),
            Constraint::Length(if narrow { 3 } else { 4 }),
            Constraint::Length(if narrow { 3 } else { 4 }),
            Constraint::Length(1),
            Constraint::Length(footer_rows),
        ])
        .split(area);
        header(frame, self, rows[0], "CHAT");
        let top = if area.width >= 100 {
            Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
                .split(rows[1])
        } else {
            Layout::vertical([Constraint::Length(3), Constraint::Min(5)]).split(rows[1])
        };
        let source = format!(
            "INPUT SOURCE\n[l] Live microphone  [f] Select WAV file\nFolder: {}\n{} WAV file(s)",
            self.folder.directory.display(),
            self.folder.wav_count()
        );
        let source = if area.width < 100 {
            format!(
                "[l] Mic  [f] WAV · {} · {} WAV file(s)",
                self.folder.directory.display(),
                self.folder.wav_count()
            )
        } else {
            source
        };
        frame.render_widget(
            Paragraph::new(source)
                .wrap(Wrap { trim: false })
                .block(Block::default().borders(Borders::ALL).title("SOURCE")),
            top[0],
        );
        let mut displayed = self.chat.snapshot().cloned();
        if let Some(snapshot) = displayed.as_mut() {
            for turn in &mut snapshot.turns {
                if turn.status == ChatPhase::Generating {
                    if let Some(text) = self
                        .chat
                        .streaming
                        .get(&turn.turn_id)
                        .filter(|text| text.len() > turn.reply.len())
                    {
                        turn.reply = text.clone();
                    }
                }
            }
        }
        let snapshot = displayed.as_ref();
        let title = snapshot
            .map(|s| s.title.as_str())
            .unwrap_or("New conversation");
        let chat = Layout::vertical([
            Constraint::Min(5),
            Constraint::Length(if narrow { 3 } else { 4 }),
        ])
        .split(top[1])
        .to_vec();
        let stt = self
            .active_model
            .as_ref()
            .map(|m| m.id.clone())
            .unwrap_or_else(|| "STT unavailable".into());
        let reply = self
            .local_runtime
            .as_ref()
            .map(|r| r.voice.reply_status().name)
            .unwrap_or_else(|| "Reply model".into());
        let mut lines = chat_lines(snapshot, chat[0].width.saturating_sub(2) as usize, &reply);
        let dots = [".", "..", "..."][((epoch_millis() / 400) % 3) as usize];
        if let Some(recording) = &self.recording {
            lines.push(Line::from(format!(
                "You · recording {:.1}s",
                recording.elapsed().as_secs_f32()
            )));
        } else if self.current_request.is_some()
            || self.chat.starting
                && self.chat.confirmed_text.is_none()
                && self.chat.pending_after.is_some()
            || snapshot
                .and_then(|s| s.turns.last())
                .is_some_and(|t| t.status == ChatPhase::Transcribing)
        {
            lines.push(Line::from(Span::styled(
                stt,
                Style::default().fg(Color::Cyan),
            )));
            lines.push(Line::from(format!("┌ {dots}  Transcribing audio ┐")));
        } else if snapshot.is_some_and(|s| s.status == "finishing") {
            lines.push(Line::from(Span::styled(
                reply,
                Style::default().fg(Color::Cyan),
            )));
            lines.push(Line::from(format!("┌ {dots}  Summarizing conversation ┐")));
        } else if snapshot
            .and_then(|s| s.turns.last())
            .is_some_and(|t| t.status == ChatPhase::Generating && t.reply.is_empty())
            || self.chat.submitting
            || self.chat.starting && self.chat.confirmed_text.is_some()
        {
            lines.push(Line::from(Span::styled(
                reply,
                Style::default().fg(Color::Cyan),
            )));
            lines.push(Line::from(format!("┌ {dots}  Generating reply ┐")));
        }
        let lines = wrap_chat_lines(lines, chat[0].width.saturating_sub(2) as usize);
        let line_count = lines.len();
        let paragraph = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("CHAT / {title}")),
        );
        let scroll = if self.chat.follow_bottom {
            line_count
                .saturating_sub(chat[0].height.saturating_sub(2) as usize)
                .min(u16::MAX as usize) as u16
        } else {
            self.chat.scroll
        };
        frame.render_widget(paragraph.scroll((scroll, 0)), chat[0]);
        if chat.len() > 1 {
            let block =
                Block::default()
                    .borders(Borders::ALL)
                    .title(if self.chat.draft_turn.is_none() {
                        if self.chat.editing {
                            "MESSAGE / EDITING / NOT SENT"
                        } else {
                            "MESSAGE / NOT SENT"
                        }
                    } else if self.chat.editing {
                        "EDIT YOUR MESSAGE / EDITING / NOT SENT"
                    } else {
                        "EDIT YOUR MESSAGE / NOT SENT"
                    });
            let inner = block.inner(chat[1]);
            frame.render_widget(block, chat[1]);
            let (lines, (x, y)) = self.chat.editor.wrapped(inner.width as usize);
            let offset = y.saturating_sub(inner.height.saturating_sub(1) as usize);
            frame.render_widget(Paragraph::new(lines[offset..].join("\n")), inner);
            if self.chat.editing && inner.width > 0 && inner.height > 0 {
                frame.set_cursor_position((
                    inner.x + x.min(inner.width.saturating_sub(1) as usize) as u16,
                    inner.y + (y - offset).min(inner.height.saturating_sub(1) as usize) as u16,
                ));
            }
        }
        let latest = snapshot.and_then(|s| s.turns.last());
        let stage = snapshot
            .filter(|s| {
                s.turns
                    .last()
                    .is_some_and(|t| t.status == ChatPhase::Transcribing)
            })
            .and_then(|s| s.runs.last())
            .and_then(|r| r.summaries.iter().find(|s| s.name == "processing_stage"))
            .map(|s| s.latest.as_str())
            .unwrap_or_else(|| {
                if snapshot.is_some_and(|s| s.status == "finishing") {
                    "generating conversation title"
                } else if self.recording.is_some() {
                    "recording"
                } else if self.current_request.is_some() || self.chat.starting {
                    if self.chat.confirmed_text.is_some() {
                        "sending message"
                    } else {
                        "transcribing"
                    }
                } else {
                    latest.map(|t| phase_label(t.status)).unwrap_or("idle")
                }
            });
        let details = format!(
            "Stage: {stage}  ·  Source: {}\nSTT run: {}  ·  Reply run: {}",
            latest
                .map(|t| t.source.as_str())
                .or(self.current_source.as_deref())
                .unwrap_or("none"),
            latest
                .and_then(|t| t.transcription_run.as_deref())
                .or(self.current_run.as_deref())
                .unwrap_or("—"),
            latest
                .and_then(|t| t.reasoning_run.as_deref())
                .unwrap_or("—")
        );
        let details = if narrow {
            format!(
                "Stage: {stage} · Source: {}",
                latest
                    .map(|t| t.source.as_str())
                    .or(self.current_source.as_deref())
                    .unwrap_or("none")
            )
        } else {
            details
        };
        frame.render_widget(
            Paragraph::new(details).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("TURN / RUN DETAILS"),
            ),
            rows[2],
        );
        let metrics = snapshot
            .map(|s| live_metrics(s, self.chat.started, narrow))
            .unwrap_or_else(|| "No conversation measurements yet.".into());
        frame.render_widget(
            Paragraph::new(metrics)
                .wrap(Wrap { trim: false })
                .block(Block::default().borders(Borders::ALL).title("LIVE METRICS")),
            rows[3],
        );
        frame.render_widget(
            Paragraph::new(
                self.chat
                    .error
                    .as_deref()
                    .or(self.chat.runtime_error.as_deref())
                    .or(self.error_message.as_deref())
                    .unwrap_or(&self.status_message),
            )
            .style(Style::default().fg(
                if self.chat.error.is_some() || self.chat.runtime_error.is_some() {
                    Color::Red
                } else {
                    Color::Gray
                },
            )),
            rows[4],
        );
        draw_shortcut_footer(frame, rows[5], &footer);
    }

    pub fn draw_conversations(&self, frame: &mut Frame<'_>, area: Rect) {
        if self.chat.detail == Detail::List {
            let snapshots = self.chat.filtered(&self.filter_query);
            let rows = snapshots.iter().map(|s| {
                Row::new(vec![
                    s.title.clone(),
                    s.status.clone(),
                    s.turns.len().to_string(),
                    s.runs
                        .iter()
                        .filter(|r| r.stage != "title")
                        .count()
                        .to_string(),
                ])
            });
            let table = Table::new(
                rows,
                [
                    Constraint::Percentage(60),
                    Constraint::Percentage(20),
                    Constraint::Percentage(10),
                    Constraint::Percentage(10),
                ],
            )
            .header(
                Row::new(["Title", "Status", "Turns", "Runs"])
                    .style(Style::default().fg(Color::Cyan)),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("CONVERSATIONS / ENTER TO OPEN"),
            )
            .row_highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");
            let mut selected = TableState::default().with_selected(Some(self.chat.selected));
            frame.render_stateful_widget(table, area, &mut selected);
            return;
        }
        let Some(snapshot) = self.chat.chosen(&self.filter_query) else {
            return;
        };
        match self.chat.detail {
            Detail::Runs => {
                let rows = snapshot.runs.iter().map(|r| {
                    Row::new(vec![
                        r.stage.clone(),
                        r.status.clone(),
                        r.model_id.clone(),
                        r.run_id.clone(),
                    ])
                });
                let table = Table::new(
                    rows,
                    [
                        Constraint::Length(15),
                        Constraint::Length(13),
                        Constraint::Percentage(30),
                        Constraint::Min(10),
                    ],
                )
                .header(
                    Row::new(["Stage", "Status", "Model", "Run"])
                        .style(Style::default().fg(Color::Cyan)),
                )
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!("{} / STAGE RUNS", snapshot.title)),
                )
                .row_highlight_style(Style::default().fg(Color::Cyan))
                .highlight_symbol("> ");
                frame.render_stateful_widget(
                    table,
                    area,
                    &mut TableState::default().with_selected(Some(self.chat.selected_run)),
                );
            }
            Detail::History => frame.render_widget(
                Paragraph::new(chat_lines(
                    Some(snapshot),
                    area.width.saturating_sub(2) as usize,
                    "Reply model",
                ))
                .wrap(Wrap { trim: false })
                .scroll((self.chat.scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!("{} / SAVED CHAT", snapshot.title)),
                ),
                area,
            ),
            Detail::Metric => {
                let summaries = self.chat_metric_summaries();
                let Some(summary) = summaries.get(self.chat.selected_metric) else {
                    return;
                };
                let samples = snapshot
                    .runs
                    .iter()
                    .filter(|r| {
                        !self.chat.metric_from_run
                            || snapshot
                                .runs
                                .get(self.chat.selected_run)
                                .is_some_and(|selected| selected.run_id == r.run_id)
                    })
                    .flat_map(|r| &r.events)
                    .filter(|e| {
                        e.name == summary.name
                            && e.source == summary.source
                            && e.scope == summary.scope
                            && e.unit == summary.unit
                    })
                    .collect::<Vec<_>>();
                let mut samples = samples;
                samples.sort_by_key(|e| (e.timestamp_ms, e.sequence));
                let points = samples
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.unavailable_reason.is_none())
                    .filter_map(|(index, e)| match e.value {
                        Some(metrics::MetricValue::Number(n)) if n.is_finite() => {
                            Some((index as f64, n))
                        }
                        Some(metrics::MetricValue::Integer(n)) => Some((index as f64, n as f64)),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let parts = Layout::vertical([
                    Constraint::Length(4),
                    Constraint::Length(4),
                    Constraint::Min(3),
                ])
                .split(area);
                frame.render_widget(Paragraph::new(format!("{} / {:?} / {:?} / {}\nLatest {} · Mean {} · Min {} · Max {}\nSamples {} · Unavailable {} · {}", summary.name, summary.unit, summary.scope, summary.source, summary.latest, number(summary.mean()), number(summary.minimum), number(summary.maximum), summary.count, summary.unavailable_count, summary.unavailable_reason.as_deref().unwrap_or(""))).wrap(Wrap { trim: false }), parts[0]);
                let min = points
                    .iter()
                    .map(|(_, n)| *n)
                    .reduce(f64::min)
                    .unwrap_or(0.0);
                let max = points
                    .iter()
                    .map(|(_, n)| *n)
                    .reduce(f64::max)
                    .unwrap_or(1.0);
                let pad = ((max - min) * 0.1).max(0.01);
                frame.render_widget(
                    Chart::new(vec![Dataset::default()
                        .data(&points)
                        .marker(ratatui::symbols::Marker::Braille)])
                    .x_axis(
                        Axis::default()
                            .bounds([0.0, samples.len().saturating_sub(1).max(1) as f64]),
                    )
                    .y_axis(Axis::default().bounds([min - pad, max + pad]))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("NUMERIC SAMPLES / gaps are unavailable"),
                    ),
                    parts[1],
                );
                let lines = samples
                    .iter()
                    .map(|e| {
                        format!(
                            "{} #{:04} {} {}",
                            crate::tui::logs::format_timestamp(e.timestamp_ms),
                            e.sequence,
                            crate::tui::telemetry::TelemetryStore::value_text(e),
                            e.unavailable_reason.as_deref().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                frame.render_widget(
                    Paragraph::new(lines).scroll((self.chat.scroll, 0)).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("RETAINED RAW EVENTS"),
                    ),
                    parts[2],
                );
            }
            Detail::Overview | Detail::Run => {
                let summary = if self.chat.detail == Detail::Run {
                    snapshot.runs.get(self.chat.selected_run).map(|r| format!("{} / {} / {}\nRun: {}\nSource: {} · Status: {} · Audio: {}\nRaw: {}\nSubmitted: {}\nOutput: {}\nError: {} · Retained-event truncation: {}", r.stage, r.model_id, r.backend, r.run_id, r.source, r.status, r.audio_duration_seconds.map_or("not applicable".into(), |s| format!("{s:.2}s")), r.raw_transcript, r.input, r.output, r.error.as_deref().unwrap_or("none"), r.dropped_events)).unwrap_or_default()
                } else {
                    overview(snapshot)
                };
                let parts = Layout::vertical([
                    Constraint::Length(if self.chat.detail == Detail::Run {
                        9
                    } else {
                        6
                    }),
                    Constraint::Min(4),
                ])
                .split(area);
                frame.render_widget(
                    Paragraph::new(summary)
                        .wrap(Wrap { trim: false })
                        .scroll((self.chat.scroll, 0))
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(snapshot.title.clone()),
                        ),
                    parts[0],
                );
                let summaries = self.chat_metric_summaries();
                let rows = summaries.iter().map(|s| {
                    Row::new(vec![
                        Cell::from(format!(
                            "{} / {:?} / {} / {:?}",
                            s.name, s.unit, s.source, s.scope
                        )),
                        Cell::from(s.latest.clone()),
                        Cell::from(number(s.mean())),
                        Cell::from(number(s.minimum)),
                        Cell::from(number(s.maximum)),
                        Cell::from(format!("{}/{}", s.count, s.unavailable_count)),
                    ])
                });
                let table = Table::new(
                    rows,
                    [
                        Constraint::Percentage(45),
                        Constraint::Percentage(13),
                        Constraint::Percentage(10),
                        Constraint::Percentage(10),
                        Constraint::Percentage(10),
                        Constraint::Min(6),
                    ],
                )
                .header(
                    Row::new([
                        "Metric / source / scope",
                        "Latest",
                        "Mean",
                        "Min",
                        "Max",
                        "N/N/A",
                    ])
                    .style(Style::default().fg(Color::Cyan)),
                )
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("ALL COLLECTED METRICS / ENTER FOR SAMPLES"),
                )
                .row_highlight_style(Style::default().fg(Color::Cyan))
                .highlight_symbol("> ");
                frame.render_stateful_widget(
                    table,
                    parts[1],
                    &mut TableState::default().with_selected(Some(self.chat.selected_metric)),
                );
            }
            Detail::List => {}
        }
    }

    pub fn conversation_footer(&self) -> String {
        if self.chat.detail == Detail::List {
            "[1-4] Tab  [j/k] Select  [Enter] Open conversation  [/] Search  [c] Clear history  [Esc] Back".into()
        } else {
            "[j/k] Select  [Enter] Details  [o] Overview  [r] Stage runs  [h] Chat history  [PgUp/PgDn] Scroll  [Esc] Back".into()
        }
    }
}

fn phase_label(phase: ChatPhase) -> &'static str {
    match phase {
        ChatPhase::Transcribing => "transcribing",
        ChatPhase::AwaitingReview => "awaiting review",
        ChatPhase::Generating => "generating",
        ChatPhase::Completed => "completed",
        ChatPhase::Failed => "failed",
        ChatPhase::Cancelled => "cancelled",
    }
}
fn number(number: Option<f64>) -> String {
    number.map_or("—".into(), |n| format!("{n:.2}"))
}
fn chat_lines(
    snapshot: Option<&ChatSnapshot>,
    width: usize,
    reply_model: &str,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(snapshot) = snapshot {
        for turn in &snapshot.turns {
            if let Some(text) = &turn.approved_text {
                lines.extend(bubble("You", text, width, true, Color::Yellow));
            } else if turn.status.terminal() {
                lines.push(Line::from(format!(
                    "You · {}\n{}",
                    phase_label(turn.status),
                    turn.transcript
                )));
            }
            if !turn.reply.is_empty() {
                let model = turn
                    .reasoning_run
                    .as_ref()
                    .and_then(|id| snapshot.runs.iter().find(|r| &r.run_id == id))
                    .map(|r| r.model_id.as_str())
                    .unwrap_or(reply_model);
                lines.extend(bubble(model, &turn.reply, width, false, Color::Cyan));
                if turn.status == ChatPhase::Generating {
                    lines.push(Line::from("▌"));
                }
            }
            if let Some(error) = &turn.error {
                lines.push(Line::from(Span::styled(
                    error.clone(),
                    Style::default().fg(Color::Red),
                )));
            }
        }
    }
    while lines.last().is_some_and(|line| line.to_string().is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push(Line::from(
            if snapshot
                .and_then(|s| s.turns.last())
                .is_some_and(|t| t.status == ChatPhase::AwaitingReview)
            {
                "Review your transcript below."
            } else {
                "Choose a source, or type a message, to begin."
            },
        ));
    }
    lines
}
fn bubble(label: &str, text: &str, width: usize, right: bool, color: Color) -> Vec<Line<'static>> {
    let width = width.max(8);
    let rows = wrap_chat_lines(
        text.lines()
            .map(|line| Line::from(line.to_owned()))
            .collect(),
        width.saturating_sub(4).min(64),
    );
    let body_width = rows.iter().map(Line::width).max().unwrap_or(0);
    let padding = if right {
        width.saturating_sub(body_width + 4)
    } else {
        0
    };
    let prefix = " ".repeat(padding);
    let mut lines = vec![
        Line::styled(format!("{prefix}{label}"), Style::default().fg(color)),
        Line::from(format!("{prefix}┌{}┐", "─".repeat(body_width + 2))),
    ];
    for row in rows {
        lines.push(Line::from(format!(
            "{prefix}│ {}{} │",
            row,
            " ".repeat(body_width.saturating_sub(row.width()))
        )));
    }
    lines.push(Line::from(format!(
        "{prefix}└{}┘",
        "─".repeat(body_width + 2)
    )));
    lines.push(Line::from(""));
    lines
}
fn duration(snapshot: &ChatSnapshot, stage: &str) -> Option<f64> {
    snapshot
        .runs
        .iter()
        .filter(|r| r.stage == stage)
        .filter_map(|r| {
            r.finished_at_ms
                .map(|end| end.saturating_sub(r.started_at_ms) as f64 / 1000.0)
        })
        .reduce(|a, b| a + b)
}
fn overview(snapshot: &ChatSnapshot) -> String {
    let completed = snapshot
        .turns
        .iter()
        .filter(|t| t.status == ChatPhase::Completed)
        .count();
    let failed = snapshot
        .turns
        .iter()
        .filter(|t| t.status == ChatPhase::Failed)
        .count();
    let cancelled = snapshot
        .turns
        .iter()
        .filter(|t| t.status == ChatPhase::Cancelled)
        .count();
    let review = snapshot
        .turns
        .iter()
        .filter_map(|t| t.timings.get("review_ms"))
        .sum::<f64>()
        / 1000.0;
    let stages = ["transcription", "reasoning", "title"].map(|stage| duration(snapshot, stage));
    let seconds = |n: Option<f64>| n.map_or("unavailable".into(), |n| format!("{n:.2}s"));
    let audio = snapshot
        .runs
        .iter()
        .filter_map(|r| r.audio_duration_seconds)
        .reduce(|a, b| a + b)
        .map(f64::from);
    let processing = stages.into_iter().flatten().reduce(|a, b| a + b);
    let wall = snapshot
        .finished_at_ms
        .or_else(|| matches!(snapshot.status.as_str(), "active" | "finishing").then(epoch_millis))
        .map(|end| end.saturating_sub(snapshot.started_at_ms) as f64 / 1000.0);
    format!("{} turns · {} completed · {failed} failed · {cancelled} cancelled · {} runs\nAudio {} · STT {} · Reasoning {} · Title overhead {}\nReview {} · Wall {} · Stage processing incl title {}\nMetrics {} · Resource sampling {} · Title {}", snapshot.turns.len(), completed, snapshot.runs.len(), seconds(audio), seconds(stages[0]), seconds(stages[1]), seconds(stages[2]), if snapshot.turns.iter().any(|t|t.timings.contains_key("review_ms")){seconds(Some(review))}else{"unavailable".into()}, seconds(wall), seconds(processing), if snapshot.metrics_enabled { "enabled" } else { "disabled" }, if snapshot.resource_sampling_enabled { "enabled" } else { "disabled" }, snapshot.title_status)
}
fn live_metrics(snapshot: &ChatSnapshot, started: Option<Instant>, narrow: bool) -> String {
    let latest = snapshot.runs.last();
    let value = |name: &str, scope: metrics::MetricScope, unit: &str| {
        latest
            .and_then(|r| {
                r.summaries
                    .iter()
                    .find(|s| s.name == name && s.scope == scope)
            })
            .filter(|s| s.latest != "unavailable")
            .map(|s| format!("{} {unit}", s.latest))
            .unwrap_or_else(|| "unavailable".into())
    };
    let stage_duration = |stage: &str| {
        snapshot
            .turns
            .last()
            .and_then(|turn| {
                if stage == "transcription" {
                    turn.transcription_run.as_ref()
                } else {
                    turn.reasoning_run.as_ref()
                }
            })
            .and_then(|id| snapshot.runs.iter().find(|run| &run.run_id == id))
            .and_then(|run| {
                run.finished_at_ms
                    .map(|end| end.saturating_sub(run.started_at_ms) as f64 / 1000.0)
            })
    };
    if narrow {
        let seconds = |n: Option<f64>| n.map_or("n/a".into(), |n| format!("{n:.2}s"));
        let compact = |name: &str, scope, unit: &str| {
            let n = value(name, scope, unit);
            if n == "unavailable" {
                "n/a".into()
            } else {
                n
            }
        };
        let ram = latest
            .and_then(|r| {
                r.summaries.iter().find(|s| {
                    s.name == "ram_usage_bytes" && s.scope == metrics::MetricScope::Process
                })
            })
            .and_then(|s| s.latest.parse::<f64>().ok())
            .map_or("n/a".into(), |bytes| format!("{:.0}MiB", bytes / 1048576.0));
        return format!(
            "STT {} · Reply {} · First {} · CPU {} · RAM {} · Power {}",
            seconds(stage_duration("transcription")),
            seconds(stage_duration("reasoning")),
            compact("reply_first_text_ms", metrics::MetricScope::Run, "ms"),
            compact("process_cpu_percent", metrics::MetricScope::Process, "%"),
            ram,
            compact(
                "whole_device_power_watts",
                metrics::MetricScope::Device,
                "W"
            )
        );
    }
    let seconds = |n: Option<f64>| n.map_or("unavailable".into(), |n| format!("{n:.2}s"));
    format!("Elapsed {} · First text {} · STT {} · Reply {}\nProcess CPU {} · Process RAM {} · Power {}", snapshot.turns.last().filter(|t| t.status.terminal()).and_then(|t|t.timings.get("turn_ms")).map(|ms|format!("{:.1}s",ms/1000.0)).or_else(|| started.map(|s| format!("{:.1}s",s.elapsed().as_secs_f32()))).unwrap_or("unavailable".into()), value("reply_first_text_ms", metrics::MetricScope::Run, "ms"), seconds(stage_duration("transcription")), seconds(stage_duration("reasoning")), value("process_cpu_percent", metrics::MetricScope::Process, "%"), value("ram_usage_bytes", metrics::MetricScope::Process, "bytes"), value("whole_device_power_watts", metrics::MetricScope::Device, "W"))
}

fn wrap_chat_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut wrapped = Vec::new();
    for line in lines {
        let style = line.spans.first().map(|s| s.style).unwrap_or_default();
        let mut row = String::new();
        for grapheme in line.to_string().graphemes(true) {
            if !row.is_empty() && row.width() + grapheme.width() > width {
                wrapped.push(Line::styled(std::mem::take(&mut row), style));
            }
            row.push_str(grapheme);
        }
        wrapped.push(Line::styled(row, style));
    }
    wrapped
}
