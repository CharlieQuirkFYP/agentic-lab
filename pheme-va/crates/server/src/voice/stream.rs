use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_core::Stream;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::mpsc;
use va_runtime::events::{EventKind, RuntimeEvent, TurnStream};
struct Events {
    receiver: mpsc::Receiver<RuntimeEvent>,
    cancel_on_drop: Option<Arc<AtomicBool>>,
    status: va_core::chat::ChatPhase,
}
impl Drop for Events {
    fn drop(&mut self) {
        if let Some(flag) = &self.cancel_on_drop {
            flag.store(true, Ordering::Release);
        }
    }
}
impl Stream for Events {
    type Item = Result<Event, Infallible>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let status = self.status;
        self.receiver.poll_recv(cx).map(|event| {
            event.map(|event| {
                let (name, mut data): (&str, Value) = match event.kind {
                    EventKind::TurnCreated { recovered } => (
                        "turn.created",
                        if recovered {
                            json!({"status":status,"recovered":true})
                        } else {
                            json!({})
                        },
                    ),
                    EventKind::TranscriptReady { text } => {
                        ("transcript.ready", json!({"text":text}))
                    }
                    EventKind::QuestionApproved { text } => {
                        ("question.approved", json!({"text":text}))
                    }
                    EventKind::ReplyStarted => ("reply.started", json!({})),
                    EventKind::ReplyDelta { text } => ("reply.delta", json!({"text":text})),
                    EventKind::ReplyCompleted { text } => ("reply.completed", json!({"text":text})),
                    EventKind::TurnFailed { error } => ("turn.failed", json!({"error":error})),
                    EventKind::TurnCancelled => ("turn.cancelled", json!({})),
                };
                if let Some(id) = event.turn_id {
                    data["turn_id"] = json!(id);
                }
                Ok(Event::default().event(name).data(data.to_string()))
            })
        })
    }
}
pub fn response(stream: TurnStream) -> Response {
    Sse::new(Events {
        receiver: stream.receiver,
        cancel_on_drop: stream.cancel_on_drop,
        status: stream.turn.status,
    })
    .keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("heartbeat"),
    )
    .into_response()
}
