use std::time::Instant;

use anyhow::{ensure, Context, Result};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use va_core::chat::{ChatPhase, ChatRun, ChatSnapshot, ChatTurn, MetricSummary};

use super::{App, Screen, TelemetryTab};
use crate::recorder::Recording;
use crate::tui::chat_client::{Command, Connection, Event};
use crate::tui::connected::AudioInput;
use crate::tui::editor::Editor;
#[path = "chat_ui.rs"]
mod ui;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Detail {
    #[default]
    List,
    Overview,
    Runs,
    History,
    Run,
    Metric,
}

#[derive(Default)]
pub struct ChatState {
    pub archives: Vec<ChatSnapshot>,
    pub current: Option<String>,
    pub connection: Option<Connection>,
    pub editor: Editor,
    pub editing: bool,
    pub draft_turn: Option<String>,
    pub submitting: bool,
    pub starting: bool,
    pub pending_after: Option<String>,
    pub streaming: std::collections::HashMap<String, String>,
    pub spoken: Option<String>,
    pub confirmed_text: Option<String>,
    pub recording: bool,
    pub isolated: bool,
    pub new_after_finish: bool,
    pub started: Option<Instant>,
    pub selected: usize,
    pub detail: Detail,
    pub selected_run: usize,
    pub selected_metric: usize,
    pub metric_from_run: bool,
    pub scroll: u16,
    pub revision: u64,
    pub persisted_revision: u64,
    pub error: Option<String>,
    pub runtime_error: Option<String>,
    pub follow_bottom: bool,
    pub local_cancelled: bool,
    pub cancel_when_admitted: bool,
    pub retry_audio: Option<(AudioInput, String)>,
}

