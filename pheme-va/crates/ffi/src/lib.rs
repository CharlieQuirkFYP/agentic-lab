//! Minimal C ABI for native mobile hosts.
//!
//! The app owns microphone capture and passes interleaved f32 samples to this
//! bridge. Build this crate with `--features whisper` for the model-backed
//! constructor. A generated Swift/Kotlin binding layer can sit on top of these
//! small functions without putting platform audio APIs in the Rust core.

use std::ffi::{c_char, CString};

#[cfg(feature = "whisper")]
use std::ffi::CStr;
#[cfg(feature = "whisper")]
use std::ptr;

#[no_mangle]
pub extern "C" fn pheme_va_has_whisper() -> bool {
    cfg!(feature = "whisper")
}

#[cfg(feature = "whisper")]
use va_core::{AudioBuffer, DictionaryHints, Engine, WhisperConfig, WhisperTranscriber};

#[cfg(feature = "whisper")]
pub struct PhemeVaHandle {
    agent: Engine,
}

/// Load a model once. The returned handle is prewarmed and should be reused
/// for multiple recordings.
///
/// # Safety
///
/// `model_path` must be null or a valid, NUL-terminated UTF-8 C string that
/// remains valid for the duration of this call.
#[cfg(feature = "whisper")]
#[no_mangle]
pub unsafe extern "C" fn pheme_va_whisper_new(model_path: *const c_char) -> *mut PhemeVaHandle {
    if model_path.is_null() {
        return ptr::null_mut();
    }
    let Ok(model_path) = CStr::from_ptr(model_path).to_str() else {
        return ptr::null_mut();
    };
    let Ok(transcriber) = WhisperTranscriber::from_file(model_path, WhisperConfig::default())
    else {
        return ptr::null_mut();
    };
    Box::into_raw(Box::new(PhemeVaHandle {
        agent: Engine::new(transcriber),
    }))
}

/// Replace dictionary terms used to build the Whisper initial prompt. The
/// value is a comma-separated UTF-8 string; the core bounds and deduplicates it.
///
/// # Safety
///
/// `handle` must come from [`pheme_va_whisper_new`] and `dictionary` must
/// be null or a valid, NUL-terminated UTF-8 C string for this call.
#[cfg(feature = "whisper")]
#[no_mangle]
pub unsafe extern "C" fn pheme_va_whisper_set_dictionary(
    handle: *mut PhemeVaHandle,
    dictionary: *const c_char,
) -> i32 {
    if handle.is_null() {
        return 1;
    }
    let values = if dictionary.is_null() {
        Vec::new()
    } else {
        let Ok(dictionary) = CStr::from_ptr(dictionary).to_str() else {
            return 1;
        };
        dictionary.split(',').map(str::to_owned).collect()
    };
    (*handle).agent.config_mut().dictionary = DictionaryHints::with_terms(values);
    0
}

/// Transcribe interleaved floating-point samples. The output is a newly
/// allocated UTF-8 C string and must be released with
/// [`pheme_va_string_free`]. Returns zero on success.
///
/// # Safety
///
/// `handle` must come from [`pheme_va_whisper_new`], `samples` must point
/// to `sample_count` readable `f32` values, and `output_text` must point to a
/// writable output pointer. The buffers must remain valid for this call.
#[cfg(feature = "whisper")]
#[no_mangle]
pub unsafe extern "C" fn pheme_va_transcribe(
    handle: *mut PhemeVaHandle,
    samples: *const f32,
    sample_count: usize,
    sample_rate: u32,
    channels: u16,
    output_text: *mut *mut c_char,
) -> i32 {
    if handle.is_null() || samples.is_null() || output_text.is_null() || sample_count == 0 {
        return 1;
    }
    *output_text = ptr::null_mut();
    let samples = std::slice::from_raw_parts(samples, sample_count);
    let Ok(audio) = AudioBuffer::new(sample_rate, channels, samples.to_vec()) else {
        return 1;
    };
    let handle = &mut *handle;
    let Ok(result) = handle.agent.transcribe(audio) else {
        return 2;
    };
    let Ok(text) = CString::new(result.text) else {
        return 2;
    };
    *output_text = text.into_raw();
    0
}

///
/// # Safety
///
/// `handle` must be null or a pointer returned by
/// [`pheme_va_whisper_new`] that has not already been freed.
#[cfg(feature = "whisper")]
#[no_mangle]
pub unsafe extern "C" fn pheme_va_whisper_free(handle: *mut PhemeVaHandle) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}

/// Release a string returned by this crate.
///
/// # Safety
///
/// `value` must be a pointer previously returned by this crate's
/// `pheme_va_transcribe` function, or null. It must not be freed twice.
#[no_mangle]
pub unsafe extern "C" fn pheme_va_string_free(value: *mut c_char) {
    if !value.is_null() {
        drop(CString::from_raw(value));
    }
}
