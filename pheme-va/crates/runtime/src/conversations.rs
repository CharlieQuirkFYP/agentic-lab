//! Independent conversations share one runtime’s loaded compute.
use super::*;
use metrics::{MetricEvent, MetricsSubscriber, MetricsSubscription};
use va_core::chat::{ChatRun, ChatSnapshot, ChatTurn, MAX_CHAT_TURNS, TITLE_INSTRUCTION};

pub(super) struct Entry {
    pub voice: Arc<Voice>,
    admission: tokio::sync::Mutex<()>,
    metadata: Mutex<ChatSnapshot>,
    retained: Arc<Retained>,
    _subscription: MetricsSubscription,
}

struct Retained {
    id: String,
    runs: Mutex<Vec<ChatRun>>,
    journal: Mutex<VecDeque<(u64, MetricEvent)>>,
    next: std::sync::atomic::AtomicU64,
    changes: watch::Sender<u64>,
}

impl MetricsSubscriber for Retained {
    fn on_event(&self, event: &MetricEvent) {
        if !["stt", "reply", "title"]
            .iter()
            .any(|stage| event.run_id.starts_with(&format!("{stage}-{}-", self.id)))
        {
            return;
        }
        let mut runs = self.runs.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(run) = runs.iter_mut().find(|r| r.run_id == event.run_id) {
            run.observe(event.clone());
        } else {
            let stage = if event.run_id.starts_with("stt-") {
                "transcription"
            } else if event.run_id.starts_with("title-") {
                "title"
            } else {
                "reasoning"
            };
            let mut run = empty_run(&event.run_id, stage, event.timestamp_ms);
            run.observe(event.clone());
            runs.push(run);
        }
        let mut journal = self.journal.lock().unwrap_or_else(|p| p.into_inner());
        let cursor = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        journal.push_back((cursor, event.clone()));
        self.changes.send_modify(|r| *r = r.wrapping_add(1));
        if journal.len() > va_core::chat::MAX_RUN_EVENTS {
            journal.pop_front();
        }
    }
}

fn empty_run(id: &str, stage: &str, now: u64) -> ChatRun {
    ChatRun {
        run_id: id.into(),
        turn_id: None,
        stage: stage.into(),
        model_id: String::new(),
        backend: String::new(),
        revision: None,
        role_sha256: None,
        source: String::new(),
        status: "running".into(),
        started_at_ms: now,
        finished_at_ms: None,
        input: String::new(),
        output: String::new(),
        raw_transcript: String::new(),
        audio_duration_seconds: None,
        language: None,
        gate_decision: None,
        error: None,
        events: Vec::new(),
        summaries: Vec::new(),
        dropped_events: 0,
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// A separate async sampler stays responsive while native inference blocks.
pub(crate) struct Sampling(Option<tokio::task::JoinHandle<()>>);
impl Drop for Sampling {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}
pub(crate) fn sample_during(state: &AgentRuntime, context: &MetricsContext) -> Sampling {
    if !state.metrics_config.enabled || !state.metrics_config.resource_sampling {
        return Sampling(None);
    }
    let sampler = state.resource_sampler.clone();
    let context = context.clone();
    Sampling(Some(tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            sample_resources(&sampler, &context);
        }
    })))
}

fn child(parent: &Arc<Voice>, id: String) -> Arc<Voice> {
    Arc::new(Voice {
        inner: Mutex::new(Inner::default()),
        inference: parent.inference.clone(),
        model: parent.model.clone(),
        prompt: parent.prompt.clone(),
        stt: parent.stt.clone(),
        reply: parent.reply.clone(),
        limits: Limits {
            retained_turns: MAX_CHAT_TURNS,
            ..parent.limits.clone()
        },
        console: true,
        console_id: id,
        conversations: Mutex::new(BTreeMap::new()),
        changes: parent.changes.clone(),
    })
}

