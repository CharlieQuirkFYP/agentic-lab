//! Mobile reasoning host. Ship as a dynamic framework, never merge its native
//! GGML objects into the Whisper static archive.

use std::collections::VecDeque;
use std::ffi::{c_char, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use va_core::{
    chat::{ChatPhase, ConversationState},
    intake::respond_with_plan,
    validate_response, ConversationConfig, ConversationModel,
};

const MAX_JSON: usize = 64 * 1024;
const MAX_OUTPUT: usize = 1024 * 1024;
type Loader = fn(&Configuration) -> Result<Box<dyn ConversationModel>, Failure>;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    schema_version: u32,
    model_path: String,
    system_prompt: String,
    threads: i32,
}

impl Configuration {
    fn validate(&self) -> Result<(), Failure> {
        if self.schema_version != 1
            || !(1..=16).contains(&self.threads)
            || self.model_path.is_empty()
            || self.model_path.len() > 4096
            || self.model_path.contains('\0')
        {
            return Err(Failure::new(1, "invalid reasoning configuration"));
        }
        ConversationState::for_role(&self.system_prompt)
            .approve(
                ChatPhase::AwaitingReview,
                &self.system_prompt,
                "Hello",
                &conversation_config(),
            )
            .map_err(|error| Failure::new(1, error.to_string()))?;
        Ok(())
    }
}

fn conversation_config() -> ConversationConfig {
    ConversationConfig::default()
}

#[derive(Clone, Debug, Serialize)]
struct Failure {
    code: i32,
    message: String,
    run_id: Option<String>,
}
impl Failure {
    fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            run_id: None,
        }
    }
    fn with_run(mut self, run_id: String) -> Self {
        self.run_id = Some(run_id);
        self
    }
}

#[derive(Default)]
struct State {
    model: Option<Box<dyn ConversationModel>>,
    configuration: Option<Configuration>,
    conversation: ConversationState,
    requests: VecDeque<String>,
    busy: bool,
    active_run: Option<String>,
    last_failure: Option<Failure>,
    load_time_ms: u64,
}

pub struct ReplyHandle {
    state: Mutex<State>,
    cancelled: Arc<AtomicBool>,
    loader: Loader,
}

impl ReplyHandle {
    fn new(loader: Loader) -> Self {
        Self {
            state: Mutex::new(State::default()),
            cancelled: Arc::new(AtomicBool::new(false)),
            loader,
        }
    }

