use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use anyhow::Result;
pub use va_core::registry::*;
use va_core::{Engine, EngineConfig, RuleBasedFormatter, Transcriber};

pub fn create_engine(
    model_id: &str,
    manifest_path: &Path,
    max_seconds: u32,
    language: Option<String>,
    dictionary: Vec<String>,
) -> Result<Engine> {
    let transcriber = load_transcriber(model_id, manifest_path)?;
    let config = EngineConfig {
        max_audio_seconds: Some(max_seconds),
        language,
        dictionary: va_core::DictionaryHints::with_terms(dictionary),
        ..EngineConfig::default()
    };
    Ok(Engine::with_boxed_config(transcriber, config).with_cleaner(RuleBasedFormatter))
}

fn load_transcriber(model_id: &str, manifest_path: &Path) -> Result<Box<dyn Transcriber>> {
    let entry = ModelManifest::load(manifest_path)?.find(model_id)?;
    if entry.purpose() != Some(ModelPurpose::Transcript) {
        anyhow::bail!("model `{model_id}` is not a transcription model");
    }
    va_runtime::loader::load_stt(
        entry,
        manifest_path,
        std::thread::available_parallelism()
            .map(|t| t.get().min(8) as i32)
            .unwrap_or(4),
    )
}

pub(crate) fn family_supported(family: &str) -> bool {
    matches!(family, "whisper" | "zipformer")
}

pub(crate) fn family_compiled(family: &str) -> bool {
    (family == "whisper" && cfg!(feature = "whisper"))
        || (family == "zipformer" && cfg!(feature = "zipformer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_manifest_paths_written_from_the_workspace_root() {
        let path = resolve_artifact(
            Path::new("models/manifest.toml"),
            Path::new("models/zipformer/small/model.tflite"),
        );
        assert_eq!(path, PathBuf::from("models/zipformer/small/model.tflite"));
    }

    #[test]
    fn never_loads_reply_entries_as_transcription() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
        let error = match create_engine("qwen2.5-1.5b-instruct-q4-k-m", &manifest, 30, None, vec![])
        {
            Ok(_) => panic!("reply entry unexpectedly became a transcriber"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("not a transcription model"));
    }

    #[test]
    fn resolves_paths_relative_to_manifest_directory() {
        let path = resolve_artifact(
            Path::new("/tmp/pheme/models/manifest.toml"),
            Path::new("zipformer/bpe.model"),
        );
        assert_eq!(path, PathBuf::from("/tmp/pheme/models/zipformer/bpe.model"));
    }
}
