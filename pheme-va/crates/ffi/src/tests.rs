use super::*;
use std::ffi::CStr;
use std::sync::{mpsc, Arc};
use std::time::Duration;
use va_core::{
    Engine, EngineError, NormalizedAudio, RawTranscription, Transcriber, TranscriptionOptions,
};

unsafe fn response(call: impl FnOnce(*mut *mut c_char) -> i32) -> (i32, serde_json::Value) {
    let mut output = ptr::null_mut();
    let code = call(&mut output);
    assert!(!output.is_null(), "expected owned output, code={code}");
    let value = serde_json::from_str(CStr::from_ptr(output).to_str().unwrap()).unwrap();
    pheme_va_string_free(output);
    (code, value)
}

unsafe fn run(handle: *mut PhemeVaHandle) -> (i32, serde_json::Value) {
    let audio = vec![0.05; 16000];
    response(|out| pheme_va_transcribe_v1(handle, audio.as_ptr(), audio.len(), 16000, 1, out))
}

#[test]
fn unloaded_inspection_config_validation_and_owned_errors() {
    unsafe {
        let handle = pheme_va_create();
        let (_, status) = response(|out| pheme_va_inspect(handle, out));
        assert_eq!(status["state"], "unavailable");
        assert_eq!(status["ready"], false);
        assert_eq!(status["capabilities"]["whisper"], cfg!(feature = "whisper"));
        let (code, _) = run(handle);
        assert_eq!(code, UNAVAILABLE);
        for value in [
            json!({"schema_version": 2, "model_path": "x"}),
            json!({"schema_version": 1, "model_path": "x", "threads": 0}),
            json!({"schema_version": 1, "model_path": "x", "max_audio_seconds": 121}),
            json!({"schema_version": 1, "model_path": "x", "unknown_setting": true}),
            json!({"schema_version": 1, "model_path": "x", "language": "en\0"}),
        ] {
            let bytes = serde_json::to_vec(&value).unwrap();
            let (code, _) =
                response(|out| pheme_va_configure(handle, bytes.as_ptr(), bytes.len(), out));
            assert!(code == INVALID_ARGUMENT || code == UNSUPPORTED);
        }
        let bytes = br#"{"schema_version":1,"model_path":"/definitely/missing/pheme-model.bin"}"#;
        let (code, _) =
            response(|out| pheme_va_configure(handle, bytes.as_ptr(), bytes.len(), out));
        assert_eq!(
            code,
            if cfg!(feature = "whisper") {
                BACKEND_ERROR
            } else {
                UNSUPPORTED
            }
        );
        let (_, status) = response(|out| pheme_va_inspect(handle, out));
        assert_eq!(status["state"], "unavailable");
        assert!(status["configuration"].is_null());
        assert_eq!(status["last_failure"]["code"], code);
        pheme_va_free(handle);
        pheme_va_free(ptr::null_mut());
        pheme_va_string_free(ptr::null_mut());
    }
}

#[test]
fn successful_result_matches_metrics_and_observer_outlives_engine() {
    unsafe {
        let handle = Box::into_raw(Box::new(test_support::handle()));
        let observer = pheme_va_observer_new(handle);
        let (code, result) = run(handle);
        assert_eq!(code, OK);
        assert_eq!(result["result"]["text"], "Incident at west entrance.");
        let run_id = result["run_id"].as_str().unwrap();
        pheme_va_free(handle);
        let (_, batches) = response(|out| pheme_va_observer_drain(observer, out));
        let batches = batches.as_array().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0]["run_id"], run_id);
        assert_eq!(
            batches[0]["schema_version"],
            metrics::METRICS_SCHEMA_VERSION
        );
        for event in batches[0]["events"].as_array().unwrap() {
            assert_eq!(event["run_id"], run_id);
        }
        let (_, empty) = response(|out| pheme_va_observer_drain(observer, out));
        assert_eq!(empty, json!([]));
        pheme_va_observer_free(observer);
    }
}