    fn inspect(&self) -> Result<serde_json::Value, Failure> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Ok(serde_json::json!({
            "schema_version": 1,
            "ready": state.model.is_some() && !state.busy,
            "state": if state.busy { "running" } else if state.model.is_some() { "ready" } else { "unloaded" },
            "model_path": state.configuration.as_ref().map(|config| &config.model_path),
            "threads": state.configuration.as_ref().map(|config| config.threads),
            "active_run": state.active_run,
            "history_turns": state.conversation.history.len() / 2,
            "load_time_ms": state.load_time_ms,
            "conversation_config": conversation_config(),
            "gpu_layers": 0,
            "runtime": {"name": "reply-ffi", "version": env!("CARGO_PKG_VERSION")},
            "last_failure": state.last_failure,
        }))
    }

    fn configure(&self, configuration: Configuration) -> Result<serde_json::Value, Failure> {
        configuration.validate()?;
        let old = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.busy {
                return Err(Failure::new(3, "reasoning engine is busy"));
            }
            state.busy = true;
            state.configuration = None;
            state.conversation.reset();
            state.requests.clear();
            state.last_failure = None;
            state.model.take()
        };
        let mut lease = Lease {
            handle: self,
            model: None,
        };
        drop(old);
        let started = Instant::now();
        match (self.loader)(&configuration) {
            Ok(model) => {
                lease.model = Some(model);
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                state.conversation = ConversationState::for_role(&configuration.system_prompt);
                state.configuration = Some(configuration);
                state.load_time_ms = started.elapsed().as_millis() as u64;
            }
            Err(error) => {
                self.state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .last_failure = Some(error.clone());
                return Err(error);
            }
        }
        drop(lease);
        self.inspect()
    }

    fn respond(&self, request: Request) -> Result<serde_json::Value, Failure> {
        if let Some(attempt) = &request.attempt_id {
            if attempt.is_empty()
                || attempt.len() > 128
                || !attempt
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err(Failure::new(
                    1,
                    "attempt_id must contain 1-128 ASCII letters, digits, underscores or hyphens",
                ));
            }
        }
        if request.request_id.is_empty() || request.request_id.len() > 128 {
            return Err(Failure::new(1, "request_id must contain 1-128 bytes"));
        }
        let config = conversation_config();
        let (model, messages, plan, run_id) = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.busy {
                return Err(Failure::new(3, "reasoning engine is busy"));
            }
            if state.requests.contains(&request.request_id) {
                return Err(Failure::new(1, "duplicate reasoning request_id"));
            }
            // Do not evict retry identities and permit a duplicate inference.
            if state.requests.len() == 256 {
                return Err(Failure::new(
                    3,
                    "reset the conversation before sending more turns",
                ));
            }
            let effective = state
                .configuration
                .as_ref()
                .ok_or_else(|| Failure::new(4, "load a reasoning model first"))?;
            let plan = state.conversation.plan(&request.text);
            let messages = state
                .conversation
                .approve(
                    ChatPhase::AwaitingReview,
                    &effective.system_prompt,
                    &request.text,
                    &config,
                )
                .map_err(|error| Failure::new(1, error.to_string()))?;
            let model = state
                .model
                .take()
                .ok_or_else(|| Failure::new(4, "load a reasoning model first"))?;
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let run_id = request
                .attempt_id
                .clone()
                .unwrap_or_else(|| format!("ios_reply_{}", NEXT.fetch_add(1, Ordering::Relaxed)));
            state.active_run = Some(run_id.clone());
            state.requests.push_back(request.request_id);
            state.last_failure = None;
            state.busy = true;
            self.cancelled.store(false, Ordering::Release);
            (model, messages, plan, run_id)
        };
        let mut lease = Lease {
            handle: self,
            model: Some(model),
        };
        let started = Instant::now();
        let (done, deadline) = mpsc::channel::<()>();
        let cancelled = Arc::clone(&self.cancelled);
        let timer = std::thread::spawn(move || {
            if matches!(
                deadline.recv_timeout(Duration::from_secs(120)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                cancelled.store(true, Ordering::Release);
            }
        });
        let response = respond_with_plan(
            lease.model.as_mut().expect("admitted model").as_mut(),
            &messages,
            &config,
            plan.as_ref(),
            &self.cancelled,
            &mut |_| {},
        );
        drop(done);
        let _ = timer.join();
        let response = response
            .and_then(|text| {
                validate_response(&text, &config)?;
                if self.cancelled.load(Ordering::Acquire) {
                    return Err(va_core::ConversationError::Cancelled);
                }
                Ok(text)
            })
            .map_err(|error| Failure::new(2, error.to_string()).with_run(run_id.clone()));
        match response {
            Ok(text) => {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                if self.cancelled.load(Ordering::Acquire) {
                    let error = Failure::new(2, "reply generation cancelled");
                    state.last_failure = Some(error.clone());
                    return Err(error);
                }
                state
                    .conversation
                    .commit(request.text, text.clone(), &config)
                    .map_err(|error| Failure::new(2, error.to_string()).with_run(run_id.clone()))?;
                Ok(serde_json::json!({
                    "schema_version": 1,
                    "run_id": run_id,
                    "text": text,
                    "processing_time_ms": started.elapsed().as_millis() as u64,
                }))
            }
            Err(error) => {
                self.state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .last_failure = Some(error.clone());
                Err(error)
            }
        }
    }

    fn reset(&self) -> Result<serde_json::Value, Failure> {
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.busy {
                return Err(Failure::new(3, "reasoning engine is busy"));
            }
            state.conversation.reset();
            state.requests.clear();
        }
        self.inspect()
    }
}

struct Lease<'a> {
    handle: &'a ReplyHandle,
    model: Option<Box<dyn ConversationModel>>,
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        let mut state = self
            .handle
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.busy = false;
        let run_id = state.active_run.take();
        if std::thread::panicking() {
            state.configuration = None;
            state.conversation.reset();
            let mut failure = Failure::new(7, "reasoning panicked; reload the model");
            failure.run_id = run_id;
            state.last_failure = Some(failure);
        } else {
            state.model = self.model.take();
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    request_id: String,
    text: String,
    #[serde(default)]
    attempt_id: Option<String>,
}

