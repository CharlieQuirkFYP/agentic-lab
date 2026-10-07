use super::*;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use http_body_util::BodyExt;
use metrics::{MetricsBatcher, MetricsConfig, MetricsHub, SysinfoResourceSampler};
use tower::ServiceExt;
use va_core::{EngineError, NormalizedAudio, RawTranscription, Transcriber, TranscriptionOptions};

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

fn setup(limits: Limits) -> (AppState, Arc<Control>) {
    let control = Arc::new(Control::default());
    let engine = Engine::new(FakeTranscriber(control.clone()));
    let hub = Arc::new(MetricsHub::new());
    let batcher = Arc::new(MetricsBatcher::new());
    let subscription = Arc::new(hub.subscribe(batcher.clone()));
    let prompt = LoadedPrompt {
        path: "incident-role.txt".into(),
        text: "incident role".into(),
        sha256: "f".repeat(64),
    };
    let voice = Voice::new(
        Some(&engine),
        Some(Box::new(FakeModel(control.clone()))),
        Some(prompt),
        limits,
    );
    (
        AppState {
            engine: Some(Arc::new(Mutex::new(engine))),
            voice,
            metrics_hub: hub,
            metrics_batcher: batcher,
            _metrics_subscription: subscription,
            metrics_config: MetricsConfig::enabled(),
            resource_sampler: Arc::new(Mutex::new(SysinfoResourceSampler::new())),
        },
        control,
    )
}
async fn request(
    state: &AppState,
    method: &str,
    path: &str,
    content_type: &str,
    body: impl Into<Body>,
    key: Option<&str>,
) -> Response {
    let mut builder = Request::builder().method(method).uri(path);
    if !content_type.is_empty() {
        builder = builder.header("content-type", content_type);
    }
    if let Some(key) = key {
        builder = builder.header("idempotency-key", key);
    }
    crate::router(state.clone())
        .oneshot(builder.body(body.into()).unwrap())
        .await
        .unwrap()
}
async fn command(state: &AppState, path: &str, text: &str) -> Response {
    request(
        state,
        "POST",
        path,
        "application/json",
        json!({"text": text}).to_string(),
        None,
    )
    .await
}
async fn value(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}
async fn snapshot(state: &AppState) -> Value {
    value(request(state, "GET", "/v1/voice/inspect", "", Body::empty(), None).await).await
}
fn current_id(state: &AppState) -> String {
    state.voice.lock().current.clone().unwrap()
}
async fn stream_text(response: Response) -> String {
    let bytes = tokio::time::timeout(
        Duration::from_secs(3),
        to_bytes(response.into_body(), 1024 * 1024),
    )
    .await
    .unwrap()
    .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}
