use super::*;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::AtomicU64;
use va_core::build_messages;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    dir: PathBuf,
    worker: PathBuf,
    model: PathBuf,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "pheme-reply-ipc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let worker = dir.join("worker.sh");
        let staged = dir.join("worker.pending");
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&staged).unwrap();
            file.write_all(include_bytes!("../tests/fixtures/worker.sh"))
                .unwrap();
            file.sync_all().unwrap();
        }
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Publish only after the writable descriptor is closed; executing a
        // freshly written file can otherwise race on some development filesystems.
        std::fs::rename(staged, &worker).unwrap();
        let model = dir.join(format!("{mode}.gguf"));
        std::fs::write(&model, b"tiny fixture, not weights").unwrap();
        Self { dir, worker, model }
    }
    fn load(&self) -> ReplyModel {
        ReplyModel::load_with_worker(
            &self.worker,
            &self.model,
            &ConversationConfig::default(),
            2,
            0,
        )
        .unwrap()
    }
    fn commands(&self) -> Vec<Command> {
        std::fs::read_to_string(self.dir.join("worker.sh.log"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn pid(&self) -> u32 {
        std::fs::read_to_string(self.dir.join("worker.sh.pid"))
            .unwrap()
            .parse()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}

fn assert_reaped(pid: u32) {
    #[cfg(target_os = "linux")]
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "worker child was not reaped"
    );
    #[cfg(not(target_os = "linux"))]
    let _ = pid;
}

#[test]
fn persistent_worker_streams_unicode_and_receives_exact_fresh_context() {
    let fixture = Fixture::new("valid");
    let mut model = fixture.load();
    let pid = model.worker_pid();
    assert!(model.is_ready());
    assert_eq!(model.name(), "fake-resident-model");
    assert_eq!(model.load_time_ms(), 7);
    let config = ConversationConfig::default();
    let cancelled = AtomicBool::new(false);
    for question in ["  question café\n ", "independent test"] {
        let messages = build_messages("trusted role", &[], question, &config).unwrap();
        let mut deltas = Vec::new();
        let text = model
            .respond_stream(&messages, &config, &cancelled, &mut |delta| {
                deltas.push(delta.to_owned())
            })
            .unwrap();
        assert_eq!(text, "Hello café 世界 🧯");
        assert_eq!(deltas.concat(), text);
        assert_eq!(model.worker_pid(), pid);
    }
    let commands = fixture.commands();
    assert_eq!(
        commands
            .iter()
            .filter(|command| matches!(command, Command::Init { .. }))
            .count(),
        1
    );
    let Command::Generate { id, messages, .. } = &commands[1] else {
        panic!("missing generate")
    };
    assert_eq!(*id, 1);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].content, "  question café\n ");
    let Command::Generate { id, messages, .. } = &commands[2] else {
        panic!("missing fresh generate")
    };
    assert_eq!(*id, 2);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].content, "independent test");
    drop(model);
    assert_reaped(pid);
}

#[test]
fn cancellation_drains_late_deltas_and_terminal_then_reuses_same_worker() {
    let fixture = Fixture::new("valid");
    let mut model = fixture.load();
    let config = ConversationConfig::default();
    let cancelled = AtomicBool::new(false);
    let pid = model.worker_pid();
    let messages = build_messages("role", &[], "WAIT", &config).unwrap();
    let mut chunks = Vec::new();
    let result = model.respond_stream(&messages, &config, &cancelled, &mut |delta| {
        chunks.push(delta.to_owned());
        cancelled.store(true, Ordering::Release);
    });
    assert!(matches!(result, Err(ConversationError::Cancelled)));
    assert_eq!(chunks, ["partial 世界"]);
    assert!(model.is_ready());
    cancelled.store(false, Ordering::Release);
    let messages = build_messages("role", &[], "next fresh question", &config).unwrap();
    assert_eq!(
        model
            .respond_stream(&messages, &config, &cancelled, &mut |_| {})
            .unwrap(),
        "Hello café 世界 🧯"
    );
    assert_eq!(model.worker_pid(), pid);
    let commands = fixture.commands();
    assert!(matches!(commands[2], Command::Cancel { id: 1 }));
    assert!(matches!(commands[3], Command::Generate { id: 2, .. }));
}

#[test]
fn precancelled_and_invalid_requests_never_reach_worker() {
    let fixture = Fixture::new("valid");
    let mut model = fixture.load();
    let config = ConversationConfig::default();
    let messages = build_messages("role", &[], "question", &config).unwrap();
    assert!(matches!(
        model.respond_stream(&messages, &config, &AtomicBool::new(true), &mut |_| panic!(
            "unexpected delta"
        )),
        Err(ConversationError::Cancelled)
    ));
    assert!(model
        .respond_stream(&[], &config, &AtomicBool::new(false), &mut |_| {})
        .is_err());
    assert_eq!(fixture.commands().len(), 1);
}

