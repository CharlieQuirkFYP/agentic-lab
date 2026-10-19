use super::*;
use metrics::{MetricsConfig, MetricsHub};
use va_core::{
    ConversationRole, EngineError, NormalizedAudio, RawTranscription, Transcriber,
    TranscriptionOptions,
};
#[derive(Default)]
struct Control {
    calls: Mutex<Vec<Vec<ConversationMessage>>>,
    hold: AtomicBool,
    cancelled: AtomicBool,
    fail: AtomicBool,
    unavailable: AtomicBool,
    empty: AtomicBool,
    flood: AtomicBool,
    stt_hold: AtomicBool,
    stt_entered: AtomicBool,
}
struct FakeModel(Arc<Control>);
impl ConversationModel for FakeModel {
    fn name(&self) -> &str {
        "fake-reply"
    }
    fn is_ready(&self) -> bool {
        !self.0.unavailable.load(Ordering::Acquire)
    }
    fn respond_stream(
        &mut self,
        messages: &[ConversationMessage],
        _: &ConversationConfig,
        cancelled: &AtomicBool,
        emit: &mut dyn FnMut(&str),
    ) -> Result<String, ConversationError> {
        self.0.calls.lock().unwrap().push(messages.to_vec());
        emit("你好 — ");
        while self.0.hold.load(Ordering::Acquire) {
            if cancelled.load(Ordering::Acquire) {
                self.0.cancelled.store(true, Ordering::Release);
                // Deliberately stale adapter callback: the host must ignore it.
                emit("stale");
                std::thread::sleep(Duration::from_millis(20));
                return Err(ConversationError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if cancelled.load(Ordering::Acquire) {
            self.0.cancelled.store(true, Ordering::Release);
            return Err(ConversationError::Cancelled);
        }
        if self.0.fail.load(Ordering::Acquire) {
            return Err(ConversationError::Backend("private model details".into()));
        }
        if self.0.empty.load(Ordering::Acquire) {
            return Ok(String::new());
        }
        if self.0.flood.load(Ordering::Acquire) {
            for _ in 0..300 {
                emit("好");
            }
            return Ok("好".repeat(300));
        }
        emit("café.");
        Ok("你好 — café.".into())
    }
}
struct FakeTranscriber(Arc<Control>);
impl Transcriber for FakeTranscriber {
    fn name(&self) -> &str {
        "fake-stt"
    }
    fn is_ready(&self) -> bool {
        true
    }
    fn transcribe(
        &mut self,
        _: &NormalizedAudio,
        _: &TranscriptionOptions,
    ) -> Result<RawTranscription, EngineError> {
        self.0.stt_entered.store(true, Ordering::Release);
        while self.0.stt_hold.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(RawTranscription::text("original transcript"))
    }
}

fn wav(silent: bool) -> Vec<u8> {
    let data_len = 32_000u32;
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + data_len).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(16_000u32.to_le_bytes());
    bytes.extend(32_000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data_len.to_le_bytes());
    for i in 0..16_000 {
        bytes.extend(
            (if silent {
                0i16
            } else if i % 20 < 10 {
                5000i16
            } else {
                -5000i16
            })
            .to_le_bytes(),
        );
    }
    bytes
}

fn setup(audio: bool, reply: bool, limits: Limits) -> (AgentRuntime, Arc<Control>) {
    let control = Arc::new(Control::default());
    let engine = audio.then(|| Engine::new(FakeTranscriber(control.clone())));
    let model = reply.then(|| Box::new(FakeModel(control.clone())) as Box<dyn ConversationModel>);
    let prompt = reply.then(|| LoadedPrompt {
        path: "fixture-role.txt".into(),
        text: "incident role".into(),
        sha256: "f".repeat(64),
    });
    (
        AgentRuntime::new(
            engine,
            model,
            prompt,
            limits,
            MetricsConfig::enabled(),
            Arc::new(MetricsHub::new()),
        ),
        control,
    )
}
async fn wait(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("runtime did not settle");
}
async fn send(agent: &AgentRuntime, id: &str, text: &str, key: &str) -> events::TurnStream {
    conversations::start_turn(
        agent,
        id,
        TurnInput::Text(text.into()),
        RequestOptions {
            idempotency_key: Some(key.into()),
            ..Default::default()
        },
        true,
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn reply_only_typed_send_is_atomic_exact_and_has_no_transcription_or_review_metrics() {
    let (agent, control) = setup(false, true, Limits::default());
    let id = conversations::new_conversation(&agent)
        .unwrap()
        .conversation_id;
    let text = "  Smoke downstairs — 你好\nEveryone is outside.  ";
    let stream = send(&agent, &id, text, "typed-1").await;
    assert_eq!(stream.turn.status, Status::Generating);
    assert_eq!(stream.turn.approved_text.as_deref(), Some(text));
    drop(stream);
    wait(|| agent.voice.idle()).await;
    let snapshot = conversations::snapshot(&agent, &id, true).unwrap();
    assert_eq!(snapshot.turns[0].status, Status::Completed);
    assert!(snapshot.turns[0].transcription_run.is_none());
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].stage, "reasoning");
    assert!(snapshot.runs[0].audio_duration_seconds.is_none());
    assert!(!snapshot.runs[0]
        .summaries
        .iter()
        .any(|m| m.name == "human_review_ms"));
    assert_eq!(control.calls.lock().unwrap()[0][1].content, text);
    let duplicate = send(&agent, &id, text, "typed-1").await;
    assert_eq!(duplicate.turn.turn_id, snapshot.turns[0].turn_id);
    assert_eq!(control.calls.lock().unwrap().len(), 1);
    let conflict = conversations::start_turn(
        &agent,
        &id,
        TurnInput::Text("different".into()),
        RequestOptions {
            idempotency_key: Some("typed-1".into()),
            ..Default::default()
        },
        true,
    )
    .await
    .err()
    .unwrap();
    assert_eq!(conflict.1.code, "idempotency_conflict");
    agent.shutdown().await;
}
#[tokio::test]
async fn audio_pauses_for_review_and_typed_follow_up_uses_only_confirmed_successful_history() {
    let (agent, control) = setup(true, true, Limits::default());
    let id = conversations::new_conversation(&agent)
        .unwrap()
        .conversation_id;
    let stream = conversations::start_turn(
        &agent,
        &id,
        TurnInput::Audio(wav(false)),
        RequestOptions {
            audio_source: "fixture.wav".into(),
            ..Default::default()
        },
        false,
    )
    .await
    .unwrap();
    let turn = stream.turn.turn_id.clone();
    drop(stream);
    wait(|| conversations::turn(&agent, &id, &turn).unwrap().status == Status::AwaitingReview)
        .await;
    assert!(control.calls.lock().unwrap().is_empty());
    conversations::approve(&agent, &id, &turn, "corrected incident — café".into()).unwrap();
    wait(|| agent.voice.idle()).await;
    drop(send(&agent, &id, "Everyone is safe.", "follow-up").await);
    wait(|| agent.voice.idle()).await;
    {
        let calls = control.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].len(), 4);
        assert_eq!(calls[1][1].role, ConversationRole::User);
        assert_eq!(calls[1][1].content, "corrected incident — café");
        assert_eq!(calls[1][2].role, ConversationRole::Assistant);
        assert_eq!(calls[1][3].content, "Everyone is safe.");
    }
    let snapshot = conversations::snapshot(&agent, &id, true).unwrap();
    assert_eq!(snapshot.runs.len(), 3);
    assert_eq!(
        snapshot
            .runs
            .iter()
            .filter(|r| r.stage == "transcription")
            .count(),
        1
    );
    assert_eq!(snapshot.runs[0].raw_transcript, "original transcript");
    agent.shutdown().await;
}
#[tokio::test]
async fn transcription_still_reviews_when_reply_is_unconfigured() {
    let (agent, _) = setup(true, false, Limits::default());
    let id = conversations::new_conversation(&agent)
        .unwrap()
        .conversation_id;
    let stream = conversations::start_turn(
        &agent,
        &id,
        TurnInput::Audio(wav(false)),
        RequestOptions::default(),
        false,
    )
    .await
    .unwrap();
    let turn = stream.turn.turn_id.clone();
    drop(stream);
    wait(|| conversations::turn(&agent, &id, &turn).unwrap().status == Status::AwaitingReview)
        .await;
    assert_eq!(
        conversations::approve(&agent, &id, &turn, "confirm".into())
            .unwrap_err()
            .1
            .code,
        "runtime_unavailable"
    );
    conversations::stop_turn(&agent, &id, &turn).unwrap();
    wait(|| agent.voice.idle()).await;
    agent.shutdown().await;
}
#[tokio::test]
async fn invalid_direct_text_does_not_create_a_turn_or_claim_its_retry_key() {
    let (agent, control) = setup(false, true, Limits::default());
    let id = conversations::new_conversation(&agent)
        .unwrap()
        .conversation_id;
    for text in ["", "  ", "bad\u{0001}"] {
        assert!(conversations::start_turn(
            &agent,
            &id,
            TurnInput::Text(text.into()),
            RequestOptions {
                idempotency_key: Some("key".into()),
                ..Default::default()
            },
            true
        )
        .await
        .is_err());
    }
    assert!(conversations::snapshot(&agent, &id, false)
        .unwrap()
        .turns
        .is_empty());
    assert!(control.calls.lock().unwrap().is_empty());
    drop(send(&agent, &id, "valid", "key").await);
    wait(|| agent.voice.idle()).await;
    agent.shutdown().await;
}
#[tokio::test]
async fn cancellation_settles_before_reuse_and_never_commits_partial_or_stale_output() {
    let (agent, control) = setup(false, true, Limits::default());
    control.hold.store(true, Ordering::Release);
    let id = conversations::new_conversation(&agent)
        .unwrap()
        .conversation_id;
    let stream = send(&agent, &id, "first", "first").await;
    let turn = stream.turn.turn_id.clone();
    drop(stream);
    wait(|| !control.calls.lock().unwrap().is_empty()).await;
    conversations::stop_turn(&agent, &id, &turn).unwrap();
    assert!(conversations::start_turn(
        &agent,
        &id,
        TurnInput::Text("next".into()),
        RequestOptions::default(),
        true
    )
    .await
    .is_err());
    wait(|| agent.voice.idle()).await;
    assert!(conversations::inspect(&agent, &id)
        .unwrap()
        .history
        .is_empty());
    let old = conversations::turn(&agent, &id, &turn).unwrap();
    assert!(!old.reply.contains("stale"));
    assert_eq!(old.status, Status::Cancelled);
    control.hold.store(false, Ordering::Release);
    drop(send(&agent, &id, "next", "next").await);
    wait(|| agent.voice.idle()).await;
    assert_eq!(control.calls.lock().unwrap()[1].len(), 2);
    agent.shutdown().await;
}
#[tokio::test]
async fn native_stt_timeout_retains_busy_and_settlement_timestamps() {
    let (agent, control) = setup(
        true,
        true,
        Limits {
            transcription_timeout: Duration::from_millis(10),
            ..Default::default()
        },
    );
    control.stt_hold.store(true, Ordering::Release);
    let id = conversations::new_conversation(&agent)
        .unwrap()
        .conversation_id;
    let stream = conversations::start_turn(
        &agent,
        &id,
        TurnInput::Audio(wav(false)),
        RequestOptions::default(),
        false,
    )
    .await
    .unwrap();
    let turn = stream.turn.turn_id.clone();
    drop(stream);
    wait(|| conversations::turn(&agent, &id, &turn).unwrap().status == Status::Failed).await;
    let snapshot = conversations::snapshot(&agent, &id, false).unwrap();
    assert!(snapshot.busy);
    assert!(snapshot.runs[0].finished_at_ms.is_none());
    assert!(conversations::start_turn(
        &agent,
        &id,
        TurnInput::Text("next".into()),
        RequestOptions::default(),
        true
    )
    .await
    .is_err());
    control.stt_hold.store(false, Ordering::Release);
    wait(|| agent.voice.idle()).await;
    assert!(conversations::snapshot(&agent, &id, false).unwrap().runs[0]
        .finished_at_ms
        .is_some());
    agent.shutdown().await;
}
#[tokio::test]
async fn dropped_or_lagging_event_consumers_recover_without_repeating_inference() {
    let (agent, control) = setup(false, true, Limits::default());
    control.flood.store(true, Ordering::Release);
    let id = conversations::new_conversation(&agent)
        .unwrap()
        .conversation_id;
    let _unread = send(&agent, &id, "question", "key").await;
    wait(|| agent.voice.idle()).await;
    let snapshot = conversations::snapshot(&agent, &id, false).unwrap();
    assert_eq!(snapshot.turns[0].reply, "好".repeat(300));
    assert_eq!(snapshot.turns[0].status, Status::Completed);
    assert_eq!(control.calls.lock().unwrap().len(), 1);
    agent.shutdown().await;
}
#[tokio::test]
async fn failed_answers_are_retained_but_excluded_from_successful_context() {
    let (agent, control) = setup(false, true, Limits::default());
    control.fail.store(true, Ordering::Release);
    let id = conversations::new_conversation(&agent)
        .unwrap()
        .conversation_id;
    drop(send(&agent, &id, "failed", "failed").await);
    wait(|| agent.voice.idle()).await;
    assert!(conversations::inspect(&agent, &id)
        .unwrap()
        .history
        .is_empty());
    control.fail.store(false, Ordering::Release);
    drop(send(&agent, &id, "good", "good").await);
    wait(|| agent.voice.idle()).await;
    assert_eq!(control.calls.lock().unwrap()[1].len(), 2);
    agent.shutdown().await;
}
#[tokio::test]
async fn isolated_tests_share_compute_without_conversation_history() {
    let (agent, control) = setup(false, true, Limits::default());
    let id = conversations::new_conversation(&agent)
        .unwrap()
        .conversation_id;
    drop(send(&agent, &id, "chat", "chat").await);
    wait(|| agent.voice.idle()).await;
    let mut test =
        super::test_reply(agent.clone(), "isolated".into(), RequestOptions::default()).unwrap();
    while test.receiver.recv().await.is_some() {}
    wait(|| agent.voice.idle()).await;
    assert_eq!(control.calls.lock().unwrap()[1].len(), 2);
    assert_eq!(
        conversations::snapshot(&agent, &id, false)
            .unwrap()
            .turns
            .len(),
        1
    );
    agent.shutdown().await;
}
#[tokio::test]
async fn finishing_generates_a_separate_title_run_and_shutdown_rejects_new_conversations() {
    let (agent, control) = setup(false, true, Limits::default());
    let id = conversations::new_conversation(&agent)
        .unwrap()
        .conversation_id;
    drop(send(&agent, &id, "smoke downstairs", "key").await);
    wait(|| agent.voice.idle()).await;
    conversations::finish(&agent, &id).unwrap();
    wait(|| conversations::snapshot(&agent, &id, false).unwrap().status == "finished").await;
    let snapshot = conversations::snapshot(&agent, &id, false).unwrap();
    assert_eq!(snapshot.title_status, "completed");
    assert_eq!(snapshot.runs.len(), 2);
    assert_eq!(snapshot.runs[1].stage, "title");
    assert_eq!(control.calls.lock().unwrap().len(), 2);
    agent.shutdown().await;
    assert!(conversations::new_conversation(&agent).is_err());
}
#[tokio::test]
async fn bounded_web_retention_keeps_expired_retry_keys_from_regenerating() {
    let (agent, control) = setup(
        false,
        true,
        Limits {
            retained_turns: 2,
            ..Default::default()
        },
    );
    for i in 0..3 {
        let stream = create(
            agent.clone(),
            TurnInput::Text(format!("q{i}")),
            RequestOptions {
                idempotency_key: (i == 0).then(|| "old".into()),
                ..Default::default()
            },
            false,
        )
        .await
        .unwrap();
        cancel(&agent, &stream.turn.turn_id).unwrap();
        drop(stream);
        wait(|| agent.voice.idle()).await;
    }
    assert_eq!(agent.voice.lock().records.len(), 2);
    assert_eq!(agent.voice.lock().retries.len(), 1);
    let error = create(
        agent.clone(),
        TurnInput::Text("q0".into()),
        RequestOptions {
            idempotency_key: Some("old".into()),
            ..Default::default()
        },
        false,
    )
    .await
    .err()
    .unwrap();
    assert_eq!(error.1.code, "turn_expired");
    assert!(control.calls.lock().unwrap().is_empty());
    agent.shutdown().await;
}
#[tokio::test]
async fn failed_stt_replacement_retains_engine_and_busy_replacement_never_loads() {
    let (agent, control) = setup(true, true, Limits::default());
    assert!(agent
        .load_engine(|| anyhow::bail!("failed candidate"))
        .is_err());
    assert!(agent.voice.stt_status().ready);
    control.hold.store(true, Ordering::Release);
    let id = conversations::new_conversation(&agent)
        .unwrap()
        .conversation_id;
    drop(send(&agent, &id, "busy", "key").await);
    wait(|| !control.calls.lock().unwrap().is_empty()).await;
    let loaded = AtomicBool::new(false);
    assert!(agent
        .load_engine(|| {
            loaded.store(true, Ordering::Release);
            Ok(Engine::new(FakeTranscriber(control.clone())))
        })
        .is_err());
    assert!(!loaded.load(Ordering::Acquire));
    agent.shutdown().await;
    assert!(control.cancelled.load(Ordering::Acquire));
}