impl ChatState {
    pub fn snapshot(&self) -> Option<&ChatSnapshot> {
        self.current
            .as_ref()
            .and_then(|id| self.archives.iter().find(|s| &s.conversation_id == id))
    }
    pub fn chosen(&self, query: &str) -> Option<&ChatSnapshot> {
        self.filtered(query).get(self.selected).copied()
    }
    fn filtered(&self, query: &str) -> Vec<&ChatSnapshot> {
        let query = query.to_lowercase();
        self.archives
            .iter()
            .filter(|s| {
                query.is_empty()
                    || format!(
                        "{} {} {}",
                        s.title,
                        s.conversation_id,
                        s.turns
                            .iter()
                            .map(|t| t.transcript.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .collect()
    }
    pub fn blocked(&self) -> bool {
        self.starting
            || self.submitting
            || self.recording
            || self.snapshot().is_some_and(|s| {
                s.busy
                    || s.status == "finishing"
                    || s.turns.last().is_some_and(|t| !t.status.terminal())
            })
            || !self.editor.text.trim().is_empty()
    }
    pub(super) fn upsert(&mut self, mut snapshot: ChatSnapshot) {
        if let Some(old) = self
            .archives
            .iter_mut()
            .find(|s| s.conversation_id == snapshot.conversation_id)
        {
            for turn in &mut snapshot.turns {
                if let Some(previous) = old.turns.iter().find(|t| t.turn_id == turn.turn_id) {
                    if previous.status.terminal() && !turn.status.terminal()
                        || previous.status == ChatPhase::Generating
                            && turn.status == ChatPhase::AwaitingReview
                    {
                        *turn = previous.clone();
                    }
                }
            }
            if old.status == "finished" || old.status == "finishing" && snapshot.status == "active"
            {
                snapshot.status = old.status.clone();
                snapshot.title = old.title.clone();
                snapshot.title_status = old.title_status.clone();
                snapshot.finished_at_ms = old.finished_at_ms;
            }
            for run in &mut snapshot.runs {
                if let Some(previous) = old.runs.iter().find(|r| r.run_id == run.run_id) {
                    if run.events.is_empty() {
                        run.events = previous.events.clone();
                    }
                    for summary in &mut run.summaries {
                        if let Some(saved) = previous.summaries.iter().find(|s| {
                            s.name == summary.name
                                && s.source == summary.source
                                && s.scope == summary.scope
                                && s.unit == summary.unit
                                && s.count > summary.count
                        }) {
                            *summary = saved.clone();
                        }
                    }
                    run.dropped_events = run.dropped_events.max(previous.dropped_events);
                }
            }
            if *old == snapshot {
                return;
            }
            *old = snapshot;
        } else {
            self.archives.insert(0, snapshot);
        }
        while self
            .archives
            .iter()
            .map(|s| s.runs.len().max(1))
            .sum::<usize>()
            > crate::tui::history::MAX_REPORTS
        {
            let oldest = self
                .archives
                .iter()
                .enumerate()
                .filter(|(_, s)| s.status != "active" && s.status != "finishing")
                .min_by_key(|(_, s)| s.started_at_ms)
                .map(|(i, _)| i);
            if let Some(index) = oldest {
                self.archives.remove(index);
            } else {
                break;
            }
        }
        self.revision = self.revision.wrapping_add(1);
    }
    fn send(&self, command: Command) -> Result<()> {
        self.connection
            .as_ref()
            .context("chat connection unavailable")?
            .send(command)
    }
}

impl App {
    pub fn start_chat_connection(&mut self, runtime: va_runtime::AgentRuntime) {
        self.local_runtime = Some(runtime.clone());
        match Connection::start(runtime) {
            Ok(connection) => {
                self.chat.connection = Some(connection);
                self.chat.starting = true;
                if let Err(error) = self.chat.send(Command::New) {
                    self.chat.error = Some(error.to_string());
                    self.chat.starting = false;
                }
            }
            Err(error) => self.chat.error = Some(error.to_string()),
        }
    }

    pub fn tick_chat(&mut self) {
        let mut events = Vec::new();
        if let Some(connection) = self.chat.connection.as_mut() {
            for _ in 0..256 {
                if let Some(event) = connection.event() {
                    events.push(event);
                } else {
                    break;
                }
            }
        }
        for event in events {
            match event {
                Event::Created(snapshot) => {
                    self.chat.current = Some(snapshot.conversation_id.clone());
                    self.chat.starting = false;
                    self.chat.new_after_finish = false;
                    self.chat.pending_after = None;
                    self.chat.cancel_when_admitted = false;
                    self.chat.error.clone_from(&self.chat.runtime_error);
                    self.chat.upsert(snapshot);
                }
                Event::Snapshot(snapshot) => {
                    let current = self.chat.current.as_deref() == Some(&snapshot.conversation_id);
                    self.chat.upsert(snapshot);
                    let latest = self.chat.snapshot().and_then(|s| s.turns.last()).cloned();
                    let finished = self.chat.snapshot().is_some_and(|s| s.status == "finished");
                    if current {
                        if let Some(turn) = latest {
                            if self.chat.starting
                                && self.chat.pending_after.as_deref() != Some(&turn.turn_id)
                            {
                                self.chat.starting = false;
                                self.chat.pending_after = None;
                            }
                            if self.chat.cancel_when_admitted
                                && !self.chat.starting
                                && !turn.status.terminal()
                            {
                                let id = self.chat.current.clone().expect("current");
                                if self
                                    .chat
                                    .send(Command::Cancel {
                                        id,
                                        turn: turn.turn_id.clone(),
                                    })
                                    .is_ok()
                                {
                                    self.chat.cancel_when_admitted = false;
                                }
                                continue;
                            }
                            if turn.status == ChatPhase::AwaitingReview
                                && self.chat.draft_turn.as_deref() != Some(&turn.turn_id)
                            {
                                if turn.transcript.len() > crate::tui::editor::MAX_INPUT_BYTES {
                                    self.chat.error = Some("Transcript exceeds editor limit; discard and use a shorter clip.".into());
                                } else {
                                    self.chat.editor = Editor::new(turn.transcript.clone());
                                    self.chat.draft_turn = Some(turn.turn_id.clone());
                                    self.chat.editing = false;
                                }
                            }
                            if turn.status.terminal() {
                                self.chat.streaming.remove(&turn.turn_id);
                            }
                            if self.chat.draft_turn.as_deref() == Some(&turn.turn_id)
                                && (turn.approved_text.is_some() || turn.status.terminal())
                            {
                                self.chat.submitting = false;
                                self.chat.confirmed_text = None;
                                if turn.approved_text.is_some()
                                    || turn.status == ChatPhase::Cancelled
                                {
                                    self.chat.editor = Editor::default();
                                    self.chat.editing = false;
                                }
                            }
                            if turn.status == ChatPhase::Completed
                                && self.voice.tests.auto_voice
                                && self.chat.spoken.as_deref() != Some(&turn.turn_id)
                            {
                                self.chat.spoken = Some(turn.turn_id.clone());
                                self.speak(&turn.reply);
                            }
                        }
                        if finished && self.chat.new_after_finish && !self.chat.starting {
                            self.chat.current = None;
                            self.chat.starting = true;
                            if let Err(error) = self.chat.send(Command::New) {
                                self.chat.error = Some(error.to_string());
                                self.chat.starting = false;
                            }
                        }
                    }
                }
                Event::Metrics {
                    id,
                    events,
                    truncated,
                } => {
                    if let Some(snapshot) = self
                        .chat
                        .archives
                        .iter_mut()
                        .find(|s| s.conversation_id == id)
                    {
                        for event in events {
                            if let Some(run) =
                                snapshot.runs.iter_mut().find(|r| r.run_id == event.run_id)
                            {
                                if !run.events.iter().any(|old| old.sequence == event.sequence) {
                                    if run.events.len() == va_core::chat::MAX_RUN_EVENTS {
                                        run.events.remove(0);
                                        run.dropped_events += 1;
                                    }
                                    run.events.push(event.clone());
                                    // The shared local metrics subscriber already feeds live telemetry.
                                }
                            }
                        }
                        if truncated {
                            self.chat.error = Some("Live metric cursor lost samples; aggregates remain retained in the local runtime.".into());
                        }
                        self.chat.revision = self.chat.revision.wrapping_add(1);
                    }
                }
                Event::Test(event) => {
                    if let Some(text) = self.voice.event(event) {
                        self.speak(&text);
                    }
                }
                Event::Run { id, run } => {
                    if let Some(snapshot) = self
                        .chat
                        .archives
                        .iter_mut()
                        .find(|s| s.conversation_id == id)
                    {
                        if let Some(old) = snapshot.runs.iter_mut().find(|r| r.run_id == run.run_id)
                        {
                            *old = *run;
                        }
                        self.chat.revision = self.chat.revision.wrapping_add(1);
                    }
                }
                Event::Submitted => {
                    if self.chat.confirmed_text.as_deref() == Some(&self.chat.editor.text) {
                        self.chat.editor = Editor::default();
                        self.chat.editing = false;
                    }
                    self.chat.confirmed_text = None;
                    self.chat.draft_turn = None;
                    self.chat.submitting = false;
                    self.chat.starting = false;
                }
                Event::Error { action, message } => {
                    self.chat.error = Some(message);
                    if action == "new" || action == "start" {
                        self.chat.starting = false;
                        self.chat.confirmed_text = None;
                        self.chat.cancel_when_admitted = false;
                    }
                    if action == "submit" {
                        self.chat.submitting = false;
                        self.chat.confirmed_text = None;
                    }
                    if action == "finish" {
                        self.chat.new_after_finish = false;
                    }
                }
            }
        }
        if self.chat.connection.is_none() {
            self.observe_local_chat();
        }
    }

    fn ensure_local_chat(&mut self) {
        if self.chat.snapshot().is_some_and(|s| s.status == "active") {
            return;
        }
        let now = super::epoch_millis();
        let id = format!("local-{}", self.take_run_id());
        self.chat.current = Some(id.clone());
        self.chat.upsert(ChatSnapshot {
            conversation_id: id,
            title: va_core::chat::placeholder_title(now),
            title_status: "placeholder".into(),
            status: "active".into(),
            started_at_ms: now,
            finished_at_ms: None,
            turns: Vec::new(),
            runs: Vec::new(),
            busy: false,
            metrics_enabled: self.config.metrics_enabled,
            resource_sampling_enabled: self.config.resource_sampling_enabled,
        });
    }

    fn observe_local_chat(&mut self) {
        let Some(run_id) = self.current_run.clone() else {
            return;
        };
        let Some(report) = self.telemetry.report(&run_id).cloned() else {
            return;
        };
        self.ensure_local_chat();
        let id = self.chat.current.clone().expect("local");
        let index = self
            .chat
            .archives
            .iter()
            .position(|s| s.conversation_id == id)
            .expect("local");
        let mut snapshot = self.chat.archives[index].clone();
        let events = if report.status == "BUSY" {
            self.telemetry
                .events_for(Some(&run_id))
                .into_iter()
                .cloned()
                .collect()
        } else {
            report.events.clone()
        };
        let run = local_run(&report, events, self.chat.local_cancelled);
        if let Some(old) = snapshot.runs.iter_mut().find(|r| r.run_id == run_id) {
            *old = run;
        } else {
            snapshot.runs.push(run);
        }
        let status = if self.chat.local_cancelled {
            ChatPhase::Cancelled
        } else if report.status == "BUSY" {
            ChatPhase::Transcribing
        } else if report.status == "speech" {
            ChatPhase::AwaitingReview
        } else {
            ChatPhase::Failed
        };
        let turn = ChatTurn {
            turn_id: run_id.clone(),
            status,
            transcript: report.transcript.clone(),
            approved_text: None,
            reply: String::new(),
            error: report.error.clone(),
            timings: Default::default(),
            source: report.source.clone(),
            transcription_run: Some(run_id.clone()),
            reasoning_run: None,
        };
        if let Some(old) = snapshot.turns.iter_mut().find(|t| t.turn_id == run_id) {
            *old = turn;
        } else {
            snapshot.turns.push(turn);
        }
        snapshot.busy = self.current_request.is_some();
        self.chat.upsert(snapshot);
        if status == ChatPhase::AwaitingReview && self.chat.draft_turn.as_deref() != Some(&run_id) {
            self.chat.editor = Editor::new(report.transcript);
            self.chat.draft_turn = Some(run_id);
            self.chat.editing = false;
        }
    }

    pub fn start_chat_audio(&mut self, audio: AudioInput, source: String) {
        let result: Result<()> = (|| {
            ensure!(
                !self.chat.blocked(),
                "Send or discard the pending turn first."
            );
            let id = self
                .chat
                .current
                .clone()
                .context("Start a conversation first.")?;
            ensure!(
                self.chat.snapshot().is_some_and(|s| s.status == "active"),
                "Start a new conversation."
            );
            let key = self.take_run_id();
            self.chat.send(Command::Audio {
                id,
                audio: audio.clone(),
                source: source.clone(),
                max_seconds: self.config.max_seconds,
                key,
            })?;
            self.chat.retry_audio = Some((audio, source));
            self.chat.pending_after = self
                .chat
                .snapshot()
                .and_then(|s| s.turns.last())
                .map(|t| t.turn_id.clone());
            self.chat.starting = true;
            self.chat.started = Some(Instant::now());
            self.chat.error = None;
            self.chat.follow_bottom = true;
            self.chat.draft_turn = None;
            Ok(())
        })();
        if let Err(error) = result {
            self.chat.error = Some(error.to_string());
        }
        self.screen = Screen::Bench;
    }

    fn send_chat_message(&mut self) {
        let result: Result<()> = (|| {
            ensure!(
                self.chat.connection.is_some(),
                "Local runtime is loading or unavailable."
            );
            ensure!(
                self.local_runtime.as_ref().is_some_and(|runtime| runtime.voice.reply_status().ready),
                "{}",
                self.chat.runtime_error.as_deref().unwrap_or("Reply model unavailable. Use Models to save a reply choice, then restart the TUI.")
            );
            ensure!(
                !self.chat.submitting
                    && !self.chat.starting
                    && !self.chat.snapshot().is_some_and(|s| s.busy
                        || s.status != "active"
                        || s.turns.last().is_some_and(|t| matches!(
                            t.status,
                            ChatPhase::Transcribing | ChatPhase::Generating
                        ))),
                "Submission is already pending."
            );
            ensure!(
                !self.chat.editor.text.trim().is_empty(),
                "Enter a message first."
            );
            let id = self
                .chat
                .current
                .clone()
                .context("No active conversation.")?;
            let text = self.chat.editor.text.clone();
            if let Some(turn) = self.chat.draft_turn.clone() {
                ensure!(self.chat.snapshot().and_then(|s| s.turns.last()).is_some_and(|t| t.turn_id == turn && t.status == ChatPhase::AwaitingReview), "This draft is no longer awaiting review; discard it explicitly.");
                self.chat.send(Command::Submit {
                    id,
                    turn,
                    text: text.clone(),
                })?;
                self.chat.confirmed_text = Some(text);
                self.chat.submitting = true;
            } else {
                let key = self.take_run_id();
                self.chat.send(Command::Text {
                    id,
                    text: text.clone(),
                    key,
                })?;
                self.chat.confirmed_text = Some(text);
                self.chat.pending_after = self
                    .chat
                    .snapshot()
                    .and_then(|s| s.turns.last())
                    .map(|t| t.turn_id.clone());
                self.chat.starting = true;
            }
            self.chat.editing = false;
            self.chat.error = None;
            self.chat.started = Some(Instant::now());
            self.chat.follow_bottom = true;
            Ok(())
        })();
        if let Err(error) = result {
            self.chat.error = Some(error.to_string());
        }
    }

    fn finish_chat(&mut self, new: bool) {
        if self.chat.blocked() || self.current_request.is_some() {
            self.chat.error = Some(
                "Send/discard the pending turn and wait for processing to settle first.".into(),
            );
            return;
        }
        if self.chat.connection.is_some() {
            let result = if let Some(id) = self
                .chat
                .current
                .clone()
                .filter(|_| self.chat.snapshot().is_some_and(|s| s.status == "active"))
            {
                self.chat.new_after_finish = new;
                self.chat.send(Command::Finish { id })
            } else if new {
                self.chat.current = None;
                self.chat.starting = true;
                self.chat.send(Command::New)
            } else {
                Ok(())
            };
            if let Err(error) = result {
                self.chat.error = Some(error.to_string());
                self.chat.new_after_finish = false;
                self.chat.starting = false;
            }
        } else {
            if let Some(id) = &self.chat.current {
                if let Some(snapshot) = self
                    .chat
                    .archives
                    .iter_mut()
                    .find(|s| &s.conversation_id == id)
                {
                    snapshot.status = "finished".into();
                    snapshot.finished_at_ms = Some(super::epoch_millis());
                }
            }
            self.current_run = None;
            self.chat.current = None;
            self.chat.draft_turn = None;
            if new {
                self.ensure_local_chat();
            }
            self.chat.revision = self.chat.revision.wrapping_add(1);
        }
    }

    fn cancel_chat(&mut self) {
        if let Some(recording) = self.recording.take() {
            recording.discard();
            self.chat.recording = false;
        }
        if self.chat.connection.is_some() {
            if self.chat.starting {
                self.chat.cancel_when_admitted = true;
            }
            if let (Some(id), Some(turn)) = (
                self.chat.current.clone(),
                self.chat
                    .snapshot()
                    .and_then(|s| s.turns.last())
                    .filter(|t| !t.status.terminal())
                    .map(|t| t.turn_id.clone()),
            ) {
                if let Err(error) = self.chat.send(Command::Cancel { id, turn }) {
                    self.chat.error = Some(error.to_string());
                    return;
                }
            }
        } else {
            self.chat.local_cancelled = true;
        }
        self.chat.editor = Editor::default();
        self.chat.editing = false;
        self.chat.confirmed_text = None;
        self.chat.draft_turn = None;
        self.status_message =
            "turn discarded/cancelled; native work must settle before reuse".into();
    }

    pub fn chat_key(&mut self, key: KeyEvent) -> Result<bool> {
        if !matches!(self.screen, Screen::Bench | Screen::Processing) || self.chat.isolated {
            return Ok(false);
        }
        if self.chat.editing {
            match key.code {
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
                    self.chat.editor.insert("\n")
                }
                KeyCode::Enter => self.send_chat_message(),
                KeyCode::Esc => self.chat.editing = false,
                code => self.chat.editor.key(code),
            }
            return Ok(true);
        }
        match key.code {
            KeyCode::Enter if self.recording.is_some() => {
                self.stop_recording();
            }
            KeyCode::Enter => self.send_chat_message(),
            KeyCode::Char('e')
                if !self.chat.editor.text.is_empty()
                    && !self.chat.submitting
                    && !self.chat.starting =>
            {
                self.chat.editing = true
            }
            KeyCode::Char('i')
                if self.chat.draft_turn.is_none()
                    && !self.chat.submitting
                    && !self.chat.starting
                    && !self.chat.recording =>
            {
                self.chat.editing = true;
            }
            KeyCode::Char('c') => self.finish_chat(true),
            KeyCode::Char('d') => self.finish_chat(false),
            KeyCode::Char('x') => self.cancel_chat(),
            KeyCode::Char('s') => {
                self.chat.isolated = true;
                self.navigate_to(Screen::ServerTests);
            }
            KeyCode::Char('l') if !self.chat.blocked() => {
                if self.chat.connection.is_some() {
                    self.playback.take();
                    match Recording::start() {
                        Ok(recording) => {
                            self.recording = Some(recording);
                            self.chat.recording = true;
                        }
                        Err(error) => self.chat.error = Some(error.to_string()),
                    }
                } else {
                    self.chat.local_cancelled = false;
                    self.start_recording();
                    self.chat.recording = true;
                    self.screen = Screen::Bench;
                }
            }
            KeyCode::Char('f') if !self.chat.blocked() => {
                self.folder.refresh();
                self.navigate_to(Screen::Folder);
            }
            KeyCode::Char('n') if !self.chat.blocked() => {
                self.chat.local_cancelled = false;
                if let Some(path) = self.folder.next_wav() {
                    self.start_file(path);
                }
            }
            KeyCode::Char('r') if !self.chat.blocked() => {
                self.chat.local_cancelled = false;
                if self.chat.connection.is_some() {
                    if let Some((audio, source)) = self.chat.retry_audio.clone() {
                        self.start_chat_audio(audio, source);
                    } else {
                        self.chat.error = Some("No previous audio source to retry.".into());
                    }
                } else {
                    self.retry_last();
                }
            }
            KeyCode::Char('p') => {
                if let Some(text) = self
                    .chat
                    .snapshot()
                    .and_then(|s| s.turns.last())
                    .filter(|t| t.status == ChatPhase::Completed)
                    .map(|t| t.reply.clone())
                {
                    self.speak(&text);
                }
            }
            KeyCode::Char('v') => self.voice.tests.auto_voice = !self.voice.tests.auto_voice,
            KeyCode::Char('j') | KeyCode::Down | KeyCode::PageDown => {
                self.chat.follow_bottom = false;
                self.chat.scroll = self.chat.scroll.saturating_add(3);
            }
            KeyCode::Char('k') | KeyCode::Up | KeyCode::PageUp => {
                self.chat.follow_bottom = false;
                self.chat.scroll = self.chat.scroll.saturating_sub(3);
            }
            KeyCode::End => {
                self.chat.follow_bottom = true;
            }
            KeyCode::Esc if self.recording.is_some() => self.cancel_chat(),
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub fn chat_telemetry_key(&mut self, code: KeyCode) -> bool {
        if self.telemetry_tab != TelemetryTab::Runs
            || self.chat.archives.is_empty()
            || self.filter_editing
            || self.clear_runs_pending
        {
            return false;
        }
        let snapshots = self.chat.filtered(&self.filter_query);
        let count = snapshots.len();
        let snapshot = snapshots.get(self.chat.selected).copied();
        let runs = snapshot.map_or(0, |s| s.runs.len());
        let metrics = self.chat_metric_summaries().len();
        match code {
            KeyCode::Char('1'..='4') | KeyCode::Char('/') | KeyCode::Char('f') => return false,
            KeyCode::Char('c') => return false,
            KeyCode::Enter => match self.chat.detail {
                Detail::List => {
                    if snapshot.is_some() {
                        self.chat.detail = Detail::Overview;
                        self.chat.selected_metric = 0;
                    }
                }
                Detail::Runs => {
                    if runs > 0 {
                        self.chat.detail = Detail::Run;
                        self.chat.selected_metric = 0;
                    }
                }
                Detail::Overview | Detail::Run if metrics > 0 => {
                    self.chat.metric_from_run = self.chat.detail == Detail::Run;
                    self.chat.detail = Detail::Metric;
                }
                _ => {}
            },
            KeyCode::Char('o') if self.chat.detail != Detail::List => {
                self.chat.detail = Detail::Overview
            }
            KeyCode::Char('r') if self.chat.detail != Detail::List => {
                self.chat.detail = Detail::Runs
            }
            KeyCode::Char('h') if self.chat.detail != Detail::List => {
                self.chat.detail = Detail::History
            }
            KeyCode::Esc => {
                self.chat.detail = match self.chat.detail {
                    Detail::List => return false,
                    Detail::Metric => {
                        if self.chat.metric_from_run {
                            Detail::Run
                        } else {
                            Detail::Overview
                        }
                    }
                    Detail::Run => Detail::Runs,
                    _ => Detail::List,
                };
            }
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('k') | KeyCode::Up => {
                let down = matches!(code, KeyCode::Char('j') | KeyCode::Down);
                let (index, len) = match self.chat.detail {
                    Detail::List => (&mut self.chat.selected, count),
                    Detail::Runs => (&mut self.chat.selected_run, runs),
                    Detail::Overview | Detail::Run | Detail::Metric => {
                        (&mut self.chat.selected_metric, metrics)
                    }
                    Detail::History => {
                        self.chat.scroll = if down {
                            self.chat.scroll.saturating_add(1)
                        } else {
                            self.chat.scroll.saturating_sub(1)
                        };
                        return true;
                    }
                };
                if len > 0 {
                    *index = if down {
                        (*index + 1) % len
                    } else {
                        (*index + len - 1) % len
                    };
                }
            }
            KeyCode::PageDown => self.chat.scroll = self.chat.scroll.saturating_add(5),
            KeyCode::PageUp => self.chat.scroll = self.chat.scroll.saturating_sub(5),
            _ => return false,
        }
        self.chat.selected_run = self.chat.selected_run.min(runs.saturating_sub(1));
        true
    }

    fn chat_metric_summaries(&self) -> Vec<MetricSummary> {
        let Some(snapshot) = self.chat.chosen(&self.filter_query) else {
            return Vec::new();
        };
        if self.chat.detail == Detail::Run
            || (self.chat.detail == Detail::Metric && self.chat.metric_from_run)
        {
            snapshot
                .runs
                .get(self.chat.selected_run)
                .map(|r| r.summaries.clone())
                .unwrap_or_default()
        } else {
            snapshot.summaries(true)
        }
    }

    pub fn chat_footer(&self) -> String {
        if self.chat.editing {
            return "[Enter] Confirm & send  [Alt+Enter] New line  [Esc] Review  [Arrows] Move cursor  [Backspace/Delete] Delete".into();
        }
        let mut footer =
            "[b] Chat  [m] Models  [t] Telemetry  [w] Web  [c] New  [d] Finish".to_owned();
        if self.recording.is_some() {
            footer.push_str("  [Enter] Stop recording  [x] Discard");
        } else if self.chat.starting
            || self.chat.submitting
            || self.chat.snapshot().is_some_and(|s| s.busy)
        {
            footer.push_str("  [x] Cancel  [i] Type next");
        } else if !self.chat.editor.text.is_empty() {
            footer.push_str(if self.chat.draft_turn.is_some() {
                "  [Enter] Confirm & send  [e] Edit  [x] Discard"
            } else {
                "  [Enter] Send  [e] Edit  [x] Discard"
            });
        } else {
            footer.push_str("  [i] Type  [l] Mic  [f] WAV  [n] Next file  [r] Retry");
        }
        if self.chat.connection.is_some() {
            footer.push_str("  [s] Tests");
        }
        footer.push_str("  [p] Replay  [v] Auto voice  [z] Stop voice  [j/k] Scroll  [End] Latest  [?] Help  [q] Quit");
        footer
    }
}

fn local_run(
    report: &crate::tui::telemetry::RunReport,
    events: Vec<metrics::MetricEvent>,
    cancelled: bool,
) -> ChatRun {
    let mut run = ChatRun {
        run_id: report.run_id.clone(),
        turn_id: Some(report.run_id.clone()),
        stage: "transcription".into(),
        model_id: report.model_id.clone(),
        backend: report.backend.clone(),
        revision: report.revision.clone(),
        role_sha256: None,
        source: report.source.clone(),
        status: if cancelled {
            "cancelled".into()
        } else {
            report.status.clone()
        },
        started_at_ms: report.started_at_ms,
        finished_at_ms: report.finished_at_ms,
        input: String::new(),
        output: report.transcript.clone(),
        raw_transcript: report.raw_transcript.clone(),
        audio_duration_seconds: (report.audio_duration_seconds > 0.0 || report.result.is_some())
            .then_some(report.audio_duration_seconds),
        language: report.language.clone(),
        gate_decision: Some(report.gate_decision.clone()),
        error: report.error.clone(),
        events: Vec::new(),
        summaries: Vec::new(),
        dropped_events: 0,
    };
    for event in events {
        run.observe(event);
    }
    run
}
pub fn legacy_snapshot(reports: &[crate::tui::telemetry::RunReport]) -> Option<ChatSnapshot> {
    if reports.is_empty() {
        return None;
    }
    Some(ChatSnapshot {
        conversation_id: "legacy-standalone".into(),
        title: "Legacy standalone runs".into(),
        title_status: "legacy".into(),
        status: "archived".into(),
        started_at_ms: reports
            .iter()
            .map(|r| r.started_at_ms)
            .min()
            .unwrap_or_default(),
        finished_at_ms: reports.iter().filter_map(|r| r.finished_at_ms).max(),
        turns: Vec::new(),
        runs: reports
            .iter()
            .map(|r| local_run(r, r.events.clone(), false))
            .collect(),
        busy: false,
        metrics_enabled: true,
        resource_sampling_enabled: true,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::workspace_ui::footer_height;
    use ratatui::crossterm::event::Event as TerminalEvent;
    use unicode_width::UnicodeWidthStr;

    fn key(app: &mut App, code: KeyCode) {
        app.handle_terminal_event(TerminalEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)))
            .unwrap();
    }

    fn wait_app(app: &mut App, mut ready: impl FnMut(&App) -> bool) {
        let until = Instant::now() + std::time::Duration::from_secs(3);
        loop {
            app.tick_chat();
            if ready(app) {
                break;
            }
            assert!(
                Instant::now() < until,
                "local UI state did not arrive: {:?}",
                app.chat.error
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    #[test]
    fn typed_ui_is_local_even_with_web_target_and_keeps_next_draft_during_generation() {
        let mut app = super::super::tests::test_app();
        app.screen = Screen::Bench;
        app.voice.target = Some("http://127.0.0.1:1".into());
        let (agent, control) = crate::tui::chat_client::tests::agent();
        let (sender, receiver) = std::sync::mpsc::sync_channel(4096);
        let _subscription =
            agent
                .metrics_hub
                .subscribe(crate::tui::telemetry::MetricForwarder::new(
                    sender,
                    std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
                ));
        app.metric_receiver = receiver;
        app.start_chat_connection(agent);
        wait_app(&mut app, |a| a.chat.current.is_some());
        control
            .hold
            .store(true, std::sync::atomic::Ordering::Release);
        key(&mut app, KeyCode::Char('i'));
        app.chat.editor.insert("  smoke — 你好\n ");
        key(&mut app, KeyCode::Enter);
        wait_app(&mut app, |a| {
            a.chat.editor.text.is_empty()
                && a.chat.snapshot().is_some_and(|s| {
                    s.turns
                        .last()
                        .is_some_and(|t| t.status == ChatPhase::Generating)
                })
        });
        assert!(
            app.voice.connection.is_none(),
            "Chat must not open Web/Go transport"
        );
        key(&mut app, KeyCode::Char('i'));
        app.chat.editor.insert("next draft — café");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.chat.editor.text, "next draft — café");
        assert_eq!(control.calls.lock().unwrap().len(), 1);
        control
            .hold
            .store(false, std::sync::atomic::Ordering::Release);
        wait_app(&mut app, |a| {
            a.chat.snapshot().is_some_and(|s| {
                !s.busy
                    && s.turns
                        .last()
                        .is_some_and(|t| t.status == ChatPhase::Completed)
            })
        });
        assert_eq!(app.chat.editor.text, "next draft — café");
        key(&mut app, KeyCode::Enter);
        wait_app(&mut app, |a| {
            a.chat.snapshot().is_some_and(|s| {
                s.turns.len() == 2 && !s.busy && s.turns[1].status == ChatPhase::Completed
            })
        });
        assert_eq!(
            control.calls.lock().unwrap()[0][1].content,
            "  smoke — 你好\n "
        );
        assert_eq!(
            control.calls.lock().unwrap()[1][3].content,
            "next draft — café"
        );
        app.telemetry.drain(&app.metric_receiver);
        let live = app.telemetry.events().collect::<Vec<_>>();
        assert!(!live.is_empty());
        let unique = live
            .iter()
            .map(|event| (&event.run_id, event.sequence))
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            unique.len(),
            live.len(),
            "snapshot recovery must not duplicate live metrics"
        );
        app.shutdown_workspace();
        assert_eq!(
            control.dropped.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn new_conversation_retains_reply_startup_error() {
        let mut app = super::super::tests::test_app();
        app.ensure_local_chat();
        let snapshot = app.chat.snapshot().unwrap().clone();
        let (connection, events, _) = Connection::fixture();
        app.chat.connection = Some(connection);
        app.chat.runtime_error = Some("Reply worker missing; build and restart.".into());
        events.try_send(Event::Created(snapshot)).ok().unwrap();
        app.tick_chat();
        assert_eq!(app.chat.error, app.chat.runtime_error);
    }

    #[test]
    fn review_is_not_focused_editing_owns_letters_and_escape_preserves_changes() {
        let mut app = super::super::tests::test_app();
        app.screen = Screen::Bench;
        app.chat.editor = Editor::new("original — café".into());
        assert!(!app.chat.editing);
        key(&mut app, KeyCode::Char('e'));
        assert!(app.chat.editing);
        for ch in "bmtcwdxq123".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        assert_eq!(app.screen, Screen::Bench);
        assert_eq!(app.chat.editor.text, "original — cafébmtcwdxq123");
        app.handle_terminal_event(TerminalEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::ALT,
        )))
        .unwrap();
        app.handle_terminal_event(TerminalEvent::Paste("你好\nsecond line".into()))
            .unwrap();
        assert!(app.chat.editor.text.ends_with("\n你好\nsecond line"));
        key(&mut app, KeyCode::Esc);
        assert!(!app.chat.editing);
        assert!(app.chat.editor.text.ends_with("second line"));
        key(&mut app, KeyCode::Enter);
        assert!(app.chat.error.as_deref().unwrap().contains("Local runtime"));
        assert!(app.chat.editor.text.ends_with("second line"));
    }

    #[test]
    fn composer_renders_immediately_and_controls_stay_in_footer_at_both_sizes() {
        use ratatui::{
            backend::{Backend, TestBackend},
            Terminal,
        };
        let mut app = super::super::tests::test_app();
        app.screen = Screen::Bench;
        key(&mut app, KeyCode::Char('i'));
        for (width, height) in [(80, 24), (120, 36)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| app.draw_chat(frame)).unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(text.contains("MESSAGE / EDITING / NOT SENT"));
            assert!(text.contains("[Enter] Confirm & send"));
            assert!(text.contains("[Alt+Enter] New line"));
            let cursor = terminal.backend_mut().get_cursor_position().unwrap();
            assert!(cursor.y < height - footer_height(&app.chat_footer(), width));
        }
    }

    #[test]
    fn new_and_finish_cannot_lose_a_pending_draft() {
        let mut app = super::super::tests::test_app();
        app.screen = Screen::Bench;
        app.ensure_local_chat();
        app.chat.editor = Editor::new("not sent".into());
        let current = app.chat.current.clone();
        for ch in ['c', 'd'] {
            key(&mut app, KeyCode::Char(ch));
            assert_eq!(app.chat.current, current);
            assert_eq!(app.chat.snapshot().unwrap().status, "active");
            assert_eq!(app.chat.editor.text, "not sent");
        }
        key(&mut app, KeyCode::Char('x'));
        key(&mut app, KeyCode::Char('d'));
        assert!(app.chat.current.is_none());
        assert_eq!(app.chat.archives[0].status, "finished");
    }
    #[test]
    fn enter_sends_exact_review_text_and_stream_snapshot_interleaving_keeps_one_reply() {
        let mut app = super::super::tests::test_app();
        app.voice.target = Some("http://127.0.0.1:8080".into());
        app.screen = Screen::Bench;
        let (connection, events, mut commands) = Connection::fixture();
        app.local_runtime = Some(crate::tui::chat_client::tests::agent().0);
        app.chat.connection = Some(connection);
        app.ensure_local_chat();
        let mut snapshot = app.chat.snapshot().unwrap().clone();
        snapshot.turns.push(ChatTurn {
            turn_id: "turn-1".into(),
            status: ChatPhase::AwaitingReview,
            transcript: "draft — café".into(),
            approved_text: None,
            reply: String::new(),
            error: None,
            timings: Default::default(),
            source: "microphone".into(),
            transcription_run: None,
            reasoning_run: None,
        });
        events
            .try_send(Event::Snapshot(snapshot.clone()))
            .ok()
            .unwrap();
        app.tick_chat();
        assert_eq!(app.chat.editor.text, "draft — café");
        assert!(!app.chat.editing);
        key(&mut app, KeyCode::Char('e'));
        app.chat.editor = Editor::new("  corrected — 你好\n".into());
        key(&mut app, KeyCode::Enter);
        match commands.try_recv().unwrap() {
            Command::Submit { turn, text, .. } => {
                assert_eq!(turn, "turn-1");
                assert_eq!(text, "  corrected — 你好\n");
            }
            _ => panic!("Enter must submit the review"),
        }
        key(&mut app, KeyCode::Enter);
        assert!(
            commands.try_recv().is_err(),
            "repeat Enter cannot duplicate a pending submission"
        );
        snapshot.turns[0].status = ChatPhase::Generating;
        snapshot.turns[0].approved_text = Some("  corrected — 你好\n".into());
        snapshot.turns[0].reply = "Hello".into();
        events
            .try_send(Event::Snapshot(snapshot.clone()))
            .ok()
            .unwrap();
        let mut delta = snapshot.clone();
        delta.turns[0].reply = "Hello café".into();
        events
            .try_send(Event::Snapshot(delta.clone()))
            .ok()
            .unwrap();
        delta.turns[0].status = ChatPhase::Completed;
        events.try_send(Event::Snapshot(delta)).ok().unwrap();
        events.try_send(Event::Snapshot(snapshot)).ok().unwrap(); // poll captured before completion
        app.tick_chat();
        assert_eq!(app.chat.snapshot().unwrap().turns[0].reply, "Hello café");
        assert_eq!(
            app.chat.snapshot().unwrap().turns[0].status,
            ChatPhase::Completed
        );
        assert!(app.chat.editor.text.is_empty());
        assert!(app.chat.streaming.is_empty());
    }
    #[test]
    fn snapshots_of_previous_completed_turn_do_not_clear_a_manual_follow_up() {
        let mut app = super::super::tests::test_app();
        app.voice.target = Some("http://127.0.0.1:8080".into());
        app.screen = Screen::Bench;
        let (connection, events, mut commands) = Connection::fixture();
        app.local_runtime = Some(crate::tui::chat_client::tests::agent().0);
        app.chat.connection = Some(connection);
        app.ensure_local_chat();
        let mut snapshot = app.chat.snapshot().unwrap().clone();
        snapshot.turns.push(ChatTurn {
            turn_id: "old".into(),
            status: ChatPhase::Completed,
            transcript: "old".into(),
            approved_text: Some("old".into()),
            reply: "old answer".into(),
            error: None,
            timings: Default::default(),
            source: "typed".into(),
            transcription_run: None,
            reasoning_run: None,
        });
        events
            .try_send(Event::Snapshot(snapshot.clone()))
            .ok()
            .unwrap();
        app.tick_chat();
        key(&mut app, KeyCode::Char('i'));
        app.chat.editor.insert("follow-up — 你好");
        events
            .try_send(Event::Snapshot(snapshot.clone()))
            .ok()
            .unwrap();
        app.tick_chat();
        assert!(app.chat.editing);
        assert_eq!(app.chat.editor.text, "follow-up — 你好");
        key(&mut app, KeyCode::Enter);
        assert!(matches!(commands.try_recv().unwrap(), Command::Text { .. }));
        events.try_send(Event::Snapshot(snapshot)).ok().unwrap();
        app.tick_chat();
        assert!(app.chat.starting);
        assert_eq!(app.chat.confirmed_text.as_deref(), Some("follow-up — 你好"));
        assert_eq!(app.chat.editor.text, "follow-up — 你好");
    }
    #[test]
    fn cancelling_before_admission_does_not_auto_send_the_new_turn() {
        let mut app = super::super::tests::test_app();
        app.voice.target = Some("http://127.0.0.1:8080".into());
        app.screen = Screen::Bench;
        let (connection, events, mut commands) = Connection::fixture();
        app.local_runtime = Some(crate::tui::chat_client::tests::agent().0);
        app.chat.connection = Some(connection);
        app.ensure_local_chat();
        app.chat.editor = Editor::new("typed but cancelled".into());
        key(&mut app, KeyCode::Enter);
        assert!(matches!(commands.try_recv().unwrap(), Command::Text { .. }));
        key(&mut app, KeyCode::Char('x'));
        let mut snapshot = app.chat.snapshot().unwrap().clone();
        snapshot.turns.push(ChatTurn {
            turn_id: "new".into(),
            status: ChatPhase::AwaitingReview,
            transcript: "typed but cancelled".into(),
            approved_text: None,
            reply: String::new(),
            error: None,
            timings: Default::default(),
            source: "typed".into(),
            transcription_run: None,
            reasoning_run: None,
        });
        events.try_send(Event::Snapshot(snapshot)).ok().unwrap();
        app.tick_chat();
        assert!(matches!(commands.try_recv().unwrap(),Command::Cancel{turn,..} if turn == "new"));
        assert!(commands.try_recv().is_err());
        assert!(app.chat.editor.text.is_empty());
        assert!(app.chat.confirmed_text.is_none());
    }
    #[test]
    fn review_preview_keeps_bubbles_draft_source_and_all_footer_actions_visible() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = super::super::tests::test_app();
        app.voice.target = Some("http://127.0.0.1:8080".into());
        app.voice.inspect(Ok(crate::tui::connected::Snapshot {
            history: vec![],
            current_turn: None,
            stt: crate::tui::connected::RuntimeStatus {
                name: "whisper-large-v3-turbo".into(),
                ready: true,
            },
            reply: crate::tui::connected::RuntimeStatus {
                name: "qwen2.5-1.5b-instruct-q4-k-m".into(),
                ready: true,
            },
            role: None,
            busy: false,
        }));
        app.screen = Screen::Bench;
        app.ensure_local_chat();
        let snapshot = app.chat.archives.first_mut().unwrap();
        snapshot.title = "Smoke downstairs and safe evacuation".into();
        snapshot.turns.push(ChatTurn {
            turn_id: "turn-1".into(),
            status: ChatPhase::Completed,
            transcript: "There is smoke downstairs.".into(),
            approved_text: Some("There is smoke downstairs.".into()),
            reply: "Is anyone in immediate danger?".into(),
            error: None,
            timings: Default::default(),
            source: "live microphone".into(),
            transcription_run: None,
            reasoning_run: None,
        });
        snapshot.turns.push(ChatTurn {
            turn_id: "turn-2".into(),
            status: ChatPhase::AwaitingReview,
            transcript: "Everyone has left the building.".into(),
            approved_text: None,
            reply: String::new(),
            error: None,
            timings: Default::default(),
            source: "live microphone".into(),
            transcription_run: Some("stt-conv-example-turn-2".into()),
            reasoning_run: None,
        });
        app.chat.editor = Editor::new("Everyone has left the building.".into());
        app.chat.draft_turn = Some("review".into());
        for (width, height) in [(80, 24), (120, 36)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| app.draw_chat(frame)).unwrap();
            let buffer = terminal.backend().buffer();
            let mut rows = Vec::new();
            for y in 0..height {
                let mut x = 0;
                let mut row = String::new();
                while x < width {
                    let symbol = buffer[(x, y)].symbol();
                    row.push_str(symbol);
                    x += UnicodeWidthStr::width(symbol).max(1) as u16;
                }
                rows.push(row.trim_end().to_owned());
            }
            let text = rows.join("\n");
            for expected in [
                "SOURCE",
                "CHAT /",
                "EDIT YOUR MESSAGE",
                "Everyone has left the building.",
                "[Enter] Confirm & send",
                "[e] Edit",
                "[x] Discard",
                "[b] Chat",
                "[q] Quit",
                "RAM",
                "Power",
                "Is anyone in immediate danger?",
            ] {
                assert!(
                    text.contains(expected),
                    "missing {expected} at {width}x{height}: {text}"
                );
            }
            if let Ok(directory) = std::env::var("PHEME_VA_UI_PREVIEW_DIR") {
                std::fs::create_dir_all(&directory).unwrap();
                std::fs::write(
                    std::path::Path::new(&directory).join(format!("chat-{width}x{height}.txt")),
                    text,
                )
                .unwrap();
            }
        }
    }
}
