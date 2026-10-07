//! One in-memory development conversation; isolated tests share compute, not history.
mod stream;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use metrics::{MetricSample, MetricScope, MetricUnit, MetricsContext};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch, Notify, OwnedSemaphorePermit, Semaphore};
use va_core::{
    build_messages, validate_response, ConversationConfig, ConversationError, ConversationMessage,
    ConversationModel, ConversationRole, Engine, LoadedPrompt, TranscriptionStatus,
};

use crate::{request_metrics, sample_resources, AppState};

const EVENT_CAPACITY: usize = 64;
const MAX_JSON_BYTES: usize = 64 * 1024;

pub struct Limits {
    pub conversation: ConversationConfig,
    pub max_body_bytes: usize,
    pub retained_turns: usize,
    pub retained_idempotency_keys: usize,
    pub review_timeout: Duration,
    pub generation_timeout: Duration,
    pub transcription_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            conversation: ConversationConfig::default(),
            max_body_bytes: 16 * 1024 * 1024,
            retained_turns: 32,
            retained_idempotency_keys: 256,
            review_timeout: Duration::from_secs(300),
            generation_timeout: Duration::from_secs(120),
            transcription_timeout: Duration::from_secs(180),
        }
    }
}

#[derive(Clone, Serialize)]
pub struct RuntimeStatus {
    pub name: String,
    pub ready: bool,
}

#[derive(Clone, Serialize)]
struct Role {
    name: String,
    sha256: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Transcribing,
    AwaitingReview,
    Generating,
    Completed,
    Failed,
    Cancelled,
}

impl Status {
    fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Serialize)]
struct Turn {
    turn_id: String,
    status: Status,
    transcript: String,
    approved_text: Option<String>,
    reply: String,
    error: Option<Failure>,
    timings: BTreeMap<String, f64>,
    role_sha256: Option<String>,
}

#[derive(Clone, Serialize, Debug)]
pub struct Failure {
    code: &'static str,
    message: &'static str,
}

impl Failure {
    fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
}

pub struct HttpError(StatusCode, Failure);
impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}
fn error(status: StatusCode, code: &'static str, message: &'static str) -> HttpError {
    HttpError(status, Failure::new(code, message))
}
fn busy() -> HttpError {
    error(
        StatusCode::CONFLICT,
        "busy",
        "Another inference operation or web turn is active; retry explicitly.",
    )
}
fn missing() -> HttpError {
    error(
        StatusCode::NOT_FOUND,
        "turn_not_found",
        "Turn is not retained (expired, reset, or server restarted); it will not be regenerated.",
    )
}
fn unavailable() -> HttpError {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "runtime_unavailable",
        "Reply model or role is unavailable; ask the operator to configure it at startup.",
    )
}

struct Operation {
    cancelled: Arc<AtomicBool>,
    cancel_notify: Notify,
    approval: Notify,
    settled: watch::Sender<bool>,
}
impl Operation {
    fn new() -> Arc<Self> {
        let (settled, _) = watch::channel(false);
        Arc::new(Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            cancel_notify: Notify::new(),
            approval: Notify::new(),
            settled,
        })
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.cancel_notify.notify_one();
    }
    async fn wait(&self) {
        let mut receiver = self.settled.subscribe();
        while !*receiver.borrow_and_update() {
            if receiver.changed().await.is_err() {
                break;
            }
        }
    }
}

struct Job {
    messages: Vec<ConversationMessage>,
    permit: OwnedSemaphorePermit,
}
struct Record {
    turn: Turn,
    epoch: u64,
    op: Arc<Operation>,
    events: Option<mpsc::Sender<stream::Frame>>,
    job: Option<Job>,
    review_started: Option<Instant>,
    started: Instant,
    metrics: MetricsContext,
}
impl Record {
    fn event(&mut self, name: &'static str, mut data: Value) {
        data["turn_id"] = json!(self.turn.turn_id);
        if let Some(sender) = &self.events {
            // A slow browser loses the stream, not the authoritative result. It
            // recovers by GET; inference must never wait for network backpressure.
            if sender.try_send(stream::Frame { name, data }).is_err() {
                self.events = None;
            }
        }
    }
    fn terminal(&mut self, status: Status, failure: Option<Failure>) {
        if self.turn.status.terminal() {
            return;
        }
        self.turn.status = status;
        self.turn.error = failure.clone();
        let duration = elapsed_ms(self.started);
        self.turn.timings.insert("turn_ms".into(), duration);
        timing(&self.metrics, "voice_turn_ms", duration);
        self.metrics.record(MetricSample::text(
            "voice_turn_status",
            format!("{status:?}").to_lowercase(),
            MetricUnit::Status,
            MetricScope::Run,
            "server.voice",
        ));
        count(
            &self.metrics,
            "reply_output_chars",
            self.turn.reply.chars().count(),
        );
        // Approval reserves compute before the worker starts. Cancellation must
        // release that reservation too, not just signal already-running native work.
        self.job = None;
        match status {
            Status::Completed => self.event("reply.completed", json!({"text": self.turn.reply})),
            Status::Cancelled => self.event("turn.cancelled", json!({})),
            _ => self.event("turn.failed", json!({"error": failure})),
        }
        self.events = None;
    }
}

