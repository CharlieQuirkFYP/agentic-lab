use std::time::Instant;

use super::connected::{self, Connection, Event, Snapshot, Transcript, TurnStatus};
use super::editor::Editor;

pub struct WebEditor {
    pub turn_id: String,
    pub original: String,
    pub buffer: Editor,
    pub pending: bool,
    pub accepted: bool,
    pub error: Option<String>,
}

pub struct TestState {
    pub buffer: Editor,
    pub editing: bool,
    pub diagnostics: bool,
    pub source: String,
    pub transcript: Option<Transcript>,
    pub reply: String,
    pub stage: &'static str,
    pub request: Option<u64>,
    pub next_request: u64,
    pub error: Option<String>,
    pub auto_voice: bool,
    pub started: Option<Instant>,
    pub elapsed_ms: Option<u128>,
}

impl Default for TestState {
    fn default() -> Self {
        Self {
            buffer: Editor::default(),
            editing: false,
            diagnostics: false,
            source: "typed text".into(),
            transcript: None,
            reply: String::new(),
            stage: "review",
            request: None,
            next_request: 1,
            error: None,
            auto_voice: false,
            started: None,
            elapsed_ms: None,
        }
    }
}

impl TestState {
    pub fn begin(&mut self, stage: &'static str) -> u64 {
        self.diagnostics = false;
        let request = self.next_request;
        self.next_request = self.next_request.wrapping_add(1);
        self.request = Some(request);
        self.stage = stage;
        self.error = None;
        self.reply.clear();
        self.started = Some(Instant::now());
        self.elapsed_ms = None;
        request
    }

    pub fn finish(&mut self, stage: &'static str) {
        self.request = None;
        self.stage = stage;
        self.elapsed_ms = self.started.take().map(|time| time.elapsed().as_millis());
    }
}

#[derive(Default)]
pub struct VoiceState {
    pub target: Option<String>,
    pub connection: Option<Connection>,
    pub online: bool,
    pub connection_error: Option<String>,
    pub snapshot: Option<Snapshot>,
    pub editor: Option<WebEditor>,
    pub discard_editor: bool,
    pub tests: TestState,
}

impl VoiceState {
    pub fn new(target: Option<String>) -> Self {
        Self {
            target,
            ..Self::default()
        }
    }

    pub fn connected_mode(&self) -> bool {
        self.target.is_some()
    }

    pub fn start(&mut self) {
        if self.connection.is_some() {
            return;
        }
        if let Some(target) = self.target.as_deref() {
            match Connection::start(target) {
                Ok(connection) => self.connection = Some(connection),
                Err(error) => self.connection_error = Some(error.to_string()),
            }
        }
    }

    pub fn inspect(&mut self, result: Result<Snapshot, String>) {
        match result {
            Ok(snapshot) => {
                if let Some(editor) = self.editor.as_mut() {
                    if let Some(turn) = snapshot
                        .current_turn
                        .as_ref()
                        .filter(|turn| turn.turn_id == editor.turn_id)
                    {
                        if let Some(approved) = turn.approved_text.as_ref() {
                            // Server-side approval wins, but never retarget a stale editor.
                            editor.buffer = Editor::new(approved.clone());
                            editor.accepted = true;
                            editor.pending = false;
                        }
                    }
                }
                self.snapshot = Some(snapshot);
                self.online = true;
                self.connection_error = None;
            }
            Err(error) => {
                self.online = false;
                self.connection_error = Some(error);
            }
        }
    }

