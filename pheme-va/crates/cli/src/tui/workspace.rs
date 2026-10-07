use anyhow::{Context, Result};
use ratatui::crossterm::event::KeyCode;

use super::{App, Screen};
use crate::model::ModelPurpose;
use crate::recorder::Recording;
use crate::tui::config;
use crate::tui::connected::{AudioInput, Command, TurnStatus};
use crate::tui::download::DownloadTask;
use crate::tui::model_actions::{self, Confirmation, Verification};
use crate::tui::playback::Playback;

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;

impl App {
    pub fn tick_workspace(&mut self) {
        for index in (0..self.download_reapers.len()).rev() {
            if self.download_reapers[index].is_finished() {
                let _ = self.download_reapers.swap_remove(index).join();
            }
        }
        let mut events = Vec::new();
        let inspection = if let Some(connection) = self.voice.connection.as_mut() {
            let inspection = connection.inspection();
            for _ in 0..64 {
                let Some(event) = connection.event() else {
                    break;
                };
                events.push(event);
            }
            inspection
        } else {
            None
        };
        if let Some(inspection) = inspection {
            self.voice.inspect(inspection);
        }
        for event in events {
            if let Some(text) = self.voice.event(event) {
                self.speak(&text);
            }
        }
        if let Some(playback) = self.playback.as_mut() {
            match playback.poll() {
                Ok(true) => {
                    self.playback.take();
                    self.status_message = "local speech completed".into();
                }
                Ok(false) => {}
                Err(error) => {
                    self.error_message = Some(error.to_string());
                    self.logs.warn("local-tts", error.to_string());
                    self.playback.take();
                }
            }
        }
        let verified = self.models.verification.as_mut().and_then(|verification| {
            verification.receiver.try_recv().ok().map(|result| {
                (
                    verification.model_id.clone(),
                    verification.entry.clone(),
                    result,
                )
            })
        });
        if let Some((id, entry, result)) = verified {
            self.models.verification.take();
            match result {
                Ok(()) => {
                    self.models.verified_choice = Some(entry);
                    self.models.scroll = 0;
                    self.models.confirmation = Some(Confirmation::Choose(id));
                    self.logs.info("local-model-files", "artifact checksums and role verified; awaiting startup-choice confirmation");
                }
                Err(error) => self.error_message = Some(error),
            }
        }
    }

    pub fn shutdown_workspace(&mut self) {
        self.voice.connection.take();
        self.playback.take();
        if let Some(mut download) = self.download.take() {
            if let Some(reaper) = download.cancel() {
                self.download_reapers.push(reaper);
            }
        }
        for reaper in self.download_reapers.drain(..) {
            let _ = reaper.join();
        }
        self.models.verification.take();
        if let Some(recording) = self.recording.take() {
            recording.discard();
        }
    }

    pub fn paste_workspace(&mut self, text: &str) {
        match self.screen {
            Screen::WebEditor => {
                if let Some(editor) = self
                    .voice
                    .editor
                    .as_mut()
                    .filter(|editor| !editor.accepted && !editor.pending)
                {
                    editor.buffer.insert(text);
                }
            }
            Screen::ServerTests if self.voice.tests.editing => self.voice.tests.buffer.insert(text),
            Screen::Models if self.models.role_selection.is_some() => {
                if let Some(roles) = self
                    .models
                    .role_selection
                    .as_mut()
                    .filter(|roles| roles.adding_path)
                {
                    for ch in text.chars() {
                        if roles.path_input.len() + ch.len_utf8() > 4096 {
                            break;
                        }
                        roles.path_input.push(ch);
                    }
                }
            }
            _ => {}
        }
    }