fn find(state: &AgentRuntime, id: &str) -> Result<Arc<Entry>, RuntimeError> {
    state
        .voice
        .conversations
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(id)
        .cloned()
        .ok_or_else(|| {
            error(
                ErrorKind::NotFound,
                "conversation_not_found",
                "Conversation is not retained; start a new conversation.",
            )
        })
}
fn scoped(mut state: AgentRuntime, entry: &Entry) -> AgentRuntime {
    state.voice = entry.voice.clone();
    state
}

pub fn new_conversation(state: &AgentRuntime) -> Result<ChatSnapshot, RuntimeError> {
    // Keep admission ordered with shutdown until the child is registered.
    let admission = state.voice.lock();
    if admission.closing {
        return Err(unavailable());
    }
    let id = format!("conversation_{}", crate::new_run_id());
    let now = now_ms();
    let retained = Arc::new(Retained {
        id: id.clone(),
        runs: Mutex::new(Vec::new()),
        journal: Mutex::new(VecDeque::new()),
        next: std::sync::atomic::AtomicU64::new(0),
        changes: state.voice.changes.clone(),
    });
    let metadata = ChatSnapshot {
        conversation_id: id.clone(),
        title: va_core::chat::placeholder_title(now),
        title_status: "placeholder".into(),
        status: "active".into(),
        started_at_ms: now,
        finished_at_ms: None,
        turns: Vec::new(),
        runs: Vec::new(),
        busy: false,
        metrics_enabled: state.metrics_config.enabled,
        resource_sampling_enabled: state.metrics_config.resource_sampling,
    };
    let entry = Arc::new(Entry {
        voice: child(&state.voice, id.clone()),
        admission: tokio::sync::Mutex::new(()),
        metadata: Mutex::new(metadata),
        _subscription: state.metrics_hub.subscribe(retained.clone()),
        retained,
    });
    let mut entries = state
        .voice
        .conversations
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if entries.len() >= 100 {
        let oldest = entries
            .iter()
            .filter(|(_, e)| {
                e.metadata.lock().unwrap_or_else(|p| p.into_inner()).status == "finished"
            })
            .min_by_key(|(_, e)| {
                e.metadata
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .started_at_ms
            })
            .map(|(id, _)| id.clone());
        if let Some(oldest) = oldest {
            entries.remove(&oldest);
        } else {
            return Err(error(
                ErrorKind::Capacity,
                "conversation_capacity",
                "Finish an existing conversation before starting another.",
            ));
        }
    }
    entries.insert(id, entry.clone());
    drop(entries);
    drop(admission);
    Ok(entry.snapshot(false))
}