fn load_model(configuration: &Configuration) -> Result<Box<dyn ConversationModel>, Failure> {
    reply_native::ReplyModel::load(
        Path::new(&configuration.model_path),
        &conversation_config(),
        configuration.threads,
        0,
    )
    .map(|model| Box::new(model) as Box<dyn ConversationModel>)
    .map_err(|error| Failure::new(2, error.to_string()))
}

unsafe fn handle<'a>(ptr: *const ReplyHandle) -> Result<&'a ReplyHandle, Failure> {
    ptr.as_ref()
        .ok_or_else(|| Failure::new(1, "null reasoning handle"))
}

unsafe fn input<T: for<'a> Deserialize<'a>>(ptr: *const u8, len: usize) -> Result<T, Failure> {
    if ptr.is_null() || len == 0 || len > MAX_JSON {
        return Err(Failure::new(1, "reasoning JSON must contain 1-65536 bytes"));
    }
    serde_json::from_slice(std::slice::from_raw_parts(ptr, len))
        .map_err(|error| Failure::new(1, error.to_string()))
}

unsafe fn boundary(
    output: *mut *mut c_char,
    call: impl FnOnce() -> Result<serde_json::Value, Failure>,
) -> i32 {
    if output.is_null() {
        return 1;
    }
    *output = std::ptr::null_mut();
    let result = catch_unwind(AssertUnwindSafe(call))
        .unwrap_or_else(|_| Err(Failure::new(7, "contained reasoning panic")));
    let (code, json) = match result {
        Ok(value) => (0, value),
        Err(error) => (error.code, serde_json::json!({ "error": error })),
    };
    match serde_json::to_string(&json) {
        Ok(text) if text.len() <= MAX_OUTPUT => match CString::new(text) {
            Ok(text) => {
                *output = text.into_raw();
                code
            }
            Err(_) => 7,
        },
        _ => 7,
    }
}