    /// Focused text/confirmation input is handled before global shortcuts.
    pub fn workspace_key(&mut self, code: KeyCode) -> Result<bool> {
        if code == KeyCode::F(8) {
            self.playback.take();
            self.status_message = "local speech stopped".into();
            return Ok(true);
        }
        if self.filter_editing
            || self.screen == Screen::DirectoryInput
            || self.clear_runs_pending
            || self.clear_runs_inflight
        {
            return Ok(false);
        }
        if self.screen == Screen::WebEditor {
            match code {
                KeyCode::Esc => self.navigate_back(),
                KeyCode::F(6) => self.submit_web(),
                _ => {
                    if let Some(editor) = self
                        .voice
                        .editor
                        .as_mut()
                        .filter(|editor| !editor.accepted && !editor.pending)
                    {
                        editor.buffer.key(code);
                    }
                }
            }
            return Ok(true);
        }
        if self.screen == Screen::ServerTests && self.voice.tests.editing {
            match code {
                KeyCode::Esc => self.voice.tests.editing = false,
                KeyCode::F(6) => self.run_reply_test(),
                _ => self.voice.tests.buffer.key(code),
            }
            return Ok(true);
        }
        if self.screen == Screen::Models && self.models.role_selection.is_some() {
            let saving = code == KeyCode::Enter
                && self
                    .models
                    .role_selection
                    .as_ref()
                    .is_some_and(|roles| !roles.adding_path);
            match self.models.handle_roles_key(code, &mut self.config) {
                Ok(()) if saving && self.models.role_selection.is_none() => {
                    self.error_message = None;
                    self.status_message =
                        "reply roles saved locally; restart required, running server unchanged"
                            .into();
                    self.logs.info("startup-choice", &self.status_message);
                }
                Ok(()) => {}
                Err(error) => self.error_message = Some(format!("{error:#}")),
            }
            return Ok(true);
        }
        if self.screen == Screen::Models && self.models.filtering {
            match code {
                KeyCode::Esc | KeyCode::Enter => self.models.filtering = false,
                KeyCode::Char(ch) => {
                    if self.models.group().filter.len() < 256 {
                        self.models.group_mut().filter.push(ch);
                    }
                }
                KeyCode::Backspace => {
                    self.models.group_mut().filter.pop();
                }
                _ => {}
            }
            self.sync_model_selection();
            self.models.scroll = 0;
            return Ok(true);
        }
        if self.screen == Screen::Web && self.voice.discard_editor {
            match code {
                KeyCode::Char('y') => {
                    self.voice.editor.take();
                    self.voice.discard_editor = false;
                }
                KeyCode::Esc => self.voice.discard_editor = false,
                _ => {}
            }
            return Ok(true);
        }
        if self.screen == Screen::Models && self.models.confirmation.is_some() {
            self.confirm_model_action(code);
            return Ok(true);
        }
        if self.screen == Screen::Models && (self.models.preview.is_some() || self.models.command) {
            match code {
                KeyCode::Esc => {
                    self.models.preview.take();
                    self.models.command = false;
                    self.models.scroll = 0;
                }
                KeyCode::Down | KeyCode::PageDown => {
                    self.models.scroll = self.models.scroll.saturating_add(3)
                }
                KeyCode::Up | KeyCode::PageUp => {
                    self.models.scroll = self.models.scroll.saturating_sub(3)
                }
                _ => {}
            }
            return Ok(true);
        }
        if matches!(code, KeyCode::Char('w' | 'm' | 't' | 'b')) && self.screen != Screen::Help {
            // Recording owns stop/discard keys; navigation must not hide it.
            if self.recording.is_some() || self.screen == Screen::Recording {
                return Ok(true);
            }
            let screen = match code {
                KeyCode::Char('w') => Screen::Web,
                KeyCode::Char('m') => Screen::Models,
                KeyCode::Char('t') => Screen::Telemetry,
                KeyCode::Char('b') => self.tests_screen(),
                _ => unreachable!(),
            };
            if screen == self.screen {
                return Ok(true);
            }
            self.error_message = None;
            if screen == Screen::Telemetry {
                self.open_telemetry();
            } else {
                self.navigate_to(screen);
                if screen == Screen::Models {
                    self.sync_model_selection();
                }
            }
            return Ok(true);
        }
        if self.screen == Screen::Telemetry {
            return Ok(false);
        }
        if code == KeyCode::F(7)
            && self.download.is_some()
            && matches!(self.screen, Screen::Models | Screen::Loading)
        {
            let mut task = self.download.take().unwrap();
            if let Some(reaper) = task.cancel() {
                self.download_reapers.push(reaper);
            }
            self.models.download_status.insert(
                task.model_id.clone(),
                "Cancelled (partial files retained)".into(),
            );
            drop(task);
            self.logs.info(
                "local-model-files",
                "local download cancelled; partial files retained for explicit retry",
            );
            self.status_message = "local download cancelled".into();
            return Ok(true);
        }
        match self.screen {
            Screen::Web => {
                match code {
                    KeyCode::Char('e') => match self.voice.open_editor() {
                        Ok(()) => {
                            self.navigate_to(Screen::WebEditor);
                            self.error_message = None;
                        }
                        Err(error) => self.error_message = Some(error),
                    },
                    KeyCode::Char('x') if self.voice.editor.is_some() => {
                        self.voice.discard_editor = true
                    }
                    KeyCode::Char('p') => {
                        let reply = self
                            .voice
                            .snapshot
                            .as_ref()
                            .and_then(|snapshot| snapshot.current_turn.as_ref())
                            .filter(|turn| matches!(turn.status, TurnStatus::Completed))
                            .map(|turn| turn.reply.clone());
                        if let Some(reply) = reply {
                            self.speak(&reply);
                        }
                    }
                    KeyCode::Char('r') => self.voice.start(),
                    KeyCode::Esc => self.navigate_back(),
                    KeyCode::Down | KeyCode::PageDown => {
                        self.scroll = self.scroll.saturating_add(3)
                    }
                    KeyCode::Up | KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(3),
                    KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Char('t') => {
                        return Ok(false)
                    }
                    _ => {}
                }
                Ok(true)
            }
            Screen::ServerTests => {
                match code {
                    KeyCode::Char('i') | KeyCode::Char('e')
                        if self.voice.tests.request.is_none() =>
                    {
                        self.voice.tests.editing = true;
                        self.voice.tests.diagnostics = false;
                        if code == KeyCode::Char('i') {
                            self.voice.tests.source = "typed text".into();
                            self.voice.tests.transcript = None;
                        }
                    }
                    KeyCode::Char('l') => self.start_server_recording(),
                    KeyCode::Char('f') if self.voice.tests.request.is_none() => {
                        self.folder.refresh();
                        self.navigate_to(Screen::Folder);
                    }
                    KeyCode::F(6) | KeyCode::Char('r') => self.run_reply_test(),
                    KeyCode::F(7) if self.voice.tests.request.is_some() => {
                        if let Some(connection) = self.voice.connection.as_ref() {
                            let _ = connection.send(Command::CancelTest);
                        }
                        self.voice.tests.finish("cancelled");
                        self.logs.info(
                            "isolated-test",
                            "test stream cancelled locally; web turn untouched",
                        );
                    }
                    KeyCode::Char('p') if self.voice.tests.stage == "completed" => {
                        self.speak(&self.voice.tests.reply.clone())
                    }
                    KeyCode::Char('d') => {
                        self.voice.tests.diagnostics = !self.voice.tests.diagnostics;
                        self.scroll = 0;
                    }
                    KeyCode::Char('v') => {
                        self.voice.tests.auto_voice = !self.voice.tests.auto_voice
                    }
                    KeyCode::Esc if self.voice.tests.diagnostics => {
                        self.voice.tests.diagnostics = false;
                        self.scroll = 0;
                    }
                    KeyCode::Esc => self.navigate_back(),

                    KeyCode::Down | KeyCode::PageDown => {
                        self.scroll = self.scroll.saturating_add(3)
                    }
                    KeyCode::Up | KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(3),
                    KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Char('t') => {
                        return Ok(false)
                    }
                    _ => {}
                }
                Ok(true)
            }
            Screen::Models => {
                self.model_key(code);
                Ok(!matches!(
                    code,
                    KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Char('t')
                ))
            }
            Screen::Recording if self.voice.connected_mode() => {
                match code {
                    KeyCode::Enter | KeyCode::Char('l') | KeyCode::Char(' ') => {
                        self.stop_recording()
                    }
                    KeyCode::Esc => {
                        if let Some(recording) = self.recording.take() {
                            recording.discard();
                        }
                        self.navigate_back();
                    }
                    KeyCode::Char('q') => return Ok(false),
                    _ => {}
                }
                Ok(true)
            }

            _ => Ok(false),
        }
    }