impl Entry {
    pub(crate) fn snapshot(&self, events: bool) -> ChatSnapshot {
        let mut snapshot = self
            .metadata
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let inner = self.voice.lock();
        let mut runs = self.retained.runs.lock().unwrap_or_else(|p| p.into_inner());
        snapshot.turns = inner
            .records
            .iter()
            .map(|record| {
                let stt_id = record
                    .transcription_metrics
                    .as_ref()
                    .map(|m| m.run_id().to_owned());
                let reply_id = record
                    .turn
                    .approved_text
                    .as_ref()
                    .map(|_| record.metrics.run_id().to_owned());
                for (id, stage) in stt_id
                    .iter()
                    .map(|id| (id, "transcription"))
                    .chain(reply_id.iter().map(|id| (id, "reasoning")))
                {
                    let index = runs
                        .iter()
                        .position(|r| r.run_id == *id)
                        .unwrap_or_else(|| {
                            runs.push(empty_run(id, stage, record.started_at_ms));
                            runs.len() - 1
                        });
                    let run = &mut runs[index];
                    run.turn_id = Some(record.turn.turn_id.clone());
                    run.source = record.source.clone();
                    run.role_sha256 = record.turn.role_sha256.clone();
                    if stage == "transcription" {
                        run.model_id = self.voice.stt_status().name;
                        run.backend = self.voice.stt_status().name;
                        run.audio_duration_seconds = record.audio_duration_seconds;
                        run.started_at_ms = record.started_at_ms;
                        if let Some(result) = &record.transcription {
                            run.model_id = result.model_id.clone();
                            run.backend = result.stt_backend.clone();
                            run.revision = result.model_revision.clone();
                            run.output = result.text.clone();
                            run.raw_transcript = result.raw_text.clone();
                            run.language = result.language.clone();
                            run.gate_decision = Some(format!("{:?}", result.gate.decision));
                            run.status = if record.turn.status == Status::Cancelled {
                                "cancelled".into()
                            } else {
                                result.status.as_str().into()
                            };
                            run.finished_at_ms = record.transcription_finished_at_ms;
                        } else if record.turn.status.terminal() {
                            run.status = format!("{:?}", record.turn.status).to_lowercase();
                            run.finished_at_ms = record.transcription_finished_at_ms;
                        }
                    } else {
                        run.started_at_ms =
                            record.reply_started_at_ms.unwrap_or(record.started_at_ms);
                        run.model_id = self.voice.reply.name.clone();
                        run.backend = "llama.cpp worker".into();
                        run.input = record.turn.approved_text.clone().unwrap_or_default();
                        run.output = record.turn.reply.clone();
                        run.status = format!("{:?}", record.turn.status).to_lowercase();
                        run.finished_at_ms = record.reply_finished_at_ms;
                    }
                    run.error = record.turn.error.as_ref().map(|e| e.message.to_owned());
                }
                ChatTurn {
                    turn_id: record.turn.turn_id.clone(),
                    status: record.turn.status,
                    transcript: record.turn.transcript.clone(),
                    approved_text: record.turn.approved_text.clone(),
                    reply: record.turn.reply.clone(),
                    error: record.turn.error.as_ref().map(|e| e.message.into()),
                    timings: record.turn.timings.clone(),
                    source: record.source.clone(),
                    transcription_run: stt_id,
                    reasoning_run: reply_id,
                }
            })
            .collect();
        snapshot.busy =
            !inner.operations.is_empty() || self.voice.inference.available_permits() == 0;
        snapshot.runs = runs.clone();
        if !events {
            for run in &mut snapshot.runs {
                run.events.clear();
            }
        }
        snapshot
    }
}

pub fn snapshot(
    state: &AgentRuntime,
    id: &str,
    events: bool,
) -> Result<ChatSnapshot, RuntimeError> {
    Ok(find(state, id)?.snapshot(events))
}
#[derive(Serialize)]
pub struct MetricPage {
    pub events: Vec<MetricEvent>,
    pub next_cursor: u64,
    pub truncated: bool,
}
pub fn metrics(state: &AgentRuntime, id: &str, after: u64) -> Result<MetricPage, RuntimeError> {
    let entry = find(state, id)?;
    let journal = entry
        .retained
        .journal
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let items = journal
        .iter()
        .filter(|(n, _)| *n > after)
        .take(256)
        .collect::<Vec<_>>();
    Ok(MetricPage {
        events: items.iter().map(|(_, e)| e.clone()).collect(),
        next_cursor: items.last().map(|(n, _)| *n).unwrap_or(after),
        truncated: journal
            .front()
            .is_some_and(|(n, _)| after.saturating_add(1) < *n),
    })
}
pub fn run(state: &AgentRuntime, id: &str, run_id: &str) -> Result<ChatRun, RuntimeError> {
    snapshot(state, id, true)?
        .runs
        .into_iter()
        .find(|r| r.run_id == run_id)
        .ok_or_else(missing)
}

