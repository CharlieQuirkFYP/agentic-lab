//! Typed inference events. Hosts choose their own serialization and presentation.
use crate::voice::{Failure, Turn};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::mpsc;
use va_core::chat::ChatPhase;

#[derive(Clone)]
pub enum EventKind {
    TurnCreated { recovered: bool },
    TranscriptReady { text: String },
    QuestionApproved { text: String },
    ReplyStarted,
    ReplyDelta { text: String },
    ReplyCompleted { text: String },
    TurnFailed { error: Option<Failure> },
    TurnCancelled,
}
#[derive(Clone)]
pub struct RuntimeEvent {
    pub turn_id: Option<String>,
    pub kind: EventKind,
}
impl RuntimeEvent {
    pub fn new(turn_id: Option<String>, kind: EventKind) -> Self {
        Self { turn_id, kind }
    }
}
pub struct TurnStream {
    pub turn: Turn,
    pub receiver: mpsc::Receiver<RuntimeEvent>,
    /// Isolated tests cancel when their consumer leaves; conversations retain state.
    pub cancel_on_drop: Option<Arc<AtomicBool>>,
}
impl TurnStream {
    pub(crate) fn recovered(turn: Turn) -> Self {
        let (sender, receiver) = mpsc::channel(64);
        let emit = |kind| {
            let _ = sender.try_send(RuntimeEvent::new(Some(turn.turn_id.clone()), kind));
        };
        emit(EventKind::TurnCreated { recovered: true });
        if turn.status != ChatPhase::Transcribing {
            emit(EventKind::TranscriptReady {
                text: turn.transcript.clone(),
            });
        }
        if let Some(text) = &turn.approved_text {
            emit(EventKind::QuestionApproved { text: text.clone() });
            emit(EventKind::ReplyStarted);
        }
        match turn.status {
            ChatPhase::Completed => emit(EventKind::ReplyCompleted {
                text: turn.reply.clone(),
            }),
            ChatPhase::Failed => emit(EventKind::TurnFailed {
                error: turn.error.clone(),
            }),
            ChatPhase::Cancelled => emit(EventKind::TurnCancelled),
            _ => {}
        }
        Self {
            turn,
            receiver,
            cancel_on_drop: None,
        }
    }
}