#[test]
fn rejects_audio_bounds_before_reading_and_initializes_output() {
    unsafe {
        let handle = Box::into_raw(Box::new(test_support::handle()));
        let bad = std::ptr::NonNull::<f32>::dangling().as_ptr();
        for (count, rate, channels) in [
            (usize::MAX, 16000, 1),
            (1, 0, 1),
            (1, 16000, 0),
            (3, 16000, 2),
            (MAX_AUDIO_SAMPLES + 1, 16000, 1),
            (16000 * 121, 16000, 1),
        ] {
            let (code, _) =
                response(|out| pheme_va_transcribe_v1(handle, bad, count, rate, channels, out));
            assert_eq!(code, INVALID_ARGUMENT);
        }
        let mut output = std::ptr::NonNull::<c_char>::dangling().as_ptr();
        assert_eq!(
            pheme_va_inspect(ptr::null_mut(), &mut output),
            INVALID_ARGUMENT
        );
        assert_ne!(output, std::ptr::NonNull::<c_char>::dangling().as_ptr());
        pheme_va_string_free(output);
        assert_eq!(pheme_va_inspect(handle, ptr::null_mut()), INVALID_ARGUMENT);
        let samples = [f32::NAN];
        let (code, error) =
            response(|out| pheme_va_transcribe_v1(handle, samples.as_ptr(), 1, 16000, 1, out));
        assert_eq!(code, INVALID_ARGUMENT);
        assert!(error["error"]["run_id"].is_string());
        pheme_va_free(handle);
    }
}

struct Blocking {
    entered: mpsc::Sender<i32>,
    resume: mpsc::Receiver<()>,
}
impl Transcriber for Blocking {
    fn name(&self) -> &str {
        "blocking-fake"
    }
    fn is_ready(&self) -> bool {
        true
    }
    fn transcribe(
        &mut self,
        _: &NormalizedAudio,
        options: &TranscriptionOptions,
    ) -> Result<RawTranscription, EngineError> {
        self.entered.send(options.decoder.threads).unwrap();
        self.resume.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok(RawTranscription::text("A controlled test."))
    }
}

#[test]
fn status_metrics_and_busy_responses_remain_live_during_inference_and_close() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let handle = Arc::new(test_support::handle());
    let config = Configuration {
        threads: 7,
        ..Configuration::for_model("test-only".to_owned())
    };
    handle.configure(config.clone()).unwrap();
    handle.lock().engine = Some(Engine::with_config(
        Blocking {
            entered: entered_tx,
            resume: resume_rx,
        },
        config.engine_config(),
    ));
    let worker_handle = Arc::clone(&handle);
    let worker = std::thread::spawn(move || unsafe { run(Arc::as_ptr(&worker_handle).cast_mut()) });
    assert_eq!(entered_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 7);
    // A regression holding the engine lock would deadlock here until the fake times out.
    let status = serde_json::to_value(handle.snapshot()).unwrap();
    assert_eq!(status["state"], "running");
    let run_id = status["active_run"].as_str().unwrap();
    unsafe {
        let observer = pheme_va_observer_new(Arc::as_ptr(&handle).cast_mut());
        let (code, batches) = response(|out| pheme_va_observer_drain(observer, out));
        assert_eq!(code, OK);
        let batches = batches.as_array().unwrap();
        assert!(!batches.is_empty());
        assert!(batches.iter().all(|b| b["run_id"] == run_id));
        pheme_va_observer_free(observer);
    }
    assert_eq!(handle.configure(config).unwrap_err().code, BUSY);
    unsafe {
        assert_eq!(run(Arc::as_ptr(&handle).cast_mut()).0, BUSY);
    }
    assert_eq!(handle.close().unwrap_err().code, BUSY);
    assert_eq!(
        serde_json::to_value(handle.snapshot()).unwrap()["state"],
        "closing"
    );
    resume_tx.send(()).unwrap();
    let (code, result) = worker.join().unwrap();
    assert_eq!(code, OK);
    assert_eq!(result["run_id"], run_id);
    assert_eq!(
        serde_json::to_value(handle.snapshot()).unwrap()["state"],
        "closed"
    );
    unsafe {
        assert_eq!(run(Arc::as_ptr(&handle).cast_mut()).0, CLOSED);
    }
    handle.close().unwrap();
}

struct Broken {
    panic: bool,
}
impl Transcriber for Broken {
    fn name(&self) -> &str {
        "broken-fake"
    }
    fn is_ready(&self) -> bool {
        true
    }
    fn transcribe(
        &mut self,
        _: &NormalizedAudio,
        _: &TranscriptionOptions,
    ) -> Result<RawTranscription, EngineError> {
        assert!(!self.panic, "controlled panic");
        Err(EngineError::Backend {
            message: "controlled failure".to_owned(),
        })
    }
}

