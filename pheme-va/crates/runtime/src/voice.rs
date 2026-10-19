//! One in-memory development conversation; isolated tests share compute, not history.
#[path = "conversations.rs"]
pub mod conversations;
#[path = "events.rs"]
pub mod events;
use events::EventKind;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use metrics::{MetricSample, MetricScope, MetricUnit, MetricsContext};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch, Notify, OwnedSemaphorePermit, Semaphore};
use va_core::{
    build_messages, validate_response, ConversationConfig, ConversationError, ConversationMessage,
    ConversationModel, Engine, LoadedPrompt, TranscriptionStatus,
};

use crate::{request_metrics, sample_resources, AgentRuntime, ErrorKind, RequestOptions};

const EVENT_CAPACITY: usize = 64;

#[derive(Clone)]
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
pub struct Role {
    name: String,
    sha256: String,
}

use va_core::chat::{ChatPhase as Status, ConversationState};

#[derive(Clone, Debug, Serialize)]
pub struct Turn {
    pub turn_id: String,
    pub status: Status,
    pub transcript: String,
    pub approved_text: Option<String>,
    pub reply: String,
    pub error: Option<Failure>,
    pub timings: BTreeMap<String, f64>,
    pub role_sha256: Option<String>,
}

#[derive(Clone, Serialize, Debug)]
pub struct Failure {
    pub code: &'static str,
    pub message: &'static str,
}

impl Failure {
    pub(crate) fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
}

