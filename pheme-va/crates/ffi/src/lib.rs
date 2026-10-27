//! Thin, synchronous C ABI for native hosts. See include/pheme_va.h for ownership,
//! concurrency, bounds, and stable error codes. Platform audio and transport stay in hosts.

mod bridge;
mod config;
#[cfg(feature = "whisper")]
mod legacy;
#[cfg(feature = "whisper")]
pub use legacy::*;
#[cfg(any(test, pheme_ffi_test))]
mod test_support;
#[cfg(test)]
mod tests;

pub use bridge::{PhemeVaHandle, PhemeVaObserver};

use std::ffi::{c_char, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::Arc;

use config::{Configuration, MAX_AUDIO_SAMPLES, MAX_CONFIG_BYTES, SCHEMA_VERSION};
use serde::Serialize;
use serde_json::json;
use va_core::AudioBuffer;

pub const OK: i32 = 0;
pub const INVALID_ARGUMENT: i32 = 1;
pub const BACKEND_ERROR: i32 = 2;
pub const BUSY: i32 = 3;
pub const UNAVAILABLE: i32 = 4;
pub const UNSUPPORTED: i32 = 5;
pub const CLOSED: i32 = 6;
pub const PANIC: i32 = 7;
const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct BridgeError {
    code: i32,
    message: String,
    run_id: Option<String>,
}

impl BridgeError {
    pub(crate) fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            run_id: None,
        }
    }
    fn with_run(self, run_id: String) -> Self {
        self.with_optional_run(Some(run_id))
    }
    fn with_optional_run(mut self, run_id: Option<String>) -> Self {
        self.run_id = run_id;
        self
    }
}

#[no_mangle]
pub extern "C" fn pheme_va_abi_version() -> u32 {
    SCHEMA_VERSION
}

#[no_mangle]
pub extern "C" fn pheme_va_has_whisper() -> bool {
    cfg!(feature = "whisper")
}

#[no_mangle]
pub extern "C" fn pheme_va_create() -> *mut PhemeVaHandle {
    catch_unwind(|| Box::into_raw(Box::new(PhemeVaHandle::new()))).unwrap_or(ptr::null_mut())
}

/// # Safety
/// Handle must be live. Bytes must reference length readable bytes. Output must
/// be writable and all buffers must remain valid until return.
#[no_mangle]
pub unsafe extern "C" fn pheme_va_configure(
    handle: *mut PhemeVaHandle,
    bytes: *const u8,
    length: usize,
    output: *mut *mut c_char,
) -> i32 {
    json_call(output, || {
        let handle = live_handle(handle)?;
        if bytes.is_null() || length == 0 || length > MAX_CONFIG_BYTES {
            return Err(BridgeError::new(
                INVALID_ARGUMENT,
                "configuration must contain 1–16384 bytes",
            ));
        }
        let configuration: Configuration =
            serde_json::from_slice(std::slice::from_raw_parts(bytes, length)).map_err(|error| {
                BridgeError::new(
                    INVALID_ARGUMENT,
                    format!("invalid configuration JSON: {error}"),
                )
            })?;
        handle.configure(configuration)?;
        Ok(handle.snapshot())
    })
}

/// # Safety
/// Handle must be live and output must be writable for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn pheme_va_inspect(
    handle: *mut PhemeVaHandle,
    output: *mut *mut c_char,
) -> i32 {
    json_call(output, || Ok(live_handle(handle)?.snapshot()))
}

/// # Safety
/// Handle must be live. Samples must reference sample_count readable, aligned
/// floats. Output must be writable. No buffer may be mutated until return.
#[no_mangle]
pub unsafe extern "C" fn pheme_va_transcribe_v1(
    handle: *mut PhemeVaHandle,
    samples: *const f32,
    sample_count: usize,
    sample_rate: u32,
    channels: u16,
    output: *mut *mut c_char,
) -> i32 {
    json_call(output, || {
        let handle = live_handle(handle)?;
        // Reject shape and absolute allocation limits before admitting or dereferencing audio.
        validate_audio_shape(samples, sample_count, sample_rate, channels, 120)?;
        let mut run_id = None;
        let operation = catch_unwind(AssertUnwindSafe(|| {
            let mut lease = handle.begin_run()?;
            run_id = lease.run_id();
            let operation = (|| {
                validate_audio_shape(
                    samples,
                    sample_count,
                    sample_rate,
                    channels,
                    lease.max_audio_seconds(),
                )?;
                let audio = AudioBuffer::new(
                    sample_rate,
                    channels,
                    std::slice::from_raw_parts(samples, sample_count).to_vec(),
                )
                .map_err(|e| BridgeError::new(INVALID_ARGUMENT, e.to_string()))?;
                lease.transcribe(audio)
            })();
            operation.map_err(|error| lease.failure(error))
        }));
        match operation {
            Ok(result) => result,
            // A later operation can already have replaced last_failure after the
            // lease settled. Keep this call's correlation independent of that state.
            Err(_) => Err(
                BridgeError::new(PANIC, "Rust operation panicked; reload the model")
                    .with_optional_run(run_id),
            ),
        }
    })
}