    fn submit_web(&mut self) {
        let result = self.voice.submission().and_then(|(turn_id, text)| {
            self.voice
                .connection
                .as_ref()
                .ok_or("connection unavailable".to_owned())?
                .send(Command::Submit { turn_id, text })
                .map_err(|error| error.to_string())
        });
        if let Some(editor) = self.voice.editor.as_mut() {
            match result {
                Ok(()) => {
                    editor.pending = true;
                    editor.error = None;
                }
                Err(error) => editor.error = Some(error),
            }
        }
    }

    fn test_ready(&self, reply: bool) -> Result<()> {
        anyhow::ensure!(
            self.voice.tests.request.is_none(),
            "an isolated test is already running"
        );
        anyhow::ensure!(
            self.voice.online,
            "server inspection unavailable; no local inference fallback"
        );
        let snapshot = self
            .voice
            .snapshot
            .as_ref()
            .context("no server inspection")?;
        anyhow::ensure!(!snapshot.busy, "server is busy");
        anyhow::ensure!(
            if reply {
                snapshot.reply.ready
            } else {
                snapshot.stt.ready
            },
            "server runtime not ready; next-start choices do not change active runtimes"
        );
        Ok(())
    }

    fn run_reply_test(&mut self) {
        let result = (|| -> Result<()> {
            self.test_ready(true)?;
            crate::tui::connected::validate_text(&self.voice.tests.buffer.text)?;
            let text = self.voice.tests.buffer.text.clone();
            let request = self.voice.tests.begin("generating");
            let sent = self
                .voice
                .connection
                .as_ref()
                .context("connection unavailable")
                .and_then(|connection| connection.send(Command::Reply { request, text }));
            if let Err(error) = sent {
                self.voice.tests.finish("failed");
                return Err(error);
            }
            self.voice.tests.editing = false;
            self.logs.info(
                "isolated-test",
                format!("reply test {request} started; no web context"),
            );
            Ok(())
        })();
        if let Err(error) = result {
            self.voice.tests.error = Some(error.to_string());
        }
    }