#[derive(Debug)]
pub struct RuntimeError(pub ErrorKind, pub Failure);
impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.1.message)
    }
}
impl std::error::Error for RuntimeError {}
fn error(kind: ErrorKind, code: &'static str, message: &'static str) -> RuntimeError {
    RuntimeError(kind, Failure::new(code, message))
}
fn busy() -> RuntimeError {
    error(
        ErrorKind::Conflict,
        "busy",
        "Another inference operation or web turn is active; retry explicitly.",
    )
}
fn missing() -> RuntimeError {
    error(
        ErrorKind::NotFound,
        "turn_not_found",
        "Turn is not retained (expired, reset, or server restarted); it will not be regenerated.",
    )
}
fn unavailable() -> RuntimeError {
    error(
        ErrorKind::Unavailable,
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
    events: Option<mpsc::Sender<events::RuntimeEvent>>,
    changes: tokio::sync::watch::Sender<u64>,
    job: Option<Job>,
    review_started: Option<Instant>,
    started: Instant,
    metrics: MetricsContext,
    transcription_metrics: Option<MetricsContext>,
    transcription: Option<va_core::TranscriptionResult>,
    audio_duration_seconds: Option<f32>,
    started_at_ms: u64,
    reply_started_at_ms: Option<u64>,
    reply_finished_at_ms: Option<u64>,
    transcription_finished_at_ms: Option<u64>,
    source: String,
}
impl Record {
    fn event(&mut self, kind: EventKind) {
        let event = events::RuntimeEvent::new(Some(self.turn.turn_id.clone()), kind);
        self.changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
        if let Some(sender) = &self.events {
            if sender.try_send(event).is_err() {
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
            "runtime.voice",
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
            Status::Completed => self.event(EventKind::ReplyCompleted {
                text: self.turn.reply.to_owned(),
            }),
            Status::Cancelled => self.event(EventKind::TurnCancelled),
            _ => self.event(EventKind::TurnFailed { error: failure }),
        }
        self.events = None;
    }
}

#[derive(Default)]
struct Inner {
    conversation: ConversationState,
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
    pub(crate) stt: Arc<Mutex<RuntimeStatus>>,
    reply: RuntimeStatus,
    pub limits: Limits,
    console: bool,
    console_id: String,
    conversations: Mutex<BTreeMap<String, Arc<conversations::Entry>>>,
    changes: watch::Sender<u64>,
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
        self.voice
            .changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
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
            stt: Arc::new(Mutex::new(stt)),
            changes: watch::channel(0).0,
            reply,
            limits,
            console: false,
            console_id: String::new(),
            conversations: Mutex::new(BTreeMap::new()),
        })
    }
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    pub fn reply_status(&self) -> RuntimeStatus {
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
    pub fn stt_status(&self) -> RuntimeStatus {
        self.stt.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    pub fn idle(&self) -> bool {
        if !self.lock().operations.is_empty() || self.inference.available_permits() == 0 {
            return false;
        }
        self.conversations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .all(|entry| entry.voice.idle())
    }
    pub fn current_id(&self) -> Option<String> {
        self.lock().current.clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    pub fn begin_inference(self: &Arc<Self>) -> Result<Lease, RuntimeError> {
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
        self.lock().closing = true;
        let children = self
            .conversations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(|entry| entry.voice.clone())
            .collect::<Vec<_>>();
        for child in children {
            Box::pin(child.shutdown()).await;
        }
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

fn validate_text(voice: &Voice, text: &str) -> Result<(), RuntimeError> {
    let system = voice
        .prompt
        .as_ref()
        .map(|p| p.text.as_str())
        .unwrap_or("_");
    build_messages(system, &[], text, &voice.limits.conversation).map(|_| ())
        .map_err(|_| error(ErrorKind::InvalidInput, "invalid_request", "Text must be non-empty, without invalid controls, and fit the aggregate input limit including the role."))
}

pub async fn create(
    state: AgentRuntime,
    input: TurnInput,
    options: RequestOptions,
    confirmed: bool,
) -> Result<events::TurnStream, RuntimeError> {
    let voice = &state.voice;
    if confirmed && matches!(&input, TurnInput::Audio(_)) {
        return Err(error(
            ErrorKind::InvalidInput,
            "invalid_request",
            "Only typed text can be sent without transcript review.",
        ));
    }
    let (text, body) = match input {
        TurnInput::Text(text) => {
            validate_text(voice, &text)?;
            (Some(text), Vec::new())
        }
        TurnInput::Audio(body) => {
            if body.len() > 16 * 1024 * 1024 {
                return Err(error(
                    ErrorKind::InvalidInput,
                    "invalid_request",
                    "WAV input exceeds the 16 MiB limit.",
                ));
            }
            if body.is_empty() {
                return Err(error(
                    ErrorKind::InvalidInput,
                    "invalid_request",
                    "A non-empty WAV body is required.",
                ));
            }
            if !voice.stt_status().ready {
                return Err(unavailable());
            }
            (None, body)
        }
    };
    if (text.is_some() || !voice.console) && !voice.reply_status().ready {
        return Err(unavailable());
    }
    let key = options.idempotency_key.clone();
    if key.as_ref().is_some_and(|v| {
        v.is_empty() || v.len() > 128 || !v.bytes().all(|c| (33..=126).contains(&c))
    }) {
        return Err(error(
            ErrorKind::InvalidInput,
            "invalid_idempotency_key",
            "Idempotency-Key must be printable ASCII of 1 to 128 bytes.",
        ));
    }
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
    let metrics = if voice.console && text.is_some() {
        MetricsContext::disabled()
    } else {
        request_metrics(&state, &options, true)
    };
    label_metrics(
        &metrics,
        if voice.console {
            "console_transcription"
        } else {
            "web_turn"
        },
        voice,
    );
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
                        ErrorKind::Conflict,
                        "idempotency_conflict",
                        "Idempotency-Key already identifies different input.",
                    ));
                }
                let turn = inner
                    .records
                    .iter()
                    .find(|record| record.turn.turn_id == *old_id)
                    .ok_or_else(|| error(ErrorKind::Expired, "turn_expired", "This key identifies a result that is no longer retained; it will not be regenerated."))?
                    .turn
                    .clone();
                return Ok(events::TurnStream::recovered(turn));
            }
            if inner.retries.len() >= voice.limits.retained_idempotency_keys {
                return Err(error(ErrorKind::Capacity, "idempotency_capacity", "The bounded retry ledger is full; restart the development host before admitting new keys."));
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
        let confirmed_job = if confirmed {
            let text = text.as_ref().expect("confirmed input is text");
            let messages = inner
                .conversation
                .approve(
                    Status::AwaitingReview,
                    &voice.prompt.as_ref().ok_or_else(unavailable)?.text,
                    text,
                    &voice.limits.conversation,
                )
                .map_err(|_| {
                    error(
                        ErrorKind::InvalidInput,
                        "invalid_request",
                        "Question exceeds the configured conversation input budget.",
                    )
                })?;
            let permit = voice
                .inference
                .clone()
                .try_acquire_owned()
                .map_err(|_| busy())?;
            Some(Job { messages, permit })
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
            changes: voice.changes.clone(),
            job: None,
            review_started: text.as_ref().map(|_| Instant::now()),
            started: Instant::now(),
            metrics: metrics.clone(),
            transcription_metrics: (voice.console && text.is_none()).then(|| metrics.clone()),
            transcription: None,
            audio_duration_seconds: None,
            started_at_ms: conversations::now_ms(),
            reply_started_at_ms: None,
            reply_finished_at_ms: None,
            transcription_finished_at_ms: None,
            source: if text.is_some() {
                "typed text".into()
            } else {
                if options.audio_source.is_empty() {
                    "audio".into()
                } else {
                    options.audio_source.chars().take(512).collect()
                }
            },
        };
        if let Some(job) = confirmed_job {
            let text = text.clone().expect("typed input");
            let options = RequestOptions {
                run_id: Some(format!("reply-{}-{}", voice.console_id, id)),
                ..Default::default()
            };
            record.metrics = request_metrics(&state, &options, true);
            label_metrics(&record.metrics, "console_reasoning", voice);
            count(&record.metrics, "reply_input_chars", text.chars().count());
            count(
                &record.metrics,
                "reply_history_turns",
                job.messages.len().saturating_sub(2) / 2,
            );
            record.turn.status = Status::Generating;
            record.turn.approved_text = Some(text);
            record.reply_started_at_ms = Some(conversations::now_ms());
            record.job = Some(job);
        }
        record.event(EventKind::TurnCreated { recovered: false });
        if confirmed {
            record.event(EventKind::QuestionApproved {
                text: text.clone().unwrap_or_default(),
            });
            record.event(EventKind::ReplyStarted);
        } else if let Some(text) = &text {
            record.event(EventKind::TranscriptReady {
                text: text.to_owned(),
            });
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
    let admitted = recover(&state, &id)?;
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
    Ok(events::TurnStream {
        turn: admitted,
        receiver,
        cancel_on_drop: None,
    })
}

async fn transcribe_turn(
    state: &AgentRuntime,
    id: &str,
    epoch: u64,
    op: &Arc<Operation>,
    body: Vec<u8>,
    permit: OwnedSemaphorePermit,
    metrics: MetricsContext,
) -> bool {
    if op.cancelled.load(Ordering::Acquire) {
        return false;
    }
    let engine = state.engine.clone();
    let sampler = state.resource_sampler.clone();
    if let Ok(audio) = va_core::AudioBuffer::from_wav(&body) {
        if let Some(record) = state.voice.lock().record(id) {
            record.audio_duration_seconds = Some(audio.duration_seconds());
        }
    }
    let sampling = conversations::sample_during(state, &metrics);
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
                .as_mut()
                .ok_or_else(|| va_core::EngineError::Backend {
                    message: "STT unavailable".into(),
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
            task.await
        }
        _ = op.cancel_notify.notified() => task.await,
    };
    drop(sampling);
    if let Some(record) = state.voice.lock().record(id) {
        record.transcription_finished_at_ms = Some(conversations::now_ms());
        if let Ok(Ok(result)) = &result {
            record.transcription = Some(result.clone());
        }
    }
    if op.cancelled.load(Ordering::Acquire) {
        return false;
    }
    let text = match result {
        Ok(Ok(result)) if result.status == TranscriptionStatus::Speech => {
            let text = result.text.clone();
            if let Some(record) = state.voice.lock().record(id) {
                record.transcription = Some(result);
            }
            text
        }
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
        record.event(EventKind::TranscriptReady {
            text: text.to_owned(),
        });
        return true;
    }
    false
}

async fn web_worker(
    state: &AgentRuntime,
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
    let metrics = state
        .voice
        .lock()
        .record(id)
        .map(|r| r.metrics.clone())
        .unwrap_or(metrics);
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
    record.reply_finished_at_ms = Some(conversations::now_ms());
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
            let _ = inner
                .conversation
                .commit(approved, text, &state.voice.limits.conversation);
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

pub fn submit(state: &AgentRuntime, id: &str, text: String) -> Result<Turn, RuntimeError> {
    let voice = &state.voice;
    validate_text(voice, &text)?;
    let mut inner = voice.lock();
    if inner.resetting || inner.closing {
        return Err(busy());
    }
    let record = inner.record(id).ok_or_else(missing)?;
    if let Some(approved) = &record.turn.approved_text {
        return if *approved == text {
            Ok(record.turn.clone())
        } else {
            Err(error(
                ErrorKind::Conflict,
                "question_conflict",
                "This turn's approved question is already frozen.",
            ))
        };
    }
    if record.turn.status != Status::AwaitingReview {
        return Err(error(
            ErrorKind::Conflict,
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
            ErrorKind::Conflict,
            "review_timeout",
            "Transcript approval timed out.",
        ));
    }
    if !voice.reply_status().ready {
        return Err(unavailable());
    }
    let messages = inner
        .conversation
        .approve(
            Status::AwaitingReview,
            &voice.prompt.as_ref().ok_or_else(unavailable)?.text,
            &text,
            &voice.limits.conversation,
        )
        .map_err(|_| {
            error(
                ErrorKind::InvalidInput,
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
    let removed_turns = (inner.conversation.history.len() / 2).saturating_sub(history_turns);
    let record = inner.record(id).expect("record exists under same lock");
    if voice.console {
        let reply_options = RequestOptions {
            run_id: Some(format!("reply-{}-{}", voice.console_id, id)),
            ..Default::default()
        };
        record.metrics = request_metrics(state, &reply_options, true);
        label_metrics(&record.metrics, "console_reasoning", voice);
    }
    count(&record.metrics, "reply_input_chars", text.chars().count());
    count(&record.metrics, "reply_history_turns", history_turns);
    count(
        &record.metrics,
        "reply_history_truncated_turns",
        removed_turns,
    );
    record.reply_started_at_ms = Some(conversations::now_ms());
    record.turn.approved_text = Some(text.clone());
    record.turn.status = Status::Generating;
    let review_ms = record.review_started.map(elapsed_ms).unwrap_or_default();
    record.turn.timings.insert("review_ms".into(), review_ms);
    timing(&record.metrics, "human_review_ms", review_ms);
    record.event(EventKind::QuestionApproved {
        text: text.to_owned(),
    });
    record.event(EventKind::ReplyStarted);
    record.job = Some(Job { messages, permit });
    record.op.approval.notify_one();
    Ok(record.turn.clone())
}

pub fn recover(state: &AgentRuntime, id: &str) -> Result<Turn, RuntimeError> {
    Ok(state
        .voice
        .lock()
        .record(id)
        .ok_or_else(missing)?
        .turn
        .clone())
}

pub fn cancel(state: &AgentRuntime, id: &str) -> Result<Turn, RuntimeError> {
    let mut inner = state.voice.lock();
    let record = inner.record(id).ok_or_else(missing)?;
    if !record.turn.status.terminal() {
        record.op.cancel();
        record.terminal(Status::Cancelled, None);
    }
    Ok(record.turn.clone())
}

pub async fn reset(state: &AgentRuntime) -> Result<(), RuntimeError> {
    let op = {
        let mut inner = state.voice.lock();
        if inner.resetting || inner.closing {
            return Err(busy());
        }
        inner.resetting = true;
        inner.epoch = inner.epoch.wrapping_add(1);
        let id = inner.current.clone();
        id.as_ref().and_then(|id| inner.record(id)).map(|record| {
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
        inner.conversation.history.clear();
        inner.current = None;
        inner.records.clear();
        inner.resetting = false;
    });
    task.await.map_err(|_| {
        error(
            ErrorKind::Internal,
            "reset_failed",
            "Conversation reset failed.",
        )
    })?;
    Ok(())
}

pub fn inspect(state: &AgentRuntime) -> Inspection {
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
    Inspection {
        history: inner.conversation.history.clone(),
        current_turn: turn.cloned(),
        stt: voice.stt_status(),
        reply,
        role,
        busy: voice.inference.available_permits() == 0 || inner.resetting || inner.closing,
    }
}

pub fn test_reply(
    state: AgentRuntime,
    text: String,
    options: RequestOptions,
) -> Result<events::TurnStream, RuntimeError> {
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
    let metrics = request_metrics(&state, &options, true);
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
        let _ = sender.try_send(events::RuntimeEvent::new(None, EventKind::ReplyStarted));
        let generation = generate(
            &state,
            None,
            &op,
            Job { messages, permit },
            metrics.clone(),
            Some(sender.clone()),
        );
        tokio::pin!(generation);
        let result = tokio::select! {
            result = &mut generation => result,
            _ = sender.closed() => {
                op.cancel();
                generation.await
            }
        };
        timing(&metrics, "voice_test_ms", elapsed_ms(started));
        metrics.record_workflow_outcome(result.is_ok());
        let frame = match result {
            Ok(text) => {
                count(&metrics, "reply_output_chars", text.chars().count());
                events::RuntimeEvent::new(
                    None,
                    EventKind::ReplyCompleted {
                        text: text.to_owned(),
                    },
                )
            }
            Err(failure) if failure.code == "cancelled" => {
                events::RuntimeEvent::new(None, EventKind::TurnCancelled)
            }
            Err(failure) => events::RuntimeEvent::new(
                None,
                EventKind::TurnFailed {
                    error: Some(failure),
                },
            ),
        };
        let _ = sender.try_send(frame);
    });
    Ok(events::TurnStream {
        turn: Turn::test(),
        receiver,
        cancel_on_drop: Some(cancelled),
    })
}

async fn generate(
    state: &AgentRuntime,
    target: Option<(String, u64)>,
    op: &Arc<Operation>,
    job: Job,
    metrics: MetricsContext,
    events: Option<mpsc::Sender<events::RuntimeEvent>>,
) -> Result<String, Failure> {
    let _sampling = conversations::sample_during(state, &metrics);
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
                        record.event(EventKind::ReplyDelta {
                            text: text.to_owned(),
                        });
                    }
                }
            } else if let Some(sender) = &events {
                if sender
                    .try_send(events::RuntimeEvent::new(
                        None,
                        EventKind::ReplyDelta {
                            text: text.to_owned(),
                        },
                    ))
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
        "runtime.voice",
    ));
}
fn count(metrics: &MetricsContext, name: &str, value: usize) {
    metrics.record(MetricSample::integer(
        name,
        value as u64,
        MetricUnit::Count,
        MetricScope::Run,
        "runtime.voice",
    ));
}
fn label_metrics(metrics: &MetricsContext, kind: &str, voice: &Voice) {
    metrics.record(MetricSample::text(
        "reply_backend",
        &voice.reply.name,
        MetricUnit::Status,
        MetricScope::Run,
        "runtime.voice",
    ));
    metrics.record(MetricSample::unavailable(
        "reply_input_tokens",
        MetricUnit::Count,
        MetricScope::Run,
        "runtime.voice",
        "the conversation adapter does not export token counts",
    ));
    metrics.record(MetricSample::unavailable(
        "reply_output_tokens",
        MetricUnit::Count,
        MetricScope::Run,
        "runtime.voice",
        "the conversation adapter does not export token counts",
    ));
    metrics.record(MetricSample::unavailable(
        "reply_runtime_cpu_percent",
        MetricUnit::Percent,
        MetricScope::Process,
        "runtime.voice",
        "the current desktop sampler covers the host process, not its native reply child",
    ));
    metrics.record(MetricSample::unavailable(
        "reply_runtime_ram_bytes",
        MetricUnit::Bytes,
        MetricScope::Process,
        "runtime.voice",
        "the current desktop sampler covers the host process, not its native reply child",
    ));
    metrics.record(MetricSample::text(
        "voice_operation",
        kind,
        MetricUnit::Status,
        MetricScope::Run,
        "runtime.voice",
    ));
    if let Some(prompt) = &voice.prompt {
        metrics.record(MetricSample::text(
            "role_sha256",
            &prompt.sha256,
            MetricUnit::Status,
            MetricScope::Run,
            "runtime.voice",
        ));
    }
}

#[derive(Clone, Serialize)]
pub struct Inspection {
    pub history: Vec<ConversationMessage>,
    pub current_turn: Option<Turn>,
    pub stt: RuntimeStatus,
    pub reply: RuntimeStatus,
    pub role: Option<Role>,
    pub busy: bool,
}
pub enum TurnInput {
    Text(String),
    Audio(Vec<u8>),
}
impl Turn {
    fn test() -> Self {
        Self {
            turn_id: String::new(),
            status: Status::Generating,
            transcript: String::new(),
            approved_text: None,
            reply: String::new(),
            error: None,
            timings: BTreeMap::new(),
            role_sha256: None,
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
