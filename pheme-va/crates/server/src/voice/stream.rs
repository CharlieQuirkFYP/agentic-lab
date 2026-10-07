use std::convert::Infallible;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_core::Stream;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::{Status, Turn, EVENT_CAPACITY};

pub struct Frame {
    pub name: &'static str,
    pub data: Value,
}
struct Events {
    receiver: mpsc::Receiver<Frame>,
    cancel_on_drop: Option<Arc<AtomicBool>>,
}
impl Drop for Events {
    fn drop(&mut self) {
        if let Some(cancelled) = &self.cancel_on_drop {
            cancelled.store(true, Ordering::Release);
        }
    }
}
impl Stream for Events {
    type Item = Result<Event, Infallible>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx).map(|frame| {
            frame.map(|frame| {
                Ok(Event::default()
                    .event(frame.name)
                    .data(frame.data.to_string()))
            })
        })
    }
}

pub fn response(
    receiver: mpsc::Receiver<Frame>,
    cancel_on_drop: Option<Arc<AtomicBool>>,
) -> Response {
    Sse::new(Events {
        receiver,
        cancel_on_drop,
    })
    .keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("heartbeat"),
    )
    .into_response()
}

/// Identical POST retries acknowledge retained state without attaching a second
/// subscriber or starting inference. Ongoing work is recovered with GET.
pub fn snapshot(turn: &Turn) -> Response {
    let (sender, receiver) = mpsc::channel(EVENT_CAPACITY);
    let event = |name, mut data: Value| {
        data["turn_id"] = json!(turn.turn_id);
        let _ = sender.try_send(Frame { name, data });
    };
    event(
        "turn.created",
        json!({"status": turn.status, "recovered": true}),
    );
    if turn.status != Status::Transcribing {
        event("transcript.ready", json!({"text": turn.transcript}));
    }
    if let Some(text) = &turn.approved_text {
        event("question.approved", json!({"text": text}));
        event("reply.started", json!({}));
    }
    match turn.status {
        Status::Completed => event("reply.completed", json!({"text": turn.reply})),
        Status::Failed => event("turn.failed", json!({"error": turn.error})),
        Status::Cancelled => event("turn.cancelled", json!({})),
        // A snapshot is not a live stream. Never fake deltas for a saved buffer.
        _ => {}
    }
    drop(sender);
    response(receiver, None)
}
