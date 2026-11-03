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
    build_messages, validate_response, ConversationConfig, ConversationMessage, ConversationModel,
    ConversationRole,
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
        build_messages(&self.system_prompt, &[], "Hello", &conversation_config())
            .map_err(|error| Failure::new(1, error.to_string()))?;
        Ok(())
    }
}

fn conversation_config() -> ConversationConfig {
    ConversationConfig {
        context_size: 2048,
        max_input_chars: 6000,
        max_output_tokens: 128,
        max_history_turns: 2,
        temperature: 0.0,
        ..Default::default()
    }
}

#[derive(Clone, Debug, Serialize)]
struct Failure {
    code: i32,
    message: String,
}
impl Failure {
    fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Default)]
struct State {
    model: Option<Box<dyn ConversationModel>>,
    configuration: Option<Configuration>,
    history: Vec<ConversationMessage>,
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
            "history_turns": state.history.len() / 2,
            "load_time_ms": state.load_time_ms,
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
            state.history.clear();
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
        if request.request_id.is_empty() || request.request_id.len() > 128 {
            return Err(Failure::new(1, "request_id must contain 1-128 bytes"));
        }
        let config = conversation_config();
        let (model, messages, run_id) = {
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
            let messages = build_messages(
                &effective.system_prompt,
                &state.history,
                &request.text,
                &config,
            )
            .map_err(|error| Failure::new(1, error.to_string()))?;
            let model = state
                .model
                .take()
                .ok_or_else(|| Failure::new(4, "load a reasoning model first"))?;
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let run_id = format!("ios_reply_{}", NEXT.fetch_add(1, Ordering::Relaxed));
            state.active_run = Some(run_id.clone());
            state.requests.push_back(request.request_id);
            state.last_failure = None;
            state.busy = true;
            self.cancelled.store(false, Ordering::Release);
            (model, messages, run_id)
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
        let response = lease
            .model
            .as_mut()
            .expect("admitted model")
            .respond_stream(&messages, &config, &self.cancelled, &mut |_| {});
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
            .map_err(|error| Failure::new(2, error.to_string()));
        match response {
            Ok(text) => {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                state.history = messages[1..].to_vec();
                state.history.push(ConversationMessage {
                    role: ConversationRole::Assistant,
                    content: text.clone(),
                });
                while state.history.len() > config.max_history_turns * 2 {
                    state.history.drain(..2);
                }
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
            state.history.clear();
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
        state.active_run = None;
        if std::thread::panicking() {
            state.configuration = None;
            state.history.clear();
            state.last_failure = Some(Failure::new(7, "reasoning panicked; reload the model"));
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
    boundary(output, || handle(ptr)?.respond(input(json, len)?))
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
            request_id: id.into(),
            text: text.into(),
        }
    }

    #[test]
    fn loads_and_retains_only_bounded_successful_pairs() {
        let handle = ReplyHandle::new(fake);
        assert!(!handle.inspect().unwrap()["ready"].as_bool().unwrap());
        handle.configure(config()).unwrap();
        for index in 0..5 {
            handle
                .respond(request(&index.to_string(), "hello"))
                .unwrap();
        }
        assert_eq!(handle.inspect().unwrap()["history_turns"], 2);
        assert!(handle.respond(request("4", "hello")).is_err());
        handle.reset().unwrap();
        assert_eq!(handle.inspect().unwrap()["history_turns"], 0);
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