    pub fn start_server_audio(&mut self, audio: AudioInput, source: String) {
        let result = (|| -> Result<()> {
            self.test_ready(false)?;
            self.playback.take();
            self.voice.tests.source = source;
            let request = self.voice.tests.begin("transcribing");
            let sent = self
                .voice
                .connection
                .as_ref()
                .context("connection unavailable")
                .and_then(|connection| {
                    connection.send(Command::Transcribe {
                        request,
                        audio,
                        max_seconds: self.config.max_seconds.min(120),
                    })
                });
            if let Err(error) = sent {
                self.voice.tests.finish("failed");
                return Err(error);
            }
            self.voice.tests.editing = false;
            self.logs.info(
                "isolated-test",
                format!("stateless STT test {request} started"),
            );
            Ok(())
        })();
        if let Err(error) = result {
            self.voice.tests.error = Some(error.to_string());
        }
        self.screen = Screen::ServerTests;
    }

    pub fn start_server_recording(&mut self) {
        let result = (|| -> Result<()> {
            self.test_ready(false)?;
            anyhow::ensure!(self.recording.is_none(), "microphone already recording");
            self.playback.take();
            self.recording = Some(Recording::start()?);
            self.navigate_to(Screen::Recording);
            self.error_message = None;
            Ok(())
        })();
        if let Err(error) = result {
            self.voice.tests.error = Some(error.to_string());
        }
    }

    fn speak(&mut self, text: &str) {
        self.playback.take();
        if self.recording.is_some() {
            self.error_message = Some("stop recording before playback".into());
            return;
        }
        match Playback::start(text) {
            Ok(playback) => {
                self.playback = Some(playback);
                self.status_message = "local espeak playback (F8 Stop)".into();
            }
            Err(error) => self.error_message = Some(error.to_string()),
        }
    }