#[tokio::test]
async fn standalone_timeout_retains_native_lease_and_shutdown_waits_for_settlement() {
    let (agent, control) = setup(
        true,
        false,
        Limits {
            transcription_timeout: Duration::from_millis(25),
            ..Default::default()
        },
    );
    control.stt_hold.store(true, Ordering::Release);
    let context = crate::request_metrics(&agent, &RequestOptions::default(), false);
    let result = agent
        .transcribe(
            va_core::AudioBuffer::from_wav(&wav(false)).unwrap(),
            context,
        )
        .await;
    let busy = !agent.voice.idle();
    let competing = agent.voice.begin_inference().err();
    let closing_agent = agent.clone();
    let shutdown = tokio::spawn(async move { closing_agent.shutdown().await });
    tokio::time::sleep(Duration::from_millis(10)).await;
    let waited = !shutdown.is_finished();
    control.stt_hold.store(false, Ordering::Release);
    shutdown.await.unwrap();
    assert_eq!(result.unwrap_err().1.code, "transcription_timeout");
    assert!(busy && waited);
    assert_eq!(competing.unwrap().1.code, "busy");
    assert!(agent.voice.idle());
}

#[tokio::test]
async fn missing_stt_returns_unavailable_without_taking_inference_lease() {
    let (agent, _) = setup(false, true, Limits::default());
    let context = crate::request_metrics(&agent, &RequestOptions::default(), false);
    let error = agent
        .transcribe(
            va_core::AudioBuffer::from_wav(&wav(false)).unwrap(),
            context,
        )
        .await
        .unwrap_err();
    assert!(matches!(error.0, ErrorKind::Unavailable));
    assert!(agent.voice.idle());
    agent.shutdown().await;
}