#[derive(Default)]
struct Inner {
    history: Vec<ConversationMessage>,
    records: VecDeque<Record>,
    current: Option<String>,
    // Bounded lifetime tombstones prevent an expired result's known key from
    // silently creating a new turn. At capacity, reject new keys until restart.
    retries: VecDeque<(String, String, String)>,
    operations: HashMap<String, Arc<Operation>>,
    epoch: u64,
    resetting: bool,
    closing: bool,
}
impl Inner {
    fn record(&mut self, id: &str) -> Option<&mut Record> {
        self.records
            .iter_mut()
            .find(|record| record.turn.turn_id == id)
    }
    fn active(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|id| self.operations.contains_key(id))
    }
}

pub struct Voice {
    inner: Mutex<Inner>,
    inference: Arc<Semaphore>,
    model: Option<Arc<Mutex<Box<dyn ConversationModel>>>>,
    prompt: Option<LoadedPrompt>,
    pub stt: RuntimeStatus,
    reply: RuntimeStatus,
    pub limits: Limits,
}

pub struct Lease {
    _permit: OwnedSemaphorePermit,
    _guard: OperationGuard,
}
struct OperationGuard {
    voice: Arc<Voice>,
    id: String,
    op: Arc<Operation>,
}
impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.voice.lock().operations.remove(&self.id);
        self.op.settled.send_replace(true);
    }
}

impl Voice {
    pub fn new(
        engine: Option<&Engine>,
        model: Option<Box<dyn ConversationModel>>,
        prompt: Option<LoadedPrompt>,
        limits: Limits,
    ) -> Arc<Self> {
        let stt = RuntimeStatus {
            name: engine
                .map(|engine| engine.backend_name().to_owned())
                .unwrap_or_else(|| "not configured".into()),
            ready: engine.is_some_and(Engine::is_ready),
        };
        let reply = RuntimeStatus {
            name: model
                .as_ref()
                .map(|model| model.name().to_owned())
                .unwrap_or_else(|| "not configured".into()),
            ready: model.as_ref().is_some_and(|model| model.is_ready()) && prompt.is_some(),
        };
        Arc::new(Self {
            inner: Mutex::new(Inner::default()),
            inference: Arc::new(Semaphore::new(1)),
            model: model.map(|model| Arc::new(Mutex::new(model))),
            prompt,
            stt,
            reply,
            limits,
        })
    }
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    fn reply_status(&self) -> RuntimeStatus {
        let mut status = self.reply.clone();
        status.ready = self.prompt.is_some()
            && self.model.as_ref().is_some_and(|model| {
                match model.try_lock() {
                    Ok(model) => model.is_ready(),
                    // Inference owns the runtime lock. Inspection must stay responsive.
                    Err(std::sync::TryLockError::WouldBlock) => self.reply.ready,
                    Err(std::sync::TryLockError::Poisoned(_)) => false,
                }
            });
        status
    }
    pub fn begin_inference(self: &Arc<Self>) -> Result<Lease, HttpError> {
        let mut inner = self.lock();
        if inner.closing {
            return Err(unavailable());
        }
        let permit = self
            .inference
            .clone()
            .try_acquire_owned()
            .map_err(|_| busy())?;
        let id = crate::new_run_id();
        let op = Operation::new();
        inner.operations.insert(id.clone(), op.clone());
        Ok(Lease {
            _permit: permit,
            _guard: OperationGuard {
                voice: self.clone(),
                id,
                op,
            },
        })
    }
    pub async fn shutdown(self: &Arc<Self>) {
        let operations = {
            let mut inner = self.lock();
            inner.closing = true;
            for record in &mut inner.records {
                if !record.turn.status.terminal() {
                    record.op.cancel();
                    record.terminal(Status::Cancelled, None);
                }
            }
            inner.operations.values().cloned().collect::<Vec<_>>()
        };
        for op in &operations {
            op.cancel();
        }
        for op in operations {
            op.wait().await;
        }
    }
    fn fail(&self, id: &str, epoch: u64, failure: Failure) {
        let mut inner = self.lock();
        if inner.epoch != epoch {
            return;
        }
        if let Some(record) = inner.record(id) {
            record.terminal(Status::Failed, Some(failure));
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/voice/turns", post(create))
        .route("/v1/voice/turns/{turn_id}", get(recover))
        .route("/v1/voice/turns/{turn_id}/submit", post(submit))
        .route("/v1/voice/turns/{turn_id}/cancel", post(cancel))
        .route("/v1/voice/reset", post(reset))
        .route("/v1/voice/inspect", get(inspect))
        .route("/v1/voice/test/reply", post(test_reply))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextRequest {
    text: String,
}

fn text_body(headers: &HeaderMap, body: &[u8]) -> Result<String, HttpError> {
    if media_type(headers) != "application/json" {
        return Err(error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Content-Type must be application/json.",
        ));
    }
    if body.len() > MAX_JSON_BYTES {
        return Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "body_too_large",
            "JSON body exceeds the configured byte limit.",
        ));
    }
    serde_json::from_slice::<TextRequest>(body)
        .map(|request| request.text)
        .map_err(|_| {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "Expected a JSON object containing only text.",
            )
        })
}
fn media_type(headers: &HeaderMap) -> String {
    headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}
