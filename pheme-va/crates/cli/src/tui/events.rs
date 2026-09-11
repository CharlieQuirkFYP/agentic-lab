use std::path::PathBuf;

use va_core::{AudioBuffer, TranscriptionResult};

use super::logs::LogEntry;

pub type RequestId = u64;
pub type RunId = String;

pub struct ModelLoadRequest {
    pub request_id: RequestId,
    pub model_id: String,
    pub manifest_path: PathBuf,
    pub max_seconds: u32,
    pub language: Option<String>,
    pub dictionary: Vec<String>,
    pub switch_from: Option<String>,
}

pub enum WorkerCommand {
    LoadModel(ModelLoadRequest),
    TranscribeWav {
        request_id: RequestId,
        run_id: RunId,
        path: PathBuf,
    },
    TranscribeAudio {
        request_id: RequestId,
        run_id: RunId,
        source: String,
        audio: AudioBuffer,
    },
    Shutdown,
}

pub enum WorkerEvent {
    ModelLoadStarted {
        request_id: RequestId,
        model_id: String,
        switch_from: Option<String>,
    },
    ModelReady {
        request_id: RequestId,
        model_id: String,
        model_family: String,
        backend: String,
        load_duration_ms: u128,
        switch_from: Option<String>,
    },
    ModelLoadFailed {
        request_id: RequestId,
        model_id: String,
        error: String,
        switch_from: Option<String>,
    },
    ProcessingStarted {
        request_id: RequestId,
        run_id: RunId,
        source: String,
    },
    Result {
        request_id: RequestId,
        run_id: RunId,
        source: String,
        audio_duration_seconds: f32,
        result: Box<TranscriptionResult>,
    },
    ProcessingFailed {
        request_id: RequestId,
        run_id: RunId,
        source: String,
        error: String,
    },
    Log(LogEntry),
    WorkerStopped,
}