#[test]
fn failures_keep_run_identity_and_panics_require_reload() {
    for panics in [false, true] {
        let handle = test_support::handle();
        handle.lock().engine = Some(Engine::new(Broken { panic: panics }));
        let (code, error) = unsafe { run((&handle as *const PhemeVaHandle).cast_mut()) };
        assert!(error["error"]["run_id"].is_string());
        assert_eq!(code, if panics { PANIC } else { BACKEND_ERROR });
        let status = serde_json::to_value(handle.snapshot()).unwrap();
        let run_id = status["last_failure"]["run_id"].as_str().unwrap();
        assert_eq!(error["error"]["run_id"], run_id);
        assert!(handle.batcher.drain().iter().all(|b| b.run_id == run_id));
        assert_eq!(status["state"], if panics { "unavailable" } else { "idle" });
        handle
            .configure(Configuration::for_model("recovered".to_owned()))
            .unwrap();
        unsafe {
            assert_eq!(run((&handle as *const PhemeVaHandle).cast_mut()).0, OK);
        }
    }
}

#[test]
fn settings_apply_without_reloading_and_metrics_disable_future_operations() {
    let handle = test_support::handle();
    let mut config = Configuration::for_model("test-only".to_owned());
    config.metrics_enabled = false;
    config.dictionary = vec![" KLASS  ".to_owned(), "KLASS".to_owned()];
    handle.configure(config).unwrap();
    let status = serde_json::to_value(handle.snapshot()).unwrap();
    assert_eq!(status["configuration"]["dictionary"], json!(["KLASS"]));
    unsafe {
        assert_eq!(run((&handle as *const PhemeVaHandle).cast_mut()).0, OK);
    }
    assert!(handle.batcher.drain().is_empty());
}

#[test]
fn failed_replacement_clears_stale_configuration_and_can_recover() {
    fn loader(config: &Configuration) -> Result<Engine, BridgeError> {
        if config.model_path == "bad" {
            return Err(BridgeError::new(BACKEND_ERROR, "bad model"));
        }
        test_support::load(config)
    }
    let handle = PhemeVaHandle::with_loader(loader);
    handle
        .configure(Configuration::for_model("good".to_owned()))
        .unwrap();
    assert_eq!(
        handle
            .configure(Configuration::for_model("bad".to_owned()))
            .unwrap_err()
            .code,
        BACKEND_ERROR
    );
    let status = serde_json::to_value(handle.snapshot()).unwrap();
    assert_eq!(status["state"], "unavailable");
    assert!(status["configuration"].is_null());
    handle
        .configure(Configuration::for_model("good".to_owned()))
        .unwrap();
    assert_eq!(
        serde_json::to_value(handle.snapshot()).unwrap()["state"],
        "idle"
    );
}

#[cfg(feature = "whisper")]
#[test]
fn legacy_transcription_and_metrics_keep_their_c_contract() {
    unsafe {
        let handle = Box::into_raw(Box::new(test_support::handle()));
        let dictionary = CString::new("KLASS").unwrap();
        assert_eq!(
            pheme_va_whisper_set_dictionary(handle, dictionary.as_ptr()),
            OK
        );
        let samples = vec![0.05; 16000];
        let mut output = ptr::null_mut();
        assert_eq!(
            pheme_va_transcribe(
                handle,
                samples.as_ptr(),
                samples.len(),
                16000,
                1,
                &mut output
            ),
            OK
        );
        assert_eq!(
            CStr::from_ptr(output).to_str().unwrap(),
            "Incident at west entrance."
        );
        pheme_va_string_free(output);
        let (code, batches) = response(|out| pheme_va_metrics_drain(handle, out));
        assert_eq!(code, OK);
        assert!(!batches.as_array().unwrap().is_empty());
        assert_eq!(pheme_va_metrics_set_enabled(handle, false), OK);
        assert_eq!(run(handle).0, OK);
        assert_eq!(
            response(|out| pheme_va_metrics_drain(handle, out)).1,
            json!([])
        );
        pheme_va_whisper_free(handle);
    }
}