fn validate_text(voice: &Voice, text: &str) -> Result<(), HttpError> {
    let prompt = voice.prompt.as_ref().ok_or_else(unavailable)?;
    build_messages(&prompt.text, &[], text, &voice.limits.conversation).map(|_| ())
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid_request", "Text must be non-empty, without invalid controls, and fit the aggregate input limit including the role."))
}

async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, HttpError> {
    let voice = &state.voice;
    if !voice.reply_status().ready {
        return Err(unavailable());
    }
    let kind = media_type(&headers);
    let text = match kind.as_str() {
        "application/json" => {
            let text = text_body(&headers, &body)?;
            validate_text(voice, &text)?;
            Some(text)
        }
        "audio/wav" | "audio/x-wav" => {
            if body.is_empty() {
                return Err(error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "A non-empty WAV body is required.",
                ));
            }
            if !voice.stt.ready {
                return Err(error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "runtime_unavailable",
                    "Transcription runtime is unavailable.",
                ));
            }
            None
        }
        _ => {
            return Err(error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                "Content-Type must be audio/wav or application/json.",
            ))
        }
    };
    let key = match headers.get("idempotency-key") {
        None => None,
        Some(value) => {
            let value = value.to_str().map_err(|_| {
                error(
                    StatusCode::BAD_REQUEST,
                    "invalid_idempotency_key",
                    "Idempotency-Key must be printable ASCII of 1 to 128 bytes.",
                )
            })?;
            if value.is_empty()
                || value.len() > 128
                || !value.bytes().all(|c| (33..=126).contains(&c))
            {
                return Err(error(
                    StatusCode::BAD_REQUEST,
                    "invalid_idempotency_key",
                    "Idempotency-Key must be printable ASCII of 1 to 128 bytes.",
                ));
            }
            Some(value.to_owned())
        }
    };
    let input = text.as_ref().map(|text| text.as_bytes()).unwrap_or(&body);
    let digest = format!(
        "{:x}",
        Sha256::new()
            .chain_update(if text.is_some() {
                b"text".as_slice()
            } else {
                b"wav".as_slice()
            })
            .chain_update([0])
            .chain_update(input)
            .finalize()
    );
    let id = format!("turn_{}", crate::new_run_id());
    let op = Operation::new();
    let (sender, receiver) = mpsc::channel(EVENT_CAPACITY);
    let metrics = request_metrics(&state, &headers, true);
    label_metrics(&metrics, "web_turn", voice);
    let (epoch, permit) = {
        let mut inner = voice.lock();
        if inner.closing || inner.resetting {
            return Err(busy());
        }
        if let Some(key) = &key {
            if let Some((_, old_digest, old_id)) =
                inner.retries.iter().find(|(old_key, _, _)| old_key == key)
            {
                if *old_digest != digest {
                    return Err(error(
                        StatusCode::CONFLICT,
                        "idempotency_conflict",
                        "Idempotency-Key already identifies different input.",
                    ));
                }
                let turn = inner
                    .records
                    .iter()
                    .find(|record| record.turn.turn_id == *old_id)
                    .ok_or_else(|| error(StatusCode::GONE, "turn_expired", "This key identifies a result that is no longer retained; it will not be regenerated."))?
                    .turn
                    .clone();
                return Ok(stream::snapshot(&turn));
            }
            if inner.retries.len() >= voice.limits.retained_idempotency_keys {
                return Err(error(StatusCode::TOO_MANY_REQUESTS, "idempotency_capacity", "The bounded retry ledger is full; restart the development server before admitting new keys."));
            }
        }
        if inner.active() {
            return Err(busy());
        }
        let permit = if text.is_none() {
            Some(
                voice
                    .inference
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| busy())?,
            )
        } else {
            None
        };
        let epoch = inner.epoch;
        let mut record = Record {
            turn: Turn {
                turn_id: id.clone(),
                status: if text.is_some() {
                    Status::AwaitingReview
                } else {
                    Status::Transcribing
                },
                transcript: text.clone().unwrap_or_default(),
                approved_text: None,
                reply: String::new(),
                error: None,
                timings: BTreeMap::new(),
                role_sha256: voice.prompt.as_ref().map(|prompt| prompt.sha256.clone()),
            },
            epoch,
            op: op.clone(),
            events: Some(sender),
            job: None,
            review_started: text.as_ref().map(|_| Instant::now()),
            started: Instant::now(),
            metrics: metrics.clone(),
        };
        record.event("turn.created", json!({}));
        if let Some(text) = &text {
            record.event("transcript.ready", json!({"text": text}));
        }
        inner.operations.insert(id.clone(), op.clone());
        inner.current = Some(id.clone());
        inner.records.push_back(record);
        if let Some(key) = key {
            inner.retries.push_back((key, digest, id.clone()));
        }
        while inner.records.len() > voice.limits.retained_turns.max(1) {
            inner.records.pop_front();
        }
        (epoch, permit)
    };
    let guard = OperationGuard {
        voice: voice.clone(),
        id: id.clone(),
        op: op.clone(),
    };
    tokio::spawn(async move {
        let _guard = guard;
        if let Some(permit) = permit {
            if !transcribe_turn(&state, &id, epoch, &op, body, permit, metrics.clone()).await {
                return;
            }
        } else {
            drop(body);
        }
        web_worker(&state, &id, epoch, &op, metrics).await;
    });
    Ok(stream::response(receiver, None))
}

