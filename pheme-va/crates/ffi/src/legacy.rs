//! Compatibility entry points, present only in Whisper-enabled builds.
use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use crate::bridge::ensure_idle;
use crate::config::Configuration;
use crate::*;

#[no_mangle]
/// # Safety
/// model_path must be null or a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn pheme_va_whisper_new(model_path: *const c_char) -> *mut PhemeVaHandle {
    catch_unwind(AssertUnwindSafe(|| {
        if model_path.is_null() {
            return ptr::null_mut();
        }
        let Ok(path) = CStr::from_ptr(model_path).to_str() else {
            return ptr::null_mut();
        };
        let handle = Box::new(PhemeVaHandle::new());
        if handle
            .configure(Configuration::for_model(path.to_owned()))
            .is_err()
        {
            return ptr::null_mut();
        }
        Box::into_raw(handle)
    }))
    .unwrap_or(ptr::null_mut())
}

#[no_mangle]
/// # Safety
/// Handle must be live. Dictionary must be null or a valid UTF-8 C string.
pub unsafe extern "C" fn pheme_va_whisper_set_dictionary(
    handle: *mut PhemeVaHandle,
    dictionary: *const c_char,
) -> i32 {
    legacy_call(|| {
        let handle = live_handle(handle)?;
        let mut state = handle.lock();
        ensure_idle(&state)?;
        let mut config = state
            .config
            .clone()
            .ok_or_else(|| BridgeError::new(UNAVAILABLE, "model unavailable"))?;
        config.dictionary = if dictionary.is_null() {
            Vec::new()
        } else {
            CStr::from_ptr(dictionary)
                .to_str()
                .map_err(|_| BridgeError::new(INVALID_ARGUMENT, "invalid UTF-8"))?
                .split(',')
                .map(str::to_owned)
                .collect()
        };
        let config = config.validate()?;
        *state
            .engine
            .as_mut()
            .ok_or_else(|| BridgeError::new(UNAVAILABLE, "model unavailable"))?
            .config_mut() = config.engine_config();
        state.config = Some(config);
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// Same audio, handle, and output contract as pheme_va_transcribe_v1.
pub unsafe extern "C" fn pheme_va_transcribe(
    handle: *mut PhemeVaHandle,
    samples: *const f32,
    count: usize,
    rate: u32,
    channels: u16,
    output: *mut *mut c_char,
) -> i32 {
    if output.is_null() {
        return INVALID_ARGUMENT;
    }
    *output = ptr::null_mut();
    legacy_call(|| {
        let mut structured = ptr::null_mut();
        let code = pheme_va_transcribe_v1(handle, samples, count, rate, channels, &mut structured);
        let json = if structured.is_null() {
            String::new()
        } else {
            CString::from_raw(structured)
                .into_string()
                .unwrap_or_default()
        };
        if code != OK {
            return Err(BridgeError::new(code, "transcription failed"));
        }
        let result: serde_json::Value = serde_json::from_str(&json)
            .map_err(|_| BridgeError::new(BACKEND_ERROR, "invalid result"))?;
        let text = result["result"]["text"]
            .as_str()
            .ok_or_else(|| BridgeError::new(BACKEND_ERROR, "missing transcript"))?;
        *output = CString::new(text)
            .map_err(|_| BridgeError::new(BACKEND_ERROR, "invalid transcript"))?
            .into_raw();
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// Handle must be live for this call. Applies to subsequently admitted runs.
pub unsafe extern "C" fn pheme_va_metrics_set_enabled(
    handle: *mut PhemeVaHandle,
    enabled: bool,
) -> i32 {
    legacy_call(|| {
        let handle = live_handle(handle)?;
        let mut state = handle.lock();
        if matches!(
            state.phase,
            crate::bridge::Phase::Closed | crate::bridge::Phase::Closing
        ) {
            return Err(BridgeError::new(CLOSED, "engine closed"));
        }
        let config = state
            .config
            .as_mut()
            .ok_or_else(|| BridgeError::new(UNAVAILABLE, "model unavailable"))?;
        config.metrics_enabled = enabled;
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// Handle must be live and output writable. Shares the observer's drain queue.
pub unsafe extern "C" fn pheme_va_metrics_drain(
    handle: *mut PhemeVaHandle,
    output: *mut *mut c_char,
) -> i32 {
    json_call(output, || Ok(live_handle(handle)?.batcher.drain()))
}

#[no_mangle]
/// # Safety
/// Same exclusive-lifetime contract as pheme_va_free.
pub unsafe extern "C" fn pheme_va_whisper_free(handle: *mut PhemeVaHandle) {
    pheme_va_free(handle);
}

fn legacy_call(operation: impl FnOnce() -> Result<(), BridgeError>) -> i32 {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(())) => OK,
        Ok(Err(error)) => error.code,
        Err(_) => PANIC,
    }
}
