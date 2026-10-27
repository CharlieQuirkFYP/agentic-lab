//! Compiled only for Rust unit tests or the isolated simulator test artifact.
use crate::{config::Configuration, BridgeError, PhemeVaHandle};
use va_core::{
    Engine, EngineError, NormalizedAudio, RawTranscription, Transcriber, TranscriptionOptions,
};

pub(crate) struct Fake;
impl Transcriber for Fake {
    fn name(&self) -> &str {
        "ffi-test-fake"
    }
    fn is_ready(&self) -> bool {
        true
    }
    fn transcribe(
        &mut self,
        _: &NormalizedAudio,
        _: &TranscriptionOptions,
    ) -> Result<RawTranscription, EngineError> {
        Ok(RawTranscription::text("Incident at west entrance."))
    }
}
pub(crate) fn load(config: &Configuration) -> Result<Engine, BridgeError> {
    Ok(Engine::with_config(Fake, config.engine_config()))
}
pub(crate) fn handle() -> PhemeVaHandle {
    let handle = PhemeVaHandle::with_loader(load);
    handle
        .configure(Configuration::for_model("test-only".to_owned()))
        .expect("fake configuration");
    handle
}

#[cfg(pheme_ffi_test)]
#[no_mangle]
pub extern "C" fn pheme_va_test_new() -> *mut PhemeVaHandle {
    Box::into_raw(Box::new(handle()))
}

#[cfg(pheme_ffi_test)]
#[no_mangle]
pub extern "C" fn pheme_va_test_create() -> *mut PhemeVaHandle {
    Box::into_raw(Box::new(PhemeVaHandle::with_loader(load)))
}