async fn transcribe_turn(
    state: &AppState,
    id: &str,
    epoch: u64,
    op: &Arc<Operation>,
    body: Bytes,
    permit: OwnedSemaphorePermit,
    metrics: MetricsContext,
) -> bool {
    if op.cancelled.load(Ordering::Acquire) {
        return false;
    }
    let engine = state.engine.clone().expect("ready STT has an engine");
    let sampler = state.resource_sampler.clone();
    let started = Instant::now();
    let mut task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        sample_resources(&sampler, &metrics);
        let result = (|| {
            let audio = va_core::AudioBuffer::from_wav(&body)?;
            engine
                .lock()
                .map_err(|_| va_core::EngineError::Backend {
                    message: "poisoned runtime".into(),
                })?
                .transcribe_with_metrics(audio, metrics.clone())
        })();
        sample_resources(&sampler, &metrics);
        result
    });
    let result = tokio::select! {
        result = &mut task => result,
        _ = tokio::time::sleep(state.voice.limits.transcription_timeout) => {
            op.cancel();
            state.voice.fail(id, epoch, Failure::new("transcription_timeout", "Transcription timed out."));
            // Current core STT has no native abort signal. Keep the permit and
            // await settlement; never overlap another model with detached STT.
            let _ = task.await;
            return false;
        }
        _ = op.cancel_notify.notified() => { let _ = task.await; return false; }
    };
    if op.cancelled.load(Ordering::Acquire) {
        return false;
    }
    let text = match result {
        Ok(Ok(result)) if result.status == TranscriptionStatus::Speech => result.text,
        Ok(Ok(_)) => {
            state.voice.fail(
                id,
                epoch,
                Failure::new(
                    "no_speech",
                    "No usable speech was recognized; record a new turn.",
                ),
            );
            return false;
        }
        Ok(Err(_)) | Err(_) => {
            state.voice.fail(
                id,
                epoch,
                Failure::new(
                    "transcription_failed",
                    "Transcription failed; check the WAV input and runtime.",
                ),
            );
            return false;
        }
    };
    if validate_text(&state.voice, &text).is_err() {
        state.voice.fail(
            id,
            epoch,
            Failure::new(
                "invalid_transcript",
                "Transcript is empty or exceeds the configured input limit.",
            ),
        );
        return false;
    }
    let mut inner = state.voice.lock();
    if inner.epoch != epoch {
        return false;
    }
    if let Some(record) = inner.record(id) {
        if record.turn.status != Status::Transcribing {
            return false;
        }
        record.turn.transcript = text.clone();
        record.turn.status = Status::AwaitingReview;
        record
            .turn
            .timings
            .insert("transcription_ms".into(), elapsed_ms(started));
        record.review_started = Some(Instant::now());
        record.event("transcript.ready", json!({"text": text}));
        return true;
    }
    false
}