#[tokio::test]
async fn immediate_shutdown_after_finish_settles_title_metadata() {
    let (agent, _) = setup(false, true, Limits::default());
    let id = conversations::new_conversation(&agent)
        .unwrap()
        .conversation_id;
    drop(send(&agent, &id, "smoke downstairs", "key").await);
    wait(|| agent.voice.idle()).await;
    conversations::finish(&agent, &id).unwrap();
    assert!(
        !agent.voice.idle(),
        "title maintenance must register synchronously"
    );
    agent.shutdown().await;
    let snapshot = conversations::snapshot(&agent, &id, false).unwrap();
    assert_eq!(snapshot.status, "finished");
    assert!(!snapshot.busy);
}

#[tokio::test]
async fn dropping_direct_isolated_test_stream_cancels_even_without_more_output() {
    let (agent, control) = setup(false, true, Limits::default());
    control.hold.store(true, Ordering::Release);
    let stream =
        super::test_reply(agent.clone(), "isolated".into(), RequestOptions::default()).unwrap();
    wait(|| !control.calls.lock().unwrap().is_empty()).await;
    drop(stream);
    wait(|| agent.voice.idle()).await;
    assert!(control.cancelled.load(Ordering::Acquire));
    assert!(super::inspect(&agent).history.is_empty());
    agent.shutdown().await;
}