#[test]
fn native_error_categories_survive_ipc_and_do_not_force_weight_reload() {
    let fixture = Fixture::new("valid");
    let mut model = fixture.load();
    let config = ConversationConfig::default();
    let messages = build_messages("role", &[], "CONTEXT_ERROR", &config).unwrap();
    assert!(matches!(
        model.respond_stream(&messages, &config, &AtomicBool::new(false), &mut |_| {}),
        Err(ConversationError::ContextExceeded)
    ));
    assert!(model.is_ready());
}

#[test]
fn protocol_failure_or_crash_poison_and_reap_worker() {
    for mode in ["CRASH", "STALE", "MISMATCH", "TRUNCATED", "OVERSIZE"] {
        let fixture = Fixture::new("valid");
        let mut model = fixture.load();
        let pid = model.worker_pid();
        let config = ConversationConfig::default();
        let messages = build_messages("role", &[], mode, &config).unwrap();
        assert!(
            matches!(
                model.respond_stream(&messages, &config, &AtomicBool::new(false), &mut |_| {}),
                Err(ConversationError::Backend(_))
            ),
            "mode {mode}"
        );
        assert!(!model.is_ready());
        assert_reaped(pid);
    }
}

#[test]
fn client_enforces_output_limit_before_emitting_oversized_delta() {
    let fixture = Fixture::new("valid");
    let mut model = fixture.load();
    let config = ConversationConfig {
        max_output_chars: 2,
        ..Default::default()
    };
    let messages = build_messages("role", &[], "question", &config).unwrap();
    let result = model.respond_stream(&messages, &config, &AtomicBool::new(false), &mut |_| {
        panic!("oversized delta was emitted")
    });
    assert!(matches!(
        result,
        Err(ConversationError::OutputTooLong { limit: 2 })
    ));
    assert!(!model.is_ready());
}

#[test]
fn unresponsive_generation_or_cancellation_is_bounded_and_reaped() {
    for cancellation in [false, true] {
        let fixture = Fixture::new("valid");
        let mut model = fixture.load();
        let pid = model.worker_pid();
        let config = ConversationConfig::default();
        let messages = build_messages("role", &[], "HANG", &config).unwrap();
        model
            .send(&Command::Generate {
                id: 1,
                messages,
                config: config.clone(),
            })
            .unwrap();
        let result = model.generate(
            1,
            &config,
            &AtomicBool::new(cancellation),
            &mut |_| {},
            Duration::from_millis(50),
            Duration::from_millis(50),
        );
        if cancellation {
            assert!(matches!(result, Err(ConversationError::Cancelled)));
        } else {
            assert!(matches!(result, Err(ConversationError::Backend(_))));
        }
        assert!(!model.is_ready());
        assert_reaped(pid);
    }
}

#[test]
fn failed_or_hung_startup_reaps_child_and_missing_binary_is_actionable() {
    for mode in ["INIT_FAIL", "INIT_HANG"] {
        let fixture = Fixture::new(mode);
        assert!(ReplyModel::load_with_timeout(
            &fixture.worker,
            &fixture.model,
            &ConversationConfig::default(),
            2,
            0,
            Duration::from_secs(1)
        )
        .is_err());
        assert_reaped(fixture.pid());
    }
    let fixture = Fixture::new("valid");
    let error = match ReplyModel::load_with_worker(
        &fixture.dir.join("missing-worker"),
        &fixture.model,
        &ConversationConfig::default(),
        2,
        0,
    ) {
        Ok(_) => panic!("unexpected startup"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("build/ship pheme-reply-worker"));
}

#[test]
#[ignore = "requires PHEME_VA_REPLY_MODEL, PHEME_VA_REPLY_PROMPT and built/shipped native worker"]
fn real_worker_streams_without_linking_llama_into_client() {
    let path = std::env::var_os("PHEME_VA_REPLY_MODEL").expect("set PHEME_VA_REPLY_MODEL");
    let prompt = std::env::var_os("PHEME_VA_REPLY_PROMPT").expect("set PHEME_VA_REPLY_PROMPT");
    let prompt = va_core::load_prompt(Path::new(&prompt)).unwrap();
    let config = ConversationConfig {
        temperature: 0.0,
        max_output_tokens: 64,
        ..Default::default()
    };
    let mut model = ReplyModel::load(Path::new(&path), &config, 2, 0).unwrap();
    let messages = build_messages(
        &prompt.text,
        &[],
        "There is smoke near the east entrance.",
        &config,
    )
    .unwrap();
    let mut streamed = String::new();
    let result = model
        .respond_stream(&messages, &config, &AtomicBool::new(false), &mut |delta| {
            streamed.push_str(delta)
        })
        .unwrap();
    assert_eq!(result, streamed);
}