async fn web_worker(
    state: &AppState,
    id: &str,
    epoch: u64,
    op: &Arc<Operation>,
    metrics: MetricsContext,
) {
    let deadline = tokio::time::Instant::now() + state.voice.limits.review_timeout;
    let job = loop {
        let job = {
            let mut inner = state.voice.lock();
            if inner.epoch != epoch || op.cancelled.load(Ordering::Acquire) {
                return;
            }
            inner.record(id).and_then(|record| record.job.take())
        };
        if let Some(job) = job {
            break job;
        }
        tokio::select! {
            _ = op.approval.notified() => {},
            _ = op.cancel_notify.notified() => return,
            _ = tokio::time::sleep_until(deadline) => {
                let mut inner = state.voice.lock();
                if inner.epoch != epoch { return; }
                let Some(record) = inner.record(id) else { return; };
                // Submission and expiry are serialized by this same lock.
                if let Some(job) = record.job.take() { break job; }
                record.terminal(Status::Failed, Some(Failure::new("review_timeout", "Transcript approval timed out.")));
                return;
            }
        }
    };
    let started = Instant::now();
    let result = generate(
        state,
        Some((id.to_owned(), epoch)),
        op,
        job,
        metrics.clone(),
        None,
    )
    .await;
    let mut inner = state.voice.lock();
    if inner.epoch != epoch || inner.current.as_deref() != Some(id) {
        return;
    }
    let Some(record) = inner.record(id) else {
        return;
    };
    record
        .turn
        .timings
        .insert("generation_ms".into(), elapsed_ms(started));
    if record.turn.status.terminal() {
        return;
    }
    match result {
        Ok(text) if !op.cancelled.load(Ordering::Acquire) => {
            record.turn.reply = text.clone();
            let approved = record.turn.approved_text.clone().expect("submitted turn");
            record.terminal(Status::Completed, None);
            inner.history.push(ConversationMessage {
                role: ConversationRole::User,
                content: approved,
            });
            inner.history.push(ConversationMessage {
                role: ConversationRole::Assistant,
                content: text,
            });
            trim_history(&mut inner.history, &state.voice.limits.conversation);
            metrics.record_workflow_outcome(true);
        }
        Ok(_) => record.terminal(Status::Cancelled, None),
        Err(failure) if failure.code == "cancelled" => record.terminal(Status::Cancelled, None),
        Err(failure) => {
            record.terminal(Status::Failed, Some(failure));
            metrics.record_workflow_outcome(false);
        }
    }
}

fn trim_history(history: &mut Vec<ConversationMessage>, config: &ConversationConfig) {
    let mut chars = history
        .iter()
        .map(|message| message.content.chars().count())
        .sum::<usize>();
    while history.len() > config.max_history_turns.saturating_mul(2)
        || chars > config.max_input_chars
    {
        if history.len() < 2 {
            break;
        }
        chars -= history[0].content.chars().count() + history[1].content.chars().count();
        history.drain(..2);
    }
}

