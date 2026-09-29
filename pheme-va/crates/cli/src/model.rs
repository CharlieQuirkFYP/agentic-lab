use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use va_core::{Engine, EngineConfig, RuleBasedFormatter, Transcriber};

#[cfg(feature = "whisper")]
use whispercpp::{WhisperConfig, WhisperTranscriber};
#[cfg(feature = "zipformer")]
use zipformer::{ZipformerConfig, ZipformerTranscriber};

#[derive(Debug, Deserialize)]
pub struct ModelManifest {
    pub models: Vec<ModelEntry>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub family: String,
    pub model: PathBuf,
    #[serde(default)]
    pub tokenizer: Option<PathBuf>,
    #[serde(default)]
    pub tokens: Option<PathBuf>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub tokenizer_sha256: Option<String>,
    #[serde(default)]
    pub tokens_sha256: Option<String>,
    #[serde(default)]
    pub model_size: Option<String>,
    #[serde(default)]
    pub model_size_bytes: Option<u64>,
    #[serde(default)]
    pub tensor_contract: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub sample_rate: Option<u32>,
    #[serde(default)]
    pub channels: Option<u16>,
    #[serde(default)]
    pub timestamps: bool,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub runtime: Option<String>,
}

impl ModelManifest {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("could not read model manifest {}", path.display()))?;
        toml::from_str(&contents)
            .with_context(|| format!("could not parse model manifest {}", path.display()))
    }

    fn find(&self, model_id: &str) -> Result<ModelEntry> {
        self.models
            .iter()
            .find(|model| model.id == model_id)
            .cloned()
            .ok_or_else(|| {
                let available = self
                    .models
                    .iter()
                    .map(|model| model.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                anyhow!("unknown speech model `{model_id}`; available models: {available}")
            })
    }
}

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
    let manifest = ModelManifest::load(manifest_path)?;
    let entry = manifest.find(model_id)?;
    let model_path = resolve_artifact(manifest_path, &entry.model);

    match entry.family.as_str() {
        "whisper" => load_whisper(entry, model_path),
        "zipformer" => load_zipformer(entry, manifest_path),
        family => Err(anyhow!(
            "model `{model_id}` uses unsupported model family `{family}`"
        )),
    }
}

#[cfg(feature = "whisper")]
fn load_whisper(entry: ModelEntry, model_path: PathBuf) -> Result<Box<dyn Transcriber>> {
    let transcriber = WhisperTranscriber::from_file_with_metadata(
        &model_path,
        WhisperConfig {
            threads: std::thread::available_parallelism()
                .map(|threads| threads.get().min(8) as i32)
                .unwrap_or(4),
            use_gpu: cfg!(feature = "whisper-metal"),
            flash_attention: false,
        },
        entry.id,
        entry.revision,
    )
    .with_context(|| format!("failed to load Whisper model {}", model_path.display()))?;
    Ok(Box::new(transcriber))
}

#[cfg(not(feature = "whisper"))]
fn load_whisper(_entry: ModelEntry, _model_path: PathBuf) -> Result<Box<dyn Transcriber>> {
    Err(anyhow!(
        "this binary was built without Whisper support; rebuild with `--features whisper`"
    ))
}

#[cfg(feature = "zipformer")]
fn load_zipformer(entry: ModelEntry, manifest_path: &Path) -> Result<Box<dyn Transcriber>> {
    let tokenizer = entry
        .tokenizer
        .as_ref()
        .ok_or_else(|| anyhow!("model `{}` has no tokenizer path", entry.id))?;
    let tokens = entry
        .tokens
        .as_ref()
        .ok_or_else(|| anyhow!("model `{}` has no token-list path", entry.id))?;
    let transcriber = ZipformerTranscriber::from_files(
        resolve_artifact(manifest_path, &entry.model),
        resolve_artifact(manifest_path, tokenizer),
        resolve_artifact(manifest_path, tokens),
        ZipformerConfig {
            model_id: entry.id,
            model_revision: entry.revision,
        },
    )
    .context("failed to load Zipformer model")?;
    Ok(Box::new(transcriber))
}

#[cfg(not(feature = "zipformer"))]
fn load_zipformer(_entry: ModelEntry, _manifest_path: &Path) -> Result<Box<dyn Transcriber>> {
    Err(anyhow!(
        "this binary was built without Zipformer support; rebuild with `--features zipformer`"
    ))
}

pub(crate) fn family_supported(family: &str) -> bool {
    matches!(family, "whisper" | "zipformer")
}

pub(crate) fn family_compiled(family: &str) -> bool {
    (family == "whisper" && cfg!(feature = "whisper"))
        || (family == "zipformer" && cfg!(feature = "zipformer"))
}

pub(crate) fn resolve_artifact(manifest_path: &Path, artifact: &Path) -> PathBuf {
    if artifact.is_absolute() {
        return artifact.to_owned();
    }
    let manifest_parent = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let root_name = manifest_parent.file_name();
    let first_component = artifact.components().next();
    if root_name.is_some()
        && first_component
            .is_some_and(|component| component.as_os_str() == root_name.expect("checked above"))
    {
        manifest_parent
            .parent()
            .unwrap_or(manifest_parent)
            .join(artifact)
    } else {
        manifest_parent.join(artifact)
    }
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
    fn resolves_paths_relative_to_manifest_directory() {
        let path = resolve_artifact(
            Path::new("/tmp/pheme/models/manifest.toml"),
            Path::new("zipformer/bpe.model"),
        );
        assert_eq!(path, PathBuf::from("/tmp/pheme/models/zipformer/bpe.model"));
    }
}