    fn model_key(&mut self, code: KeyCode) {
        let result = (|| -> Result<()> {
            match code {
                KeyCode::Up | KeyCode::Char('k') => self.models.move_selection(&self.catalog, -1),
                KeyCode::Down | KeyCode::Char('j') => self.models.move_selection(&self.catalog, 1),
                KeyCode::PageUp => self.models.scroll = self.models.scroll.saturating_sub(5),
                KeyCode::PageDown => self.models.scroll = self.models.scroll.saturating_add(5),
                KeyCode::Enter => {
                    let entry = self
                        .models
                        .selected(&self.catalog)
                        .context("no selected model")?;
                    if !entry.artifacts_available() {
                        self.start_model_download(entry.manifest.id.clone());
                    } else if entry.manifest.purpose() == Some(ModelPurpose::Reply)
                        || self.voice.connected_mode()
                    {
                        self.status_message = "available local artifacts; use s to verify a next-start choice, running server unchanged".into();
                    } else {
                        anyhow::ensure!(self.current_request.is_none() && self.build.is_none(), "standalone STT is still busy; await completion before selecting a model");
                        self.sync_model_selection();
                        self.choose_catalog_model();
                    }
                    self.models.scroll = 0;
                }
                KeyCode::Char('/') => self.models.filtering = true,
                KeyCode::Char('p') => self.models.preview(&self.catalog, &self.config)?,
                KeyCode::Char('o') => self.models.open_roles(&self.catalog, &self.config)?,
                KeyCode::Char('g') => {
                    self.models.command = true;
                    self.models.scroll = 0;
                }
                KeyCode::Char('r') => {
                    self.catalog.refresh();
                    self.sync_model_selection();
                    if let Some(download) = self.download.as_ref() {
                        self.models
                            .download_status
                            .retain(|id, _| id == &download.model_id);
                    } else {
                        self.models.download_status.clear();
                    }
                    self.start_cache_probe();
                    self.error_message = None;
                    self.logs.info("manifest", "rescanned model manifest");
                }
                KeyCode::Char('d') => {
                    let entry = self
                        .models
                        .selected(&self.catalog)
                        .context("no selected model")?;
                    self.start_model_download(entry.manifest.id.clone());
                }
                KeyCode::Char('s') => {
                    anyhow::ensure!(
                        self.models.verification.is_none(),
                        "startup-choice verification is already running"
                    );
                    let entry = self
                        .models
                        .selected(&self.catalog)
                        .context("no selected model")?;
                    model_actions::next_start_config(&self.config, &self.catalog, &entry.manifest)?;
                    self.models.verification =
                        Some(Verification::start(entry, &self.catalog.manifest_path)?);
                    self.status_message =
                        "verifying startup choice in background; server unchanged".into();
                }
                KeyCode::Esc => self.navigate_back(),
                _ => {}
            }
            Ok(())
        })();
        self.sync_model_selection();
        if let Err(error) = result {
            self.error_message = Some(error.to_string());
        }
    }

    fn confirm_model_action(&mut self, code: KeyCode) {
        match code {
            KeyCode::Down | KeyCode::PageDown => {
                self.models.scroll = self.models.scroll.saturating_add(3);
                return;
            }
            KeyCode::Up | KeyCode::PageUp => {
                self.models.scroll = self.models.scroll.saturating_sub(3);
                return;
            }
            _ => {}
        }
        if code == KeyCode::Esc {
            self.models.confirmation.take();
            self.models.verified_choice.take();
            return;
        }
        if code != KeyCode::Char('y') {
            return;
        }
        let confirmation = self.models.confirmation.take().unwrap();
        let result = (|| -> Result<()> {
            match confirmation {
                Confirmation::Download(id) => {
                    anyhow::ensure!(self.download.is_none(), "a download is already running");
                    let entry = self
                        .catalog
                        .entry_by_id(&id)
                        .context("model no longer in catalog")?;
                    let root = self
                        .catalog
                        .manifest_path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."));
                    self.download = Some(DownloadTask::start(&entry.manifest, root)?);

                    self.models
                        .download_status
                        .insert(id.clone(), "Downloading".into());
                    self.logs.info(
                        "local-model-files",
                        format!("direct script download started: {id}; active server unchanged"),
                    );
                }
                Confirmation::Choose(id) => {
                    let verified = self
                        .models
                        .verified_choice
                        .take()
                        .context("choice has not been verified")?;
                    let entry = self
                        .catalog
                        .entry_by_id(&id)
                        .context("model no longer in catalog")?;
                    anyhow::ensure!(
                        entry.artifacts_available()
                            && serde_json::to_value(&entry.manifest)?
                                == serde_json::to_value(&verified)?,
                        "catalog changed after verification; verify again"
                    );
                    let next = model_actions::next_start_config(
                        &self.config,
                        &self.catalog,
                        &entry.manifest,
                    )?;
                    config::save(&next)?;
                    self.config = next;
                    self.models.command = true;
                    self.logs.info("startup-choice", format!("saved next-start choice {id}; no activation or HTTP management request"));
                    self.status_message =
                        "startup choice saved locally; restart required, running server unchanged"
                            .into();
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.error_message = Some(error.to_string());
        }
    }
}