#[no_mangle]
pub extern "C" fn pheme_reply_create() -> *mut ReplyHandle {
    catch_unwind(|| Box::into_raw(Box::new(ReplyHandle::new(load_model))))
        .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// Handle must be live. No calls may race destruction. Output must be writable.
#[no_mangle]
pub unsafe extern "C" fn pheme_reply_inspect(
    ptr: *const ReplyHandle,
    output: *mut *mut c_char,
) -> i32 {
    boundary(output, || handle(ptr)?.inspect())
}

/// # Safety
/// Handle must be live, JSON readable for len bytes, and output writable.
#[no_mangle]
pub unsafe extern "C" fn pheme_reply_configure(
    ptr: *const ReplyHandle,
    json: *const u8,
    len: usize,
    output: *mut *mut c_char,
) -> i32 {
    boundary(output, || handle(ptr)?.configure(input(json, len)?))
}

/// # Safety
/// Handle must be live, JSON readable for len bytes, and output writable.
#[no_mangle]
pub unsafe extern "C" fn pheme_reply_respond(
    ptr: *const ReplyHandle,
    json: *const u8,
    len: usize,
    output: *mut *mut c_char,
) -> i32 {
    boundary(output, || {
        let request: Request = input(json, len)?;
        let attempt_id = request.attempt_id.clone();
        catch_unwind(AssertUnwindSafe(|| handle(ptr)?.respond(request))).unwrap_or_else(|_| {
            let mut failure = Failure::new(7, "contained reasoning panic; reload the model");
            failure.run_id = attempt_id;
            Err(failure)
        })
    })
}

/// # Safety
/// Handle must be live and output writable.
#[no_mangle]
pub unsafe extern "C" fn pheme_reply_reset(
    ptr: *const ReplyHandle,
    output: *mut *mut c_char,
) -> i32 {
    boundary(output, || handle(ptr)?.reset())
}

/// # Safety
/// Handle must be live; may run concurrently with generation, never destruction.
#[no_mangle]
pub unsafe extern "C" fn pheme_reply_cancel(ptr: *const ReplyHandle) {
    if let Some(handle) = ptr.as_ref() {
        handle.cancelled.store(true, Ordering::Release);
    }
}

/// # Safety
/// Pointer must be null or a unique output from this framework, freed once.
#[no_mangle]
pub unsafe extern "C" fn pheme_reply_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

/// # Safety
/// Pointer must be null or a unique live handle. Join all calls before freeing.
#[no_mangle]
pub unsafe extern "C" fn pheme_reply_free(ptr: *mut ReplyHandle) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;
    use va_core::ConversationMessage;

    struct FakeModel;
    impl ConversationModel for FakeModel {
        fn name(&self) -> &str {
            "fake"
        }
        fn is_ready(&self) -> bool {
            true
        }
        fn respond_stream(
            &mut self,
            messages: &[ConversationMessage],
            _: &ConversationConfig,
            cancelled: &AtomicBool,
            _: &mut dyn FnMut(&str),
        ) -> Result<String, va_core::ConversationError> {
            let text = &messages.last().unwrap().content;
            if text == "cancel" {
                cancelled.store(true, Ordering::Release);
            }
            if text == "panic" {
                panic!("fake model panic");
            }
            Ok(format!("Reply to {text}"))
        }
    }
    fn fake(_: &Configuration) -> Result<Box<dyn ConversationModel>, Failure> {
        Ok(Box::new(FakeModel))
    }
    fn config() -> Configuration {
        Configuration {
            schema_version: 1,
            model_path: "fake.gguf".into(),
            system_prompt: "Be accurate.".into(),
            threads: 2,
        }
    }
    fn request(id: &str, text: &str) -> Request {
        Request {
            attempt_id: None,
            request_id: id.into(),
            text: text.into(),
        }
    }

    #[test]
    fn attempt_identity_preserves_request_dedup_and_failure_correlation() {
        let handle = ReplyHandle::new(fake);
        handle.configure(config()).unwrap();
        let mut first = request("logical-request", "hello");
        first.attempt_id = Some("attempt-success".into());
        assert_eq!(handle.respond(first).unwrap()["run_id"], "attempt-success");
        let mut duplicate = request("logical-request", "hello");
        duplicate.attempt_id = Some("different-attempt".into());
        assert!(handle.respond(duplicate).is_err());
        let mut cancelled = request("cancelled-request", "cancel");
        cancelled.attempt_id = Some("attempt-cancelled".into());
        assert_eq!(
            handle.respond(cancelled).unwrap_err().run_id.as_deref(),
            Some("attempt-cancelled")
        );
        assert_eq!(
            handle.inspect().unwrap()["last_failure"]["run_id"],
            "attempt-cancelled"
        );
        assert_eq!(handle.inspect().unwrap()["history_turns"], 1);
        let mut bad = request("invalid-request", "hello");
        bad.attempt_id = Some("unbounded bad identity".into());
        assert_eq!(handle.respond(bad).unwrap_err().code, 1);
        let metadata = handle.inspect().unwrap();
        assert_eq!(
            metadata["conversation_config"],
            serde_json::to_value(conversation_config()).unwrap()
        );
        assert_eq!(metadata["gpu_layers"], 0);
    }

    #[test]
    fn loads_and_retains_only_bounded_successful_pairs() {
        let handle = ReplyHandle::new(fake);
        assert!(!handle.inspect().unwrap()["ready"].as_bool().unwrap());
        handle.configure(config()).unwrap();
        let history_limit = ConversationConfig::default().max_history_turns;
        for index in 0..history_limit + 3 {
            handle
                .respond(request(&index.to_string(), "hello"))
                .unwrap();
        }
        assert_eq!(handle.inspect().unwrap()["history_turns"], history_limit);
        assert!(handle.respond(request("0", "hello")).is_err());
        handle.reset().unwrap();
        assert_eq!(handle.inspect().unwrap()["history_turns"], 0);
    }

    #[test]
    fn mobile_intake_filters_bad_replies_retains_fields_and_resets() {
        let handle = ReplyHandle::new(fake);
        let mut configuration = config();
        configuration.system_prompt = include_str!("../../../roles/incident-reporting.txt").into();
        handle.configure(configuration).unwrap();
        let first = handle
            .respond(request("report", "Smoke at the west entrance."))
            .unwrap();
        assert_eq!(
            first["text"],
            "I have your account. Is anyone injured or trapped?"
        );
        // A retry never advances the state or reruns a turn.
        assert!(handle
            .respond(request("report", "Smoke at the west entrance."))
            .is_err());
        let revision = handle
            .state
            .lock()
            .unwrap()
            .conversation
            .intake
            .as_ref()
            .unwrap()
            .revision;
        assert!(handle.respond(request("cancelled", "cancel")).is_err());
        assert_eq!(
            handle
                .state
                .lock()
                .unwrap()
                .conversation
                .intake
                .as_ref()
                .unwrap()
                .revision,
            revision
        );
        let second = handle
            .respond(request("injuries", "Nobody is injured."))
            .unwrap();
        assert_eq!(
            second["text"],
            "I have your account. Are there any ongoing hazards?"
        );
        // Detail state survives eviction of all raw chat pairs.
        for index in 0..8 {
            handle
                .respond(request(&format!("followup-{index}"), "thanks"))
                .unwrap();
        }
        assert!(handle
            .state
            .lock()
            .unwrap()
            .conversation
            .intake
            .as_ref()
            .unwrap()
            .location
            .as_ref()
            .unwrap()
            .contains("west entrance"));
        handle.reset().unwrap();
        let state = handle.state.lock().unwrap();
        assert!(state.conversation.history.is_empty());
        assert!(state
            .conversation
            .intake
            .as_ref()
            .unwrap()
            .location
            .is_none());
        assert_eq!(
            state
                .conversation
                .intake
                .as_ref()
                .unwrap()
                .reviewed_revision,
            None
        );
    }

    #[test]
    fn cancelled_turn_does_not_commit_and_next_turn_works() {
        let handle = ReplyHandle::new(fake);
        handle.configure(config()).unwrap();
        assert!(handle.respond(request("1", "cancel")).is_err());
        assert_eq!(handle.inspect().unwrap()["history_turns"], 0);
        assert!(handle.respond(request("2", "hello")).is_ok());
    }

    #[test]
    fn busy_admission_and_retry_capacity_do_not_run_another_turn() {
        let handle = ReplyHandle::new(fake);
        handle.configure(config()).unwrap();
        {
            let mut state = handle.state.lock().unwrap();
            state.busy = true;
        }
        assert_eq!(handle.respond(request("1", "hello")).unwrap_err().code, 3);
        assert_eq!(handle.configure(config()).unwrap_err().code, 3);
        assert_eq!(handle.reset().unwrap_err().code, 3);
        {
            let mut state = handle.state.lock().unwrap();
            state.busy = false;
            state.requests = (0..256).map(|index| index.to_string()).collect();
        }
        assert_eq!(handle.respond(request("new", "hello")).unwrap_err().code, 3);
        assert_eq!(handle.inspect().unwrap()["history_turns"], 0);
        handle.reset().unwrap();
        assert!(handle.respond(request("new", "hello")).is_ok());
    }

    #[test]
    fn panic_unloads_native_owner_and_boundary_returns_error() {
        let handle = ReplyHandle::new(fake);
        handle.configure(config()).unwrap();
        let mut output = std::ptr::null_mut();
        let code = unsafe { boundary(&mut output, || handle.respond(request("1", "panic"))) };
        assert_eq!(code, 7);
        assert!(!output.is_null());
        unsafe { pheme_reply_string_free(output) };
        assert_eq!(handle.inspect().unwrap()["ready"], false);
        assert!(handle.configure(config()).is_ok());
    }

    #[test]
    fn invalid_configuration_and_missing_model_are_recoverable() {
        let handle = ReplyHandle::new(load_model);
        assert!(handle.respond(request("1", "hello")).is_err());
        let mut invalid = config();
        invalid.threads = 0;
        assert!(handle.configure(invalid).is_err());
        assert!(handle.configure(config()).is_err());
        assert_eq!(handle.inspect().unwrap()["state"], "unloaded");
    }

    #[test]
    fn c_abi_nulls_bad_json_and_string_ownership() {
        unsafe {
            let ptr = pheme_reply_create();
            let mut output = std::ptr::null_mut();
            assert_eq!(pheme_reply_inspect(std::ptr::null(), &mut output), 1);
            assert!(CStr::from_ptr(output)
                .to_str()
                .unwrap()
                .contains("null reasoning handle"));
            pheme_reply_string_free(output);
            assert_eq!(
                pheme_reply_configure(ptr, std::ptr::null(), 0, &mut output),
                1
            );
            pheme_reply_string_free(output);
            assert_eq!(pheme_reply_inspect(ptr, &mut output), 0);
            pheme_reply_string_free(output);
            pheme_reply_free(ptr);
            pheme_reply_free(std::ptr::null_mut());
            pheme_reply_string_free(std::ptr::null_mut());
        }
    }
}