pub async fn start_turn(
    state: &AgentRuntime,
    id: &str,
    input: TurnInput,
    mut options: RequestOptions,
    confirmed: bool,
) -> Result<events::TurnStream, RuntimeError> {
    let entry = find(state, id)?;
    let _admission = entry.admission.lock().await;
    if entry
        .metadata
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .status
        != "active"
    {
        return Err(error(
            ErrorKind::Conflict,
            "conversation_closed",
            "Start a new conversation.",
        ));
    }
    let retry = options.idempotency_key.as_ref().is_some_and(|key| {
        entry
            .voice
            .lock()
            .retries
            .iter()
            .any(|(old, _, _)| old == key)
    });
    if entry.voice.lock().records.len() >= MAX_CHAT_TURNS && !retry {
        return Err(error(
            ErrorKind::Conflict,
            "turn_capacity",
            "Finish this conversation and start a new one.",
        ));
    }
    let run_id = format!("stt-{id}-{}", crate::new_run_id());
    options.run_id = Some(run_id);
    super::create(scoped(state.clone(), &entry), input, options, confirmed).await
}
pub fn turn(state: &AgentRuntime, id: &str, turn: &str) -> Result<Turn, RuntimeError> {
    super::recover(&scoped(state.clone(), find(state, id)?.as_ref()), turn)
}
pub fn approve(
    state: &AgentRuntime,
    id: &str,
    turn: &str,
    text: String,
) -> Result<Turn, RuntimeError> {
    super::submit(
        &scoped(state.clone(), find(state, id)?.as_ref()),
        turn,
        text,
    )
}
pub fn stop_turn(state: &AgentRuntime, id: &str, turn: &str) -> Result<Turn, RuntimeError> {
    super::cancel(&scoped(state.clone(), find(state, id)?.as_ref()), turn)
}