/// # Safety
/// Handle must be live and output must be writable. Closing may overlap work;
/// freeing must wait until all callers have returned.
#[no_mangle]
pub unsafe extern "C" fn pheme_va_close(
    handle: *mut PhemeVaHandle,
    output: *mut *mut c_char,
) -> i32 {
    json_call(output, || {
        let handle = live_handle(handle)?;
        handle.close()?;
        Ok(handle.snapshot())
    })
}

/// # Safety
/// Handle must be null or allocated by this library and not previously freed.
/// The caller must have joined ALL calls on this handle, including inspection.
#[no_mangle]
pub unsafe extern "C" fn pheme_va_free(handle: *mut PhemeVaHandle) {
    if !handle.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(Box::from_raw(handle))));
    }
}

/// Acquire a separate reference to the one shared drain queue.
/// # Safety
/// Handle must be live. Creation may not race engine destruction.
#[no_mangle]
pub unsafe extern "C" fn pheme_va_observer_new(handle: *mut PhemeVaHandle) -> *mut PhemeVaObserver {
    catch_unwind(AssertUnwindSafe(|| match handle.as_ref() {
        Some(handle) => Box::into_raw(Box::new(PhemeVaObserver {
            batcher: Arc::clone(&handle.batcher),
        })),
        None => ptr::null_mut(),
    }))
    .unwrap_or(ptr::null_mut())
}

/// Returns the existing MetricBatch JSON array, without an extra envelope.
/// # Safety
/// Observer must be live and output writable. It can outlive the engine.
#[no_mangle]
pub unsafe extern "C" fn pheme_va_observer_drain(
    observer: *mut PhemeVaObserver,
    output: *mut *mut c_char,
) -> i32 {
    json_call(output, || {
        let observer = observer
            .as_ref()
            .ok_or_else(|| BridgeError::new(INVALID_ARGUMENT, "null observer"))?;
        Ok(observer.batcher.drain())
    })
}

/// # Safety
/// Observer must be live and output must be writable for this call.
#[no_mangle]
pub unsafe extern "C" fn pheme_va_observer_inspect(
    observer: *mut PhemeVaObserver,
    output: *mut *mut c_char,
) -> i32 {
    json_call(output, || {
        let observer = observer
            .as_ref()
            .ok_or_else(|| BridgeError::new(INVALID_ARGUMENT, "null observer"))?;
        Ok(json!({
            "schema_version": SCHEMA_VERSION, "capacity_events": bridge::METRIC_CAPACITY,
            "pending_events": observer.batcher.pending_events(),
            "dropped_events": observer.batcher.dropped_events(),
        }))
    })
}

/// # Safety
/// Observer must be null or returned by observer_new, not freed, with no calls in flight.
#[no_mangle]
pub unsafe extern "C" fn pheme_va_observer_free(observer: *mut PhemeVaObserver) {
    if !observer.is_null() {
        drop(Box::from_raw(observer));
    }
}

/// # Safety
/// Value must be null or an owned string returned by ANY function in this library.
/// Release exactly once; do not modify, release elsewhere, or use after freeing.
#[no_mangle]
pub unsafe extern "C" fn pheme_va_string_free(value: *mut c_char) {
    if !value.is_null() {
        drop(CString::from_raw(value));
    }
}

unsafe fn live_handle<'a>(handle: *mut PhemeVaHandle) -> Result<&'a PhemeVaHandle, BridgeError> {
    handle
        .as_ref()
        .ok_or_else(|| BridgeError::new(INVALID_ARGUMENT, "null engine handle"))
}

fn validate_audio_shape(
    samples: *const f32,
    count: usize,
    rate: u32,
    channels: u16,
    max_seconds: u32,
) -> Result<(), BridgeError> {
    if samples.is_null()
        || count == 0
        || count > MAX_AUDIO_SAMPLES
        || !(8000..=192000).contains(&rate)
        || !(1..=8).contains(&channels)
        || count % usize::from(channels) != 0
        || count as u64 > u64::from(rate) * u64::from(channels) * u64::from(max_seconds)
    {
        return Err(BridgeError::new(
            INVALID_ARGUMENT,
            "invalid audio shape or size/duration limit exceeded",
        ));
    }
    Ok(())
}

unsafe fn json_call<T: Serialize>(
    output: *mut *mut c_char,
    operation: impl FnOnce() -> Result<T, BridgeError>,
) -> i32 {
    if output.is_null() {
        return INVALID_ARGUMENT;
    }
    *output = ptr::null_mut();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let (code, value) = match operation() {
            Ok(value) => (OK, serde_json::to_string(&value)),
            Err(error) => (
                error.code,
                serde_json::to_string(&json!({"schema_version": SCHEMA_VERSION, "error": error})),
            ),
        };
        let value = value.map_err(|_| BACKEND_ERROR)?;
        if value.len() > MAX_OUTPUT_BYTES {
            return Err(BACKEND_ERROR);
        }
        let value = CString::new(value).map_err(|_| BACKEND_ERROR)?;
        *output = value.into_raw();
        Ok(code)
    }));
    match outcome {
        Ok(Ok(code)) => code,
        Ok(Err(code)) => code,
        Err(_) => {
            let error = json!({"schema_version": SCHEMA_VERSION, "error": BridgeError::new(PANIC, "Rust operation panicked; inspect the engine before retrying")});
            if let Ok(value) = CString::new(error.to_string()) {
                *output = value.into_raw();
            }
            PANIC
        }
    }
}
