//! Shared adapter construction; hosts choose and provision artifacts.
#[cfg(feature = "zipformer")]
use anyhow::Context;
use anyhow::{bail, Result};
use std::path::Path;
use va_core::{
    registry::{ModelEntry, ModelPurpose},
    ConversationConfig, LoadedPrompt, Transcriber,
};
pub fn load_stt(entry: ModelEntry, manifest: &Path, threads: i32) -> Result<Box<dyn Transcriber>> {
    match entry.family.as_str() {
        "whisper" => load_whisper(&entry.resolve_artifact(manifest), Some(&entry), threads),
        "zipformer" => load_zipformer(entry, manifest),
        family => bail!("unsupported STT family `{family}`"),
    }
}

#[cfg(feature = "whisper")]
pub fn load_whisper(
    path: &Path,
    entry: Option<&ModelEntry>,
    threads: i32,
) -> Result<Box<dyn Transcriber>> {
    let config = whispercpp::WhisperConfig {
        threads,
        use_gpu: cfg!(feature = "whisper-metal"),
        flash_attention: false,
    };
    Ok(Box::new(match entry {
        Some(entry) => whispercpp::WhisperTranscriber::from_file_with_metadata(
            path,
            config,
            entry.id.clone(),
            entry.revision.clone(),
        )?,
        None => whispercpp::WhisperTranscriber::from_file(path, config)?,
    }))
}

#[cfg(not(feature = "whisper"))]
pub fn load_whisper(
    _path: &Path,
    _entry: Option<&ModelEntry>,
    _threads: i32,
) -> Result<Box<dyn Transcriber>> {
    bail!("this binary was built without Whisper support; rebuild with --features whisper")
}

#[cfg(feature = "zipformer")]
fn load_zipformer(entry: ModelEntry, manifest: &Path) -> Result<Box<dyn Transcriber>> {
    use va_core::resolve_artifact;
    Ok(Box::new(zipformer::ZipformerTranscriber::from_files(
        entry.resolve_artifact(manifest),
        resolve_artifact(
            manifest,
            entry
                .tokenizer
                .as_deref()
                .context("missing Zipformer tokenizer")?,
        ),
        resolve_artifact(
            manifest,
            entry
                .tokens
                .as_deref()
                .context("missing Zipformer tokens")?,
        ),
        zipformer::ZipformerConfig {
            model_id: entry.id,
            model_revision: entry.revision,
        },
    )?))
}

#[cfg(not(feature = "zipformer"))]
fn load_zipformer(_entry: ModelEntry, _manifest: &Path) -> Result<Box<dyn Transcriber>> {
    bail!("this binary was built without Zipformer support; rebuild with --features zipformer")
}

pub fn load_reply(
    path: &Path,
    config: &ConversationConfig,
    threads: i32,
    gpu_layers: u32,
) -> Result<reply_model::ReplyModel> {
    reply_model::ReplyModel::load(path, config, threads, gpu_layers)
}
pub fn reply_prompt(
    entry: &ModelEntry,
    manifest: &Path,
    paths: &[std::path::PathBuf],
    config: &ConversationConfig,
) -> Result<LoadedPrompt> {
    anyhow::ensure!(
        entry.purpose() == Some(ModelPurpose::Reply),
        "model is not a reply model"
    );
    anyhow::ensure!(
        entry.runtime.as_deref() == Some("llama.cpp")
            && entry.model.extension().is_some_and(|e| e == "gguf"),
        "unsupported reply adapter"
    );
    let prompt = if paths.is_empty() {
        entry.load_prompt(manifest, None)?
    } else {
        va_core::load_prompt_files(paths)?
    };
    anyhow::ensure!(
        prompt.text.chars().count() < config.max_input_chars,
        "role leaves no room for a message"
    );
    Ok(prompt)
}