pub fn finish(state: &AgentRuntime, id: &str) -> Result<ChatSnapshot, RuntimeError> {
    let state = state.clone();
    let id = id.to_owned();
    let entry = find(&state, &id)?;
    let _admission = entry.admission.try_lock().map_err(|_| busy())?;
    if entry.voice.lock().active() {
        return Err(busy());
    }
    let maintenance = {
        let mut meta = entry.metadata.lock().unwrap_or_else(|p| p.into_inner());
        if meta.status != "active" {
            drop(meta);
            return Ok(entry.snapshot(false));
        }
        // Register before spawning, so immediate shutdown also settles metadata.
        let mut inner = entry.voice.lock();
        if inner.closing {
            return Err(unavailable());
        }
        let id = crate::new_run_id();
        let op = Operation::new();
        inner.operations.insert(id.clone(), op.clone());
        let guard = OperationGuard {
            voice: entry.voice.clone(),
            id,
            op,
        };
        meta.status = "finishing".into();
        meta.title_status = "generating".into();
        guard
    };
    let finished = entry.clone();
    tokio::spawn(async move {
        let maintenance_op = maintenance.op.clone();
        let _maintenance = maintenance;
        let snapshot = finished.snapshot(false);
        let mut text = String::new();
        let budget = state
            .voice
            .limits
            .conversation
            .max_input_chars
            .saturating_sub(TITLE_INSTRUCTION.chars().count() + 100);
        let completed = snapshot
            .turns
            .iter()
            .filter(|t| t.status == Status::Completed)
            .collect::<Vec<_>>();
        let per_message = budget.saturating_sub(completed.len() * 20) / completed.len().max(1) / 2;
        let mut omitted = 0usize;
        for turn in &completed {
            let clip = |s: &str| {
                let chars = s.chars().collect::<Vec<_>>();
                if chars.len() <= per_message {
                    return s.to_owned();
                }
                let half = per_message.saturating_sub(3) / 2;
                format!(
                    "{}...{}",
                    chars[..half].iter().collect::<String>(),
                    chars[chars.len() - half..].iter().collect::<String>()
                )
            };
            let user = turn.approved_text.as_deref().unwrap_or_default();
            let question = clip(user);
            let reply = clip(&turn.reply);
            omitted += user
                .chars()
                .count()
                .saturating_sub(question.chars().count())
                + turn
                    .reply
                    .chars()
                    .count()
                    .saturating_sub(reply.chars().count());
            text.push_str(&format!("User: {question}\nAssistant: {reply}\n"));
        }
        let result = async {
            if text.trim().is_empty() {
                return Err(Failure::new(
                    "no_completed_turns",
                    "No completed exchanges to summarize.",
                ));
            }
            let mut title_voice = child(&finished.voice, id.clone());
            let voice = Arc::get_mut(&mut title_voice).expect("new child");
            voice.limits.conversation.max_output_tokens =
                32.min(voice.limits.conversation.context_size.saturating_sub(1));
            voice.limits.conversation.max_output_chars = 256;
            voice.limits.generation_timeout = Duration::from_secs(20);
            let messages =
                build_messages(TITLE_INSTRUCTION, &[], &text, &voice.limits.conversation).map_err(
                    |_| Failure::new("title_input", "Title input exceeds the context budget."),
                )?;
            let lease = finished.voice.begin_inference().map_err(|_| {
                Failure::new("busy", "Shared inference is busy; placeholder retained.")
            })?;
            let run_id = format!("title-{id}-{}", crate::new_run_id());
            let options = RequestOptions {
                run_id: Some(run_id.clone()),
                ..Default::default()
            };
            let metrics = request_metrics(&state, &options, false);
            let mut run = empty_run(&run_id, "title", now_ms());
            run.model_id = title_voice.reply.name.clone();
            run.backend = "llama.cpp worker".into();
            run.source = "conversation summary".into();
            run.input = text.clone();
            finished
                .retained
                .runs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(run);
            label_metrics(&metrics, "conversation_title", &title_voice);
            for (name, count) in [
                ("title_completed_exchanges", completed.len()),
                ("title_omitted_characters", omitted),
            ] {
                metrics.record(MetricSample::integer(
                    name,
                    count as u64,
                    MetricUnit::Count,
                    MetricScope::Run,
                    "runtime.conversation",
                ));
            }
            let Lease {
                _permit: permit,
                _guard: guard,
            } = lease;
            let _guard = guard;
            let mut title_state = state.clone();
            title_state.voice = title_voice;
            let result = generate(
                &title_state,
                None,
                &maintenance_op,
                Job { messages, permit },
                metrics,
                None,
            )
            .await
            .and_then(|title| {
                let title = title.trim().trim_matches('"');
                if title.is_empty() || title.contains(['\n', '\r']) || title.chars().count() > 64 {
                    Err(Failure::new(
                        "invalid_title",
                        "Title was not a short single line.",
                    ))
                } else {
                    Ok(title.to_owned())
                }
            });
            if let Some(run) = finished
                .retained
                .runs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter_mut()
                .find(|r| r.run_id == run_id)
            {
                run.finished_at_ms = Some(now_ms());
                run.status = if result.is_ok() {
                    "completed"
                } else {
                    "failed"
                }
                .into();
                match &result {
                    Ok(t) => run.output = t.clone(),
                    Err(e) => run.error = Some(e.message.into()),
                }
            }
            result
        }
        .await;
        let mut meta = finished.metadata.lock().unwrap_or_else(|p| p.into_inner());
        match result {
            Ok(title) => {
                meta.title = title;
                meta.title_status = "completed".into();
            }
            Err(_) => meta.title_status = "failed".into(),
        }
        meta.status = "finished".into();
        meta.finished_at_ms = Some(now_ms());
        finished
            .voice
            .changes
            .send_modify(|r| *r = r.wrapping_add(1));
    });
    Ok(entry.snapshot(false))
}

pub fn inspect(state: &AgentRuntime, id: &str) -> Result<Inspection, RuntimeError> {
    Ok(super::inspect(&scoped(
        state.clone(),
        find(state, id)?.as_ref(),
    )))
}