async fn submit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Turn>, HttpError> {
    let text = text_body(&headers, &body)?;
    let voice = &state.voice;
    validate_text(voice, &text)?;
    let mut inner = voice.lock();
    if inner.resetting || inner.closing {
        return Err(busy());
    }
    let record = inner.record(&id).ok_or_else(missing)?;
    if let Some(approved) = &record.turn.approved_text {
        return if *approved == text {
            Ok(Json(record.turn.clone()))
        } else {
            Err(error(
                StatusCode::CONFLICT,
                "question_conflict",
                "This turn's approved question is already frozen.",
            ))
        };
    }
    if record.turn.status != Status::AwaitingReview {
        return Err(error(
            StatusCode::CONFLICT,
            "turn_not_reviewable",
            "This turn is not awaiting transcript approval.",
        ));
    }
    if record
        .review_started
        .is_some_and(|start| start.elapsed() >= voice.limits.review_timeout)
    {
        record.terminal(
            Status::Failed,
            Some(Failure::new(
                "review_timeout",
                "Transcript approval timed out.",
            )),
        );
        record.op.approval.notify_one();
        return Err(error(
            StatusCode::CONFLICT,
            "review_timeout",
            "Transcript approval timed out.",
        ));
    }
    if !voice.reply_status().ready {
        return Err(unavailable());
    }
    let messages = build_messages(
        &voice.prompt.as_ref().ok_or_else(unavailable)?.text,
        &inner.history,
        &text,
        &voice.limits.conversation,
    )
    .map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Question exceeds the configured conversation input budget.",
        )
    })?;
    let permit = voice
        .inference
        .clone()
        .try_acquire_owned()
        .map_err(|_| busy())?;
    let history_turns = messages.len().saturating_sub(2) / 2;
    let removed_turns = (inner.history.len() / 2).saturating_sub(history_turns);
    let record = inner.record(&id).expect("record exists under same lock");
    count(&record.metrics, "reply_input_chars", text.chars().count());
    count(&record.metrics, "reply_history_turns", history_turns);
    count(
        &record.metrics,
        "reply_history_truncated_turns",
        removed_turns,
    );
    record.turn.approved_text = Some(text.clone());
    record.turn.status = Status::Generating;
    let review_ms = record.review_started.map(elapsed_ms).unwrap_or_default();
    record.turn.timings.insert("review_ms".into(), review_ms);
    timing(&record.metrics, "human_review_ms", review_ms);
    record.event("question.approved", json!({"text": text}));
    record.event("reply.started", json!({}));
    record.job = Some(Job { messages, permit });
    record.op.approval.notify_one();
    Ok(Json(record.turn.clone()))
}

async fn recover(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Turn>, HttpError> {
    Ok(Json(
        state
            .voice
            .lock()
            .record(&id)
            .ok_or_else(missing)?
            .turn
            .clone(),
    ))
}

async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Turn>, HttpError> {
    let mut inner = state.voice.lock();
    let record = inner.record(&id).ok_or_else(missing)?;
    if !record.turn.status.terminal() {
        record.op.cancel();
        record.terminal(Status::Cancelled, None);
    }
    Ok(Json(record.turn.clone()))
}

async fn reset(State(state): State<AppState>) -> Result<Response, HttpError> {
    let op = {
        let mut inner = state.voice.lock();
        if inner.resetting || inner.closing {
            return Err(busy());
        }
        inner.resetting = true;
        inner.epoch = inner.epoch.wrapping_add(1);
        let id = inner.current.clone();
        id.and_then(|id| inner.record(&id)).map(|record| {
            record.op.cancel();
            record.terminal(Status::Cancelled, None);
            record.op.clone()
        })
    };
    // Own settlement even if the reset HTTP caller disconnects.
    let voice = state.voice.clone();
    let task = tokio::spawn(async move {
        if let Some(op) = op {
            op.wait().await;
        }
        let mut inner = voice.lock();
        inner.history.clear();
        inner.current = None;
        inner.records.clear();
        inner.resetting = false;
    });
    task.await.map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "reset_failed",
            "Conversation reset failed.",
        )
    })?;
    Ok(Json(json!({"status": "reset"})).into_response())
}

async fn inspect(State(state): State<AppState>) -> Json<Value> {
    let voice = &state.voice;
    let reply = voice.reply_status();
    let inner = voice.lock();
    let turn = inner
        .current
        .as_ref()
        .and_then(|id| {
            inner
                .records
                .iter()
                .find(|record| record.turn.turn_id == *id)
        })
        .map(|record| &record.turn);
    let role = voice.prompt.as_ref().map(|prompt| Role {
        name: prompt.path.to_string_lossy().into_owned(),
        sha256: prompt.sha256.clone(),
    });
    Json(
        json!({"history": inner.history, "current_turn": turn, "stt": voice.stt, "reply": reply, "role": role, "busy": voice.inference.available_permits() == 0 || inner.resetting || inner.closing}),
    )
}