async fn wait_until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("operation failed to settle");
}
async fn wait_idle(state: &AppState) {
    wait_until(|| state.voice.lock().operations.is_empty()).await;
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

#[tokio::test]
async fn worker_failure_is_reflected_in_readiness_and_rejects_new_inference() {
    let (state, control) = setup(Limits::default());
    assert_eq!(snapshot(&state).await["reply"]["ready"], true);
    control.unavailable.store(true, Ordering::Release);
    assert_eq!(snapshot(&state).await["reply"]["ready"], false);
    let response = command(&state, "/v1/voice/turns", "new question").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let response = command(&state, "/v1/voice/test/reply", "test").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(control.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn edited_question_reaches_original_stream_exactly_once_and_commits_history() {
    let (state, control) = setup(Limits::default());
    let response = command(&state, "/v1/voice/turns", "uncorrected").await;
    assert_eq!(response.status(), StatusCode::OK);
    let id = current_id(&state);
    let inspect = snapshot(&state).await;
    assert_eq!(inspect["current_turn"]["status"], "awaiting_review");
    assert!(control.calls.lock().unwrap().is_empty());
    let text = "  No fire — café entrance.\n你好 ";
    let ack = command(&state, &format!("/v1/voice/turns/{id}/submit"), text).await;
    assert_eq!(ack.status(), StatusCode::OK);
    assert_eq!(value(ack).await["approved_text"], text);
    let stream = stream_text(response).await;
    for event in [
        "turn.created",
        "transcript.ready",
        "question.approved",
        "reply.started",
        "reply.delta",
        "reply.completed",
    ] {
        assert!(
            stream.contains(&format!("event: {event}")),
            "missing {event}"
        );
    }
    assert!(stream.contains(&serde_json::to_string(text).unwrap()));
    wait_idle(&state).await;
    {
        let calls = control.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0].role, ConversationRole::System);
        assert_eq!(calls[0][1].content, text);
    }
    let inspect = snapshot(&state).await;
    assert_eq!(inspect["history"].as_array().unwrap().len(), 2);
    assert_eq!(inspect["history"][0]["content"], text);
    assert_eq!(inspect["current_turn"]["reply"], "你好 — café.");
    assert_eq!(inspect["role"]["sha256"], "f".repeat(64));
    let repeat = command(&state, &format!("/v1/voice/turns/{id}/submit"), text).await;
    assert_eq!(value(repeat).await["status"], "completed");
    assert_eq!(
        command(&state, &format!("/v1/voice/turns/{id}/submit"), "different")
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let batches = state.metrics_batcher.drain();
    let metrics = serde_json::to_string(&batches).unwrap();
    assert!(
        metrics.contains("human_review_ms")
            && metrics.contains("reply_generation_ms")
            && metrics.contains("role_sha256")
    );
    assert!(!metrics.contains("café") && !metrics.contains("uncorrected"));
}

#[tokio::test]
async fn isolated_reply_uses_role_but_no_web_history_and_review_releases_compute() {
    let (state, control) = setup(Limits::default());
    let response = command(&state, "/v1/voice/turns", "web first").await;
    let id = current_id(&state);
    command(&state, &format!("/v1/voice/turns/{id}/submit"), "web first").await;
    stream_text(response).await;
    wait_idle(&state).await;
    let pending = command(&state, "/v1/voice/turns", "web second").await;
    let before = snapshot(&state).await;
    let test = command(&state, "/v1/voice/test/reply", "private test").await;
    assert_eq!(test.status(), StatusCode::OK);
    let test_stream = stream_text(test).await;
    assert!(!test_stream.contains("turn_id"));
    assert!(test_stream.contains("reply.completed"));
    wait_until(|| state.voice.inference.available_permits() == 1).await;
    assert_eq!(snapshot(&state).await, before);
    {
        let calls = control.calls.lock().unwrap();
        assert_eq!(calls[1].len(), 2);
        assert_eq!(calls[1][0].content, "incident role");
        assert_eq!(calls[1][1].content, "private test");
    }
    let id = current_id(&state);
    command(
        &state,
        &format!("/v1/voice/turns/{id}/submit"),
        "web second",
    )
    .await;
    stream_text(pending).await;
    wait_idle(&state).await;
    let calls = control.calls.lock().unwrap();
    assert_eq!(calls[2].len(), 4);
    assert!(calls[2]
        .iter()
        .all(|message| !message.content.contains("private test")));
}

#[tokio::test]
async fn busy_submit_stays_reviewable_and_competing_submits_freeze_first_question() {
    let (state, control) = setup(Limits::default());
    let web = command(&state, "/v1/voice/turns", "pending").await;
    let id = current_id(&state);
    assert_eq!(
        command(&state, "/v1/voice/turns", "overlap").await.status(),
        StatusCode::CONFLICT
    );
    control.hold.store(true, Ordering::Release);
    let test = command(&state, "/v1/voice/test/reply", "test").await;
    wait_until(|| !control.calls.lock().unwrap().is_empty()).await;
    let submit = command(&state, &format!("/v1/voice/turns/{id}/submit"), "edited").await;
    assert_eq!(submit.status(), StatusCode::CONFLICT);
    assert_eq!(
        snapshot(&state).await["current_turn"]["approved_text"],
        Value::Null
    );
    assert_eq!(
        request(
            &state,
            "POST",
            "/v1/transcribe",
            "audio/wav",
            wav(false),
            None
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    drop(test);
    wait_until(|| control.cancelled.load(Ordering::Acquire)).await;
    wait_until(|| state.voice.inference.available_permits() == 1).await;
    control.hold.store(false, Ordering::Release);
    let path = format!("/v1/voice/turns/{id}/submit");
    let (first, second) = tokio::join!(
        command(&state, &path, "first"),
        command(&state, &path, "second")
    );
    assert!(matches!(
        (first.status(), second.status()),
        (StatusCode::OK, StatusCode::CONFLICT) | (StatusCode::CONFLICT, StatusCode::OK)
    ));
    stream_text(web).await;
    wait_idle(&state).await;
    assert_eq!(control.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn creation_idempotency_retries_are_bounded_and_never_regenerate() {
    let (state, control) = setup(Limits {
        retained_turns: 2,
        ..Default::default()
    });
    let body = json!({"text":"question"}).to_string();
    let response = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "application/json",
        body.clone(),
        Some("key"),
    )
    .await;
    let id = current_id(&state);
    let retry = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "application/json",
        body.clone(),
        Some("key"),
    )
    .await;
    assert!(stream_text(retry).await.contains(&id));
    let equivalent = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "application/json",
        "{ \"text\" : \"question\" }",
        Some("key"),
    )
    .await;
    assert!(stream_text(equivalent).await.contains(&id));
    assert_eq!(
        request(
            &state,
            "POST",
            "/v1/voice/turns",
            "application/json",
            "{\"text\":\"other\"}",
            Some("key")
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    command(&state, &format!("/v1/voice/turns/{id}/submit"), "question").await;
    stream_text(response).await;
    wait_idle(&state).await;
    let retry = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "application/json",
        body,
        Some("key"),
    )
    .await;
    assert!(stream_text(retry).await.contains("reply.completed"));
    assert_eq!(control.calls.lock().unwrap().len(), 1);
    for _ in 0..3 {
        let turn = command(&state, "/v1/voice/turns", "new").await;
        let new_id = current_id(&state);
        request(
            &state,
            "POST",
            &format!("/v1/voice/turns/{new_id}/cancel"),
            "",
            Body::empty(),
            None,
        )
        .await;
        stream_text(turn).await;
        wait_idle(&state).await;
    }
    assert_eq!(
        request(
            &state,
            "GET",
            &format!("/v1/voice/turns/{id}"),
            "",
            Body::empty(),
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(state.voice.lock().records.len(), 2);
    assert_eq!(state.voice.lock().retries.len(), 1);
    let expired = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "application/json",
        json!({"text":"question"}).to_string(),
        Some("key"),
    )
    .await;
    assert_eq!(expired.status(), StatusCode::GONE);
    assert_eq!(control.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn failed_or_empty_answers_do_not_commit_partial_history() {
    for empty in [false, true] {
        let (state, control) = setup(Limits::default());
        control.fail.store(!empty, Ordering::Release);
        control.empty.store(empty, Ordering::Release);
        let response = command(&state, "/v1/voice/turns", "question").await;
        let id = current_id(&state);
        command(&state, &format!("/v1/voice/turns/{id}/submit"), "question").await;
        let text = stream_text(response).await;
        assert!(text.contains("turn.failed"));
        assert!(!text.contains("private model details"));
        wait_idle(&state).await;
        let inspect = snapshot(&state).await;
        assert_eq!(inspect["current_turn"]["status"], "failed");
        assert!(inspect["history"].as_array().unwrap().is_empty());
    }
}

#[tokio::test]
async fn review_and_generation_timeouts_settle_without_history() {
    let (state, control) = setup(Limits {
        review_timeout: Duration::from_millis(35),
        generation_timeout: Duration::from_millis(35),
        ..Default::default()
    });
    let review = command(&state, "/v1/voice/turns", "unapproved").await;
    assert!(stream_text(review).await.contains("review_timeout"));
    wait_idle(&state).await;
    assert!(control.calls.lock().unwrap().is_empty());
    control.hold.store(true, Ordering::Release);
    let response = command(&state, "/v1/voice/turns", "question").await;
    let id = current_id(&state);
    command(&state, &format!("/v1/voice/turns/{id}/submit"), "question").await;
    assert!(stream_text(response).await.contains("generation_timeout"));
    wait_idle(&state).await;
    assert!(control.cancelled.load(Ordering::Acquire));
    assert!(snapshot(&state).await["history"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn reset_waits_for_native_cancel_and_rejects_stale_completion_and_edit() {
    let (state, control) = setup(Limits::default());
    control.hold.store(true, Ordering::Release);
    let response = command(&state, "/v1/voice/turns", "old").await;
    let id = current_id(&state);
    command(&state, &format!("/v1/voice/turns/{id}/submit"), "old").await;
    wait_until(|| !control.calls.lock().unwrap().is_empty()).await;
    let result = request(&state, "POST", "/v1/voice/reset", "", Body::empty(), None).await;
    assert_eq!(result.status(), StatusCode::OK);
    assert!(control.cancelled.load(Ordering::Acquire));
    assert!(stream_text(response).await.contains("turn.cancelled"));
    let inspect = snapshot(&state).await;
    assert!(inspect["history"].as_array().unwrap().is_empty());
    assert_eq!(inspect["current_turn"], Value::Null);
    assert_eq!(inspect["busy"], false);
    assert_eq!(
        command(
            &state,
            &format!("/v1/voice/turns/{id}/submit"),
            "stale edit"
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn web_disconnect_retains_result_but_test_disconnect_cancels_native_work() {
    let (state, control) = setup(Limits::default());
    control.hold.store(true, Ordering::Release);
    let response = command(&state, "/v1/voice/turns", "question").await;
    let id = current_id(&state);
    command(&state, &format!("/v1/voice/turns/{id}/submit"), "question").await;
    drop(response);
    wait_until(|| !control.calls.lock().unwrap().is_empty()).await;
    assert!(!control.cancelled.load(Ordering::Acquire));
    control.hold.store(false, Ordering::Release);
    wait_idle(&state).await;
    let recovered = value(
        request(
            &state,
            "GET",
            &format!("/v1/voice/turns/{id}"),
            "",
            Body::empty(),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(recovered["status"], "completed");
    let before = snapshot(&state).await;
    control.hold.store(true, Ordering::Release);
    let mut test = command(&state, "/v1/voice/test/reply", "private").await;
    assert!(test.body_mut().frame().await.is_some());
    wait_until(|| control.calls.lock().unwrap().len() == 2).await;
    drop(test);
    wait_idle(&state).await;
    assert!(control.cancelled.load(Ordering::Acquire));
    assert_eq!(snapshot(&state).await, before);
}

#[tokio::test]
async fn slow_web_stream_does_not_block_generation_and_results_remain_recoverable() {
    let (state, control) = setup(Limits::default());
    control.flood.store(true, Ordering::Release);
    let response = command(&state, "/v1/voice/turns", "question").await;
    let id = current_id(&state);
    command(&state, &format!("/v1/voice/turns/{id}/submit"), "question").await;
    wait_idle(&state).await;
    let recovered = value(
        request(
            &state,
            "GET",
            &format!("/v1/voice/turns/{id}"),
            "",
            Body::empty(),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(recovered["status"], "completed");
    assert_eq!(recovered["reply"], "好".repeat(300));
    assert!(stream_text(response).await.contains("reply.delta"));
}

#[tokio::test]
async fn unicode_budgets_count_characters_and_history_evicts_whole_pairs() {
    let (state, _) = setup(Limits {
        conversation: ConversationConfig {
            max_input_chars: 21,
            max_history_turns: 1,
            ..Default::default()
        },
        ..Default::default()
    });
    let response = command(&state, "/v1/voice/turns", "你好你好你好你好").await;
    assert_eq!(response.status(), StatusCode::OK); // 13 role + 8 question characters.
    let id = current_id(&state);
    assert_eq!(
        command(
            &state,
            &format!("/v1/voice/turns/{id}/submit"),
            "你好你好你好你好你"
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    command(
        &state,
        &format!("/v1/voice/turns/{id}/submit"),
        "你好你好你好你好",
    )
    .await;
    stream_text(response).await;
    wait_idle(&state).await;
    for text in ["新", "更"] {
        let response = command(&state, "/v1/voice/turns", text).await;
        let id = current_id(&state);
        command(&state, &format!("/v1/voice/turns/{id}/submit"), text).await;
        stream_text(response).await;
        wait_idle(&state).await;
    }
    let history = snapshot(&state).await["history"].clone();
    assert_eq!(history.as_array().unwrap().len(), 2);
    assert_eq!(history[0]["content"], "更");
}

#[tokio::test]
async fn wav_json_silence_and_body_limits_are_handled_without_auto_answering() {
    let (state, control) = setup(Limits::default());
    for (kind, body, status) in [
        ("text/plain", "question", StatusCode::UNSUPPORTED_MEDIA_TYPE),
        ("application/json", "{broken", StatusCode::BAD_REQUEST),
        (
            "application/json",
            "{\"text\":\" \"}",
            StatusCode::BAD_REQUEST,
        ),
        (
            "application/json",
            "{\"text\":\"q\",\"path\":\"x\"}",
            StatusCode::BAD_REQUEST,
        ),
        ("audio/wav", "", StatusCode::BAD_REQUEST),
    ] {
        assert_eq!(
            request(&state, "POST", "/v1/voice/turns", kind, body, None)
                .await
                .status(),
            status
        );
    }
    let large = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "audio/wav",
        vec![0u8; state.voice.limits.max_body_bytes + 1],
        None,
    )
    .await;
    assert_eq!(large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let silent = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "audio/wav",
        wav(true),
        None,
    )
    .await;
    assert!(stream_text(silent).await.contains("no_speech"));
    wait_idle(&state).await;
    let invalid = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "audio/wav",
        "invalid",
        None,
    )
    .await;
    assert!(stream_text(invalid).await.contains("transcription_failed"));
    wait_idle(&state).await;
    let voiced = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "Audio/Wav; charset=binary",
        wav(false),
        None,
    )
    .await;
    let id = current_id(&state);
    wait_until(|| {
        state
            .voice
            .lock()
            .record(&id)
            .is_some_and(|record| record.turn.status == Status::AwaitingReview)
    })
    .await;
    assert!(control.calls.lock().unwrap().is_empty());
    let id = current_id(&state);
    let stateless = request(
        &state,
        "POST",
        "/v1/transcribe",
        "audio/wav",
        wav(false),
        None,
    )
    .await;
    assert_eq!(stateless.status(), StatusCode::OK);
    assert_eq!(value(stateless).await["text"], "original transcript");
    assert_eq!(current_id(&state), id);
    request(
        &state,
        "POST",
        &format!("/v1/voice/turns/{id}/cancel"),
        "",
        Body::empty(),
        None,
    )
    .await;
    let stream = stream_text(voiced).await;
    assert!(stream.contains("transcript.ready") && stream.contains("turn.cancelled"));
}

#[tokio::test]
async fn timed_out_stt_keeps_shared_compute_reserved_until_native_work_settles() {
    let (state, control) = setup(Limits {
        transcription_timeout: Duration::from_millis(30),
        ..Default::default()
    });
    control.stt_hold.store(true, Ordering::Release);
    let web = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "audio/wav",
        wav(false),
        None,
    )
    .await;
    wait_until(|| control.stt_entered.load(Ordering::Acquire)).await;
    assert!(stream_text(web).await.contains("transcription_timeout"));
    assert_eq!(
        command(&state, "/v1/voice/test/reply", "test")
            .await
            .status(),
        StatusCode::CONFLICT
    );
    control.stt_hold.store(false, Ordering::Release);
    wait_idle(&state).await;
    assert_eq!(snapshot(&state).await["current_turn"]["status"], "failed");
}

#[tokio::test]
async fn cancel_before_worker_starts_releases_reserved_inference_permit() {
    let (state, control) = setup(Limits::default());
    let web = command(&state, "/v1/voice/turns", "question").await;
    let id = current_id(&state);
    command(&state, &format!("/v1/voice/turns/{id}/submit"), "question").await;
    assert!(state.voice.lock().record(&id).unwrap().job.is_some());
    request(
        &state,
        "POST",
        &format!("/v1/voice/turns/{id}/cancel"),
        "",
        Body::empty(),
        None,
    )
    .await;
    assert_eq!(state.voice.inference.available_permits(), 1);
    assert!(stream_text(web).await.contains("turn.cancelled"));
    wait_idle(&state).await;
    assert!(control.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reset_preserves_expired_key_tombstones_and_retry_ledger_is_bounded() {
    let (state, _) = setup(Limits {
        retained_idempotency_keys: 1,
        ..Default::default()
    });
    let body = json!({"text":"question"}).to_string();
    let web = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "application/json",
        body.clone(),
        Some("old-key"),
    )
    .await;
    request(&state, "POST", "/v1/voice/reset", "", Body::empty(), None).await;
    assert!(stream_text(web).await.contains("turn.cancelled"));
    let expired = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "application/json",
        body.clone(),
        Some("old-key"),
    )
    .await;
    assert_eq!(expired.status(), StatusCode::GONE);
    let saturated = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "application/json",
        body,
        Some("new-key"),
    )
    .await;
    assert_eq!(saturated.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(state.voice.lock().retries.len(), 1);
}

#[tokio::test]
async fn output_budget_failure_cancels_native_and_never_commits_history() {
    let (state, control) = setup(Limits {
        conversation: ConversationConfig {
            max_output_chars: 12,
            ..Default::default()
        },
        ..Default::default()
    });
    control.flood.store(true, Ordering::Release);
    let web = command(&state, "/v1/voice/turns", "question").await;
    let id = current_id(&state);
    command(&state, &format!("/v1/voice/turns/{id}/submit"), "question").await;
    assert!(stream_text(web).await.contains("invalid_model_response"));
    wait_idle(&state).await;
    let inspect = snapshot(&state).await;
    assert!(inspect["history"].as_array().unwrap().is_empty());
    assert!(
        inspect["current_turn"]["reply"]
            .as_str()
            .unwrap()
            .chars()
            .count()
            <= 12
    );
}

#[tokio::test]
async fn stateless_stt_is_isolated_and_reset_waits_for_uncancellable_web_stt() {
    let (state, control) = setup(Limits::default());
    control.stt_hold.store(true, Ordering::Release);
    let web = request(
        &state,
        "POST",
        "/v1/voice/turns",
        "audio/wav",
        wav(false),
        None,
    )
    .await;
    wait_until(|| control.stt_entered.load(Ordering::Acquire)).await;
    let resetting_state = state.clone();
    let reset = tokio::spawn(async move {
        request(
            &resetting_state,
            "POST",
            "/v1/voice/reset",
            "",
            Body::empty(),
            None,
        )
        .await
    });
    wait_until(|| state.voice.lock().resetting).await;
    assert!(!reset.is_finished());
    control.stt_hold.store(false, Ordering::Release);
    assert_eq!(reset.await.unwrap().status(), StatusCode::OK);
    assert!(stream_text(web).await.contains("turn.cancelled"));
    let before = snapshot(&state).await;
    let stt = request(
        &state,
        "POST",
        "/v1/transcribe",
        "audio/wav",
        wav(false),
        None,
    )
    .await;
    assert_eq!(stt.status(), StatusCode::OK);
    assert_eq!(snapshot(&state).await, before);
}

#[tokio::test]
async fn missing_reply_or_role_returns_not_ready_without_fallback() {
    let (mut state, control) = setup(Limits::default());
    state.voice = Voice::new(None, None, None, Limits::default());
    for path in ["/v1/voice/turns", "/v1/voice/test/reply"] {
        let response = command(&state, path, "question").await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            value(response).await["error"]["code"],
            "runtime_unavailable"
        );
    }
    assert!(control.calls.lock().unwrap().is_empty());
    assert_eq!(snapshot(&state).await["role"], Value::Null);
}

#[tokio::test]
async fn shutdown_cancels_all_active_operations_and_readiness_does_not_lock_models() {
    let (state, control) = setup(Limits::default());
    control.hold.store(true, Ordering::Release);
    let test = command(&state, "/v1/voice/test/reply", "test").await;
    wait_until(|| !control.calls.lock().unwrap().is_empty()).await;
    assert_eq!(
        request(&state, "GET", "/ready", "", Body::empty(), None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(snapshot(&state).await["busy"], true);
    state.voice.shutdown().await;
    assert!(control.cancelled.load(Ordering::Acquire));
    assert!(stream_text(test).await.contains("turn.cancelled"));
    assert_eq!(
        command(&state, "/v1/voice/turns", "after shutdown")
            .await
            .status(),
        StatusCode::CONFLICT
    );
}