    pub fn open_editor(&mut self) -> Result<(), String> {
        let turn = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.current_turn.as_ref())
            .filter(|turn| matches!(turn.status, TurnStatus::AwaitingReview))
            .ok_or("no web turn is awaiting review")?;
        if self
            .editor
            .as_ref()
            .is_some_and(|editor| editor.turn_id == turn.turn_id)
        {
            return Ok(());
        }
        if self
            .editor
            .as_ref()
            .is_some_and(|editor| editor.pending || (editor.buffer.dirty && !editor.accepted))
        {
            return Err(
                "unsaved editor belongs to another turn; Esc then x discards it explicitly".into(),
            );
        }
        if turn.transcript.len() > super::editor::MAX_INPUT_BYTES {
            return Err(
                "web transcript exceeds editor input limit; refusing to silently truncate it"
                    .into(),
            );
        }
        self.editor = Some(WebEditor {
            turn_id: turn.turn_id.clone(),
            original: turn.transcript.clone(),
            buffer: Editor::new(turn.transcript.clone()),
            pending: false,
            accepted: false,
            error: None,
        });
        Ok(())
    }

    pub fn submission(&self) -> Result<(String, String), String> {
        let editor = self.editor.as_ref().ok_or("no editor")?;
        if editor.accepted || editor.pending {
            return Err("question is already approved or submission is pending".into());
        }
        if !self.online {
            return Err("inspection is disconnected; reconnect before submitting".into());
        }
        let snapshot = self.snapshot.as_ref().ok_or("inspection unavailable")?;
        if !snapshot.reply.ready || snapshot.busy {
            return Err("server reply runtime is unavailable or busy".into());
        }
        let turn = snapshot
            .current_turn
            .as_ref()
            .ok_or("web turn is no longer retained")?;
        if turn.turn_id != editor.turn_id || !matches!(turn.status, TurnStatus::AwaitingReview) {
            return Err(
                "stale editor: this turn is no longer awaiting review; text retained locally"
                    .into(),
            );
        }
        connected::validate_text(&editor.buffer.text).map_err(|error| error.to_string())?;
        Ok((editor.turn_id.clone(), editor.buffer.text.clone()))
    }

    /// Returns text eligible for optional local test speech. Inspection never does.
    pub fn event(&mut self, event: Event) -> Option<String> {
        match event {
            Event::Submitted {
                turn_id,
                text,
                result,
            } => {
                if let Some(editor) = self
                    .editor
                    .as_mut()
                    .filter(|editor| editor.turn_id == turn_id)
                {
                    editor.pending = false;
                    match result {
                        Ok(()) => {
                            if !editor.accepted {
                                editor.buffer = Editor::new(text);
                            }
                            editor.accepted = true;
                            editor.error = None;
                        }
                        Err(error) => editor.error = Some(error),
                    }
                }
            }
            Event::Transcript { request, result } if self.tests.request == Some(request) => {
                self.tests.finish("review");
                match result {
                    Ok(transcript) => {
                        self.tests.error = None;
                        self.tests.buffer = Editor::new(transcript.text.clone());
                        if !matches!(transcript.status.as_str(), "Speech" | "speech") {
                            self.tests.error = Some(format!(
                                "STT gate: {} — no reply was started",
                                transcript.status
                            ));
                        }
                        self.tests.transcript = Some(transcript);
                    }
                    Err(error) => {
                        self.tests.stage = "failed";
                        self.tests.error = Some(error);
                    }
                }
            }
            Event::ReplyStarted { request } if self.tests.request == Some(request) => {
                self.tests.stage = "generating"
            }
            Event::ReplyDelta { request, text } if self.tests.request == Some(request) => {
                self.tests.reply.push_str(&text)
            }
            Event::ReplyCompleted { request, text } if self.tests.request == Some(request) => {
                self.tests.reply = text.clone();
                self.tests.error = None;
                self.tests.finish("completed");
                return self.tests.auto_voice.then_some(text);
            }
            Event::TestFailed { request, error } if self.tests.request == Some(request) => {
                self.tests.finish("failed");
                self.tests.error = Some(error);
            }
            _ => {}
        }
        None
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub fn snapshot(id: &str, reply: &str) -> Snapshot {
        serde_json::from_value(serde_json::json!({
            "history": [{"role":"user", "content":"earlier"}],
            "current_turn": {"turn_id":id,"status":"awaiting_review","transcript":"original",
                "approved_text":null,"reply":reply,"error":null,"timings":{}},
            "stt":{"name":"stt","ready":true}, "reply":{"name":"reply","ready":true},
            "role":null,"busy":false
        }))
        .unwrap()
    }

    #[test]
    fn dirty_editor_is_not_clobbered_or_retargeted_by_polling() {
        let mut state = VoiceState::default();
        state.inspect(Ok(snapshot("one", "a")));
        state.open_editor().unwrap();
        state.editor.as_mut().unwrap().buffer.insert(" corrected");
        state.inspect(Ok(snapshot("one", "ab")));
        assert_eq!(
            state.editor.as_ref().unwrap().buffer.text,
            "original corrected"
        );
        assert_eq!(
            state
                .snapshot
                .as_ref()
                .unwrap()
                .current_turn
                .as_ref()
                .unwrap()
                .reply,
            "ab"
        );
        assert_eq!(
            state.submission().unwrap(),
            ("one".into(), "original corrected".into())
        );
        state.inspect(Ok(snapshot("two", "")));
        assert!(state.submission().unwrap_err().contains("stale"));
        assert!(state.open_editor().is_err());
        state.event(Event::Submitted {
            turn_id: "one".into(),
            text: "original corrected".into(),
            result: Err("conflict".into()),
        });
        assert_eq!(
            state.editor.as_ref().unwrap().buffer.text,
            "original corrected"
        );
    }

    #[test]
    fn tests_never_modify_web_snapshot_and_stale_results_are_ignored() {
        let mut state = VoiceState::default();
        state.inspect(Ok(snapshot("web", "web reply")));
        let first = state.tests.begin("generating");
        let current = state.tests.begin("generating");
        state.event(Event::ReplyDelta {
            request: first,
            text: "stale".into(),
        });
        state.event(Event::ReplyDelta {
            request: current,
            text: "partial".into(),
        });
        assert_eq!(state.tests.reply, "partial");
        assert!(state
            .event(Event::ReplyCompleted {
                request: current,
                text: "final".into()
            })
            .is_none());
        assert_eq!(state.tests.reply, "final");
        assert_eq!(
            state
                .snapshot
                .as_ref()
                .unwrap()
                .current_turn
                .as_ref()
                .unwrap()
                .reply,
            "web reply"
        );
        assert_eq!(state.snapshot.as_ref().unwrap().history.len(), 1);
    }

    #[test]
    fn approval_freezes_authoritative_wording_and_readiness_only_gates_submit() {
        let mut state = VoiceState::default();
        let mut snap = snapshot("web", "");
        snap.reply.ready = false;
        state.inspect(Ok(snap.clone()));
        state.open_editor().unwrap();
        assert!(state.submission().is_err());
        snap.current_turn.as_mut().unwrap().approved_text = Some("web approved".into());
        snap.current_turn.as_mut().unwrap().status = TurnStatus::Generating;
        state.inspect(Ok(snap));
        assert!(state.editor.as_ref().unwrap().accepted);
        assert_eq!(state.editor.as_ref().unwrap().buffer.text, "web approved");
        assert!(state.submission().is_err());
    }
}