async fn test_reply(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, HttpError> {
    let text = text_body(&headers, &body)?;
    let voice = &state.voice;
    if !voice.reply_status().ready {
        return Err(unavailable());
    }
    validate_text(voice, &text)?;
    let messages = build_messages(
        &voice.prompt.as_ref().ok_or_else(unavailable)?.text,
        &[],
        &text,
        &voice.limits.conversation,
    )
    .map_err(|_| unavailable())?;
    let lease = voice.begin_inference()?;
    let op = lease._guard.op.clone();
    let cancelled = op.cancelled.clone();
    let (sender, receiver) = mpsc::channel(EVENT_CAPACITY);
    let metrics = request_metrics(&state, &headers, true);
    label_metrics(&metrics, "isolated_test", voice);
    count(&metrics, "reply_input_chars", text.chars().count());
    count(&metrics, "reply_history_turns", 0);
    let started = Instant::now();
    tokio::spawn(async move {
        let Lease {
            _permit: permit,
            _guard: guard,
        } = lease;
        let _guard = guard;
        let _ = sender.try_send(stream::Frame {
            name: "reply.started",
            data: json!({}),
        });
        let result = generate(
            &state,
            None,
            &op,
            Job { messages, permit },
            metrics.clone(),
            Some(sender.clone()),
        )
        .await;
        timing(&metrics, "voice_test_ms", elapsed_ms(started));
        metrics.record_workflow_outcome(result.is_ok());
        let frame = match result {
            Ok(text) => {
                count(&metrics, "reply_output_chars", text.chars().count());
                stream::Frame {
                    name: "reply.completed",
                    data: json!({"text": text}),
                }
            }
            Err(failure) if failure.code == "cancelled" => stream::Frame {
                name: "turn.cancelled",
                data: json!({}),
            },
            Err(failure) => stream::Frame {
                name: "turn.failed",
                data: json!({"error": failure}),
            },
        };
        let _ = sender.try_send(frame);
    });
    Ok(stream::response(receiver, Some(cancelled)))
}

async fn generate(
    state: &AppState,
    target: Option<(String, u64)>,
    op: &Arc<Operation>,
    job: Job,
    metrics: MetricsContext,
    events: Option<mpsc::Sender<stream::Frame>>,
) -> Result<String, Failure> {
    let model = state
        .voice
        .model
        .clone()
        .ok_or_else(|| Failure::new("runtime_unavailable", "Reply model is unavailable."))?;
    let voice = state.voice.clone();
    let config = voice.limits.conversation.clone();
    let cancelled = op.cancelled.clone();
    let sampler = state.resource_sampler.clone();
    let invalid_output = Arc::new(AtomicBool::new(false));
    let invalid = invalid_output.clone();
    let started = Instant::now();
    let metrics_for_finish = metrics.clone();
    let timeout_target = target.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        let _permit = job.permit;
        sample_resources(&sampler, &metrics);
        let mut chars = 0usize;
        let mut first = true;
        let mut emit = |text: &str| {
            if cancelled.load(Ordering::Acquire) || text.is_empty() {
                return;
            }
            chars = chars.saturating_add(text.chars().count());
            if chars > config.max_output_chars
                || text
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            {
                invalid.store(true, Ordering::Release);
                cancelled.store(true, Ordering::Release);
                return;
            }
            if first {
                let ms = elapsed_ms(started);
                timing(&metrics, "reply_first_text_ms", ms);
                if let Some((id, epoch)) = &target {
                    let mut inner = voice.lock();
                    if inner.epoch == *epoch {
                        if let Some(record) = inner.record(id) {
                            record.turn.timings.insert("first_text_ms".into(), ms);
                        }
                    }
                }
                first = false;
            }
            if let Some((id, epoch)) = &target {
                let mut inner = voice.lock();
                if inner.epoch != *epoch {
                    return;
                }
                if let Some(record) = inner.record(id) {
                    if record.epoch == *epoch && record.turn.status == Status::Generating {
                        record.turn.reply.push_str(text);
                        record.event("reply.delta", json!({"text": text}));
                    }
                }
            } else if let Some(sender) = &events {
                if sender
                    .try_send(stream::Frame {
                        name: "reply.delta",
                        data: json!({"text": text}),
                    })
                    .is_err()
                {
                    cancelled.store(true, Ordering::Release);
                }
            }
        };
        let result = match model.lock() {
            Ok(mut model) if model.is_ready() => {
                model.respond_stream(&job.messages, &config, &cancelled, &mut emit)
            }
            _ => Err(ConversationError::Backend("runtime unavailable".into())),
        }
        .and_then(|text| {
            validate_response(&text, &config)?;
            Ok(text)
        });
        sample_resources(&sampler, &metrics);
        result
    });
    let result = tokio::select! {
        result = &mut task => result,
        _ = tokio::time::sleep(state.voice.limits.generation_timeout) => {
            op.cancel();
            if let Some((id, epoch)) = &timeout_target {
                state.voice.fail(id, *epoch, Failure::new("generation_timeout", "Reply generation timed out."));
            }
            let _ = task.await;
            timing(&metrics_for_finish, "reply_generation_ms", elapsed_ms(started));
            return Err(Failure::new("generation_timeout", "Reply generation timed out."));
        }
        _ = op.cancel_notify.notified() => {
            let _ = task.await;
            return Err(Failure::new("cancelled", "Reply generation cancelled."));
        }
    };
    timing(
        &metrics_for_finish,
        "reply_generation_ms",
        elapsed_ms(started),
    );
    if invalid_output.load(Ordering::Acquire) {
        return Err(Failure::new(
            "invalid_model_response",
            "Reply output exceeds limits or contains invalid controls.",
        ));
    }
    if op.cancelled.load(Ordering::Acquire) {
        return Err(Failure::new("cancelled", "Reply generation cancelled."));
    }
    match result {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(ConversationError::ContextExceeded | ConversationError::InputTooLong { .. })) => {
            Err(Failure::new(
                "context_exceeded",
                "Question and history exceed the reply model context budget.",
            ))
        }
        Ok(Err(ConversationError::Cancelled)) => {
            Err(Failure::new("cancelled", "Reply generation cancelled."))
        }
        Ok(Err(ConversationError::OutputTooLong { .. } | ConversationError::EmptyResponse)) => {
            Err(Failure::new(
                "invalid_model_response",
                "Reply model returned an invalid answer.",
            ))
        }
        Ok(Err(_)) | Err(_) => Err(Failure::new("reply_failed", "Reply generation failed.")),
    }
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}
fn timing(metrics: &MetricsContext, name: &str, ms: f64) {
    metrics.record(MetricSample::number(
        name,
        ms,
        MetricUnit::Milliseconds,
        MetricScope::Run,
        "server.voice",
    ));
}
fn count(metrics: &MetricsContext, name: &str, value: usize) {
    metrics.record(MetricSample::integer(
        name,
        value as u64,
        MetricUnit::Count,
        MetricScope::Run,
        "server.voice",
    ));
}
fn label_metrics(metrics: &MetricsContext, kind: &str, voice: &Voice) {
    metrics.record(MetricSample::text(
        "reply_backend",
        &voice.reply.name,
        MetricUnit::Status,
        MetricScope::Run,
        "server.voice",
    ));
    metrics.record(MetricSample::unavailable(
        "reply_input_tokens",
        MetricUnit::Count,
        MetricScope::Run,
        "server.voice",
        "the conversation adapter does not export token counts",
    ));
    metrics.record(MetricSample::unavailable(
        "reply_output_tokens",
        MetricUnit::Count,
        MetricScope::Run,
        "server.voice",
        "the conversation adapter does not export token counts",
    ));
    metrics.record(MetricSample::unavailable(
        "reply_runtime_cpu_percent",
        MetricUnit::Percent,
        MetricScope::Process,
        "server.voice",
        "the current desktop sampler covers the server, not its native reply child",
    ));
    metrics.record(MetricSample::unavailable(
        "reply_runtime_ram_bytes",
        MetricUnit::Bytes,
        MetricScope::Process,
        "server.voice",
        "the current desktop sampler covers the server, not its native reply child",
    ));
    metrics.record(MetricSample::text(
        "voice_operation",
        kind,
        MetricUnit::Status,
        MetricScope::Run,
        "server.voice",
    ));
    if let Some(prompt) = &voice.prompt {
        metrics.record(MetricSample::text(
            "role_sha256",
            &prompt.sha256,
            MetricUnit::Status,
            MetricScope::Run,
            "server.voice",
        ));
    }
}
