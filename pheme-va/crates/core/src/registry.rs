//! Shared local TOML model catalog. Loading metadata never downloads or activates a model.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_PROMPT_CHARS: usize = 16_384;
pub const MAX_PROMPT_FILES: usize = 32;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelPurpose {
    Transcript,
    Reply,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelManifest {
    pub models: Vec<ModelEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelEntry {
    pub id: String,
    pub family: String,
    pub model: PathBuf,
    #[serde(default)]
    pub purpose: Option<ModelPurpose>,
    #[serde(default)]
    pub download_id: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<PathBuf>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub quantization: Option<String>,
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
        let manifest: Self = toml::from_str(&contents)
            .with_context(|| format!("could not parse model manifest {}", path.display()))?;
        let mut ids = std::collections::HashSet::new();
        for entry in &manifest.models {
            if entry.id.trim().is_empty() || !ids.insert(&entry.id) {
                bail!("empty or duplicate model ID in {}", path.display());
            }
        }
        Ok(manifest)
    }

    pub fn find(&self, model_id: &str) -> Result<ModelEntry> {
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
                anyhow!("unknown model `{model_id}`; available models: {available}")
            })
    }
}

impl ModelEntry {
    /// Unknown legacy families stay unknown rather than becoming transcribers.
    pub fn purpose(&self) -> Option<ModelPurpose> {
        self.purpose.or(match self.family.as_str() {
            "whisper" | "zipformer" => Some(ModelPurpose::Transcript),
            _ => None,
        })
    }

    pub fn resolve_artifact(&self, manifest_path: &Path) -> PathBuf {
        resolve_artifact(manifest_path, &self.model)
    }

    /// All required local files, including shared STT tokenizer/token artifacts.
    pub fn artifact_paths(&self, manifest_path: &Path) -> Vec<PathBuf> {
        std::iter::once(&self.model)
            .chain(self.tokenizer.iter())
            .chain(self.tokens.iter())
            .map(|path| resolve_artifact(manifest_path, path))
            .collect()
    }

    /// Metadata validation only; the direct script remains the target allowlist.
    pub fn validated_download_id(&self) -> Result<&str> {
        let id = self
            .download_id
            .as_deref()
            .ok_or_else(|| anyhow!("model `{}` has no download_id", self.id))?;
        if !id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
            || !id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'.'))
        {
            bail!("model `{}` has an unsafe download_id", self.id);
        }
        let repository = self.repository.as_deref().unwrap_or_default();
        if repository.split('/').count() != 2
            || !repository
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'/' | b'-' | b'_' | b'.'))
            || !hex_digest(self.revision.as_deref(), 40)
            || !hex_digest(self.sha256.as_deref(), 64)
        {
            bail!(
                "model `{}` lacks complete pinned repository/revision/checksum metadata",
                self.id
            );
        }
        if self.tokenizer.is_some() && !hex_digest(self.tokenizer_sha256.as_deref(), 64)
            || self.tokens.is_some() && !hex_digest(self.tokens_sha256.as_deref(), 64)
            || self.family == "zipformer" && (self.tokenizer.is_none() || self.tokens.is_none())
        {
            bail!("model `{}` lacks complete bundle checksums", self.id);
        }
        if self.purpose().is_none() {
            bail!("model `{}` has unknown purpose", self.id);
        }
        if self.purpose() == Some(ModelPurpose::Reply)
            && (self
                .license
                .as_deref()
                .map_or(true, |s| s.trim().is_empty())
                || self
                    .quantization
                    .as_deref()
                    .map_or(true, |s| s.trim().is_empty())
                || self.model_size_bytes.map_or(true, |size| size == 0)
                || self.system_prompt.is_none())
        {
            bail!(
                "reply model `{}` lacks license, quantization, size or role metadata",
                self.id
            );
        }
        Ok(id)
    }

    /// The manifest binding is relative to its directory. A trusted host override
    /// is a host-supplied path, not a user message and not manifest-relative.
    pub fn load_prompt(
        &self,
        manifest_path: &Path,
        override_path: Option<&Path>,
    ) -> Result<LoadedPrompt> {
        if self.purpose() != Some(ModelPurpose::Reply) {
            bail!("model `{}` is not a reply model", self.id);
        }
        let path =
            match override_path {
                Some(path) => path.to_owned(),
                None => manifest_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(self.system_prompt.as_ref().ok_or_else(|| {
                        anyhow!("reply model `{}` has no system prompt", self.id)
                    })?),
            };
        load_prompt(&path)
    }
}

fn hex_digest(value: Option<&str>, size: usize) -> bool {
    value.is_some_and(|s| s.len() == size && s.bytes().all(|c| c.is_ascii_hexdigit()))
}

/// Preserve the old workspace-prefixed paths while accepting manifest-relative
/// paths in new entries. This does not search for or move legacy weights.
pub fn resolve_artifact(manifest_path: &Path, artifact: &Path) -> PathBuf {
    if artifact.is_absolute() {
        return artifact.to_owned();
    }
    let parent = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    if parent.file_name().is_some_and(|name| {
        artifact
            .components()
            .next()
            .is_some_and(|c| c.as_os_str() == name)
    }) {
        parent.parent().unwrap_or(parent).join(artifact)
    } else {
        parent.join(artifact)
    }
}

#[derive(Clone, Debug)]
pub struct LoadedPrompt {
    pub path: PathBuf,
    pub text: String,
    /// SHA-256 of the exact UTF-8 bytes; no whitespace normalization.
    pub sha256: String,
}

pub fn load_prompt(path: &Path) -> Result<LoadedPrompt> {
    load_prompt_files(&[path.to_owned()])
}

/// Compose trusted host role files in caller-selected order, without normalizing
/// their bytes. Exact `\n\n` separators count toward the aggregate character limit.
/// `LoadedPrompt.path` identifies the first source, not the entire source list.
pub fn load_prompt_files(paths: &[PathBuf]) -> Result<LoadedPrompt> {
    if paths.is_empty() || paths.len() > MAX_PROMPT_FILES {
        bail!("select between 1 and {MAX_PROMPT_FILES} role prompt files");
    }
    let mut remaining_chars = MAX_PROMPT_CHARS - 2 * (paths.len() - 1);
    let mut text = String::new();
    for (index, path) in paths.iter().enumerate() {
        let part = read_prompt(path, remaining_chars)
            .with_context(|| format!("invalid role prompt {}", path.display()))?;
        remaining_chars -= part.chars().count();
        if index > 0 {
            text.push_str("\n\n");
        }
        text.push_str(&part);
    }
    let sha256 = format!("{:x}", Sha256::digest(text.as_bytes()));
    Ok(LoadedPrompt {
        path: paths[0].clone(),
        text,
        sha256,
    })
}

fn read_prompt(path: &Path, max_chars: usize) -> Result<String> {
    // Check before opening as well, so a FIFO/device cannot block a startup read.
    if !std::fs::metadata(path)
        .with_context(|| format!("could not inspect role prompt {}", path.display()))?
        .is_file()
    {
        bail!("role prompt {} is not a regular file", path.display());
    }
    let file = std::fs::File::open(path)
        .with_context(|| format!("could not open role prompt {}", path.display()))?;
    if !file.metadata()?.is_file() {
        bail!("role prompt {} is not a regular file", path.display());
    }
    // Each valid read consumes the shared budget. Even a changing/oversized file
    // cannot make aggregate reads exceed MAX_PROMPT_CHARS * 4 + one sentinel byte.
    let max_bytes = max_chars * 4;
    let mut bytes = Vec::new();
    file.take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("could not read role prompt {}", path.display()))?;
    if bytes.len() > max_bytes {
        bail!("combined role prompt exceeds {MAX_PROMPT_CHARS} characters");
    }
    let text = String::from_utf8(bytes).context("role prompt is not valid UTF-8")?;
    if text.trim().is_empty()
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        bail!("role prompt must be non-empty bounded text without control characters");
    }
    if text.chars().count() > max_chars {
        bail!("combined role prompt exceeds {MAX_PROMPT_CHARS} characters");
    }
    Ok(text)
}

/// Local choices for the next server startup, never an active-model declaration.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct StartupChoices {
    pub stt_model: Option<String>,
    pub reply_model: Option<String>,
    #[serde(default)]
    pub reply_prompt_files: Vec<PathBuf>,
}

impl StartupChoices {
    /// Relative role paths are resolved against this choices file's directory,
    /// not the process working directory. Resolution does not require files to exist.
    pub fn load(path: &Path) -> Result<Self> {
        let mut choices: Self = toml::from_str(
            &std::fs::read_to_string(path)
                .with_context(|| format!("could not read startup choices {}", path.display()))?,
        )
        .context("invalid startup choices TOML")?;
        if choices
            .reply_prompt_files
            .iter()
            .any(|path| path.is_relative())
        {
            let absolute = if path.is_absolute() {
                path.to_owned()
            } else {
                std::env::current_dir()
                    .context("could not resolve startup choices directory")?
                    .join(path)
            };
            let directory = absolute.parent().expect("choices file has a directory");
            for prompt in &mut choices.reply_prompt_files {
                if prompt.is_relative() {
                    *prompt = directory.join(&*prompt);
                }
            }
        }
        Ok(choices)
    }

    pub fn validate(&self, manifest: &ModelManifest) -> Result<()> {
        if !self.reply_prompt_files.is_empty() && self.reply_model.is_none() {
            bail!("startup role prompt files require a reply_model selection");
        }
        if self.reply_prompt_files.len() > MAX_PROMPT_FILES {
            bail!("select at most {MAX_PROMPT_FILES} role prompt files");
        }
        for (id, purpose) in [
            (&self.stt_model, ModelPurpose::Transcript),
            (&self.reply_model, ModelPurpose::Reply),
        ] {
            if let Some(id) = id {
                if manifest.find(id)?.purpose() != Some(purpose) {
                    bail!("model `{id}` has the wrong startup purpose");
                }
            }
        }
        Ok(())
    }

    /// Atomic replacement; a failed write does not corrupt existing choices.
    pub fn save(&self, path: &Path) -> Result<()> {
        let contents = toml::to_string(self)?;
        let temp = path.with_extension(format!("toml.{}.part", std::process::id()));
        let result = (|| -> Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&temp, path)?;
            Ok(())
        })();

        result.with_context(|| format!("could not save startup choices {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "pheme-registry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn resolves_both_legacy_and_relative_paths() {
        assert_eq!(
            resolve_artifact(
                Path::new("models/manifest.toml"),
                Path::new("models/transcript/whisper/m.bin")
            ),
            Path::new("models/transcript/whisper/m.bin")
        );
        assert_eq!(
            resolve_artifact(
                Path::new("/tmp/models/manifest.toml"),
                Path::new("reply/m.gguf")
            ),
            Path::new("/tmp/models/reply/m.gguf")
        );
        assert_eq!(
            resolve_artifact(
                Path::new("/tmp/models/manifest.toml"),
                Path::new("/weights/m.gguf")
            ),
            Path::new("/weights/m.gguf")
        );
    }

    #[test]
    fn only_known_legacy_families_default_to_transcript() {
        for (family, expected) in [
            ("whisper", Some(ModelPurpose::Transcript)),
            ("zipformer", Some(ModelPurpose::Transcript)),
            ("new", None),
        ] {
            let entry: ModelEntry =
                toml::from_str(&format!("id = 'm'\nfamily = '{family}'\nmodel = 'm.bin'\n"))
                    .unwrap();
            assert_eq!(entry.purpose(), expected);
        }
    }

    #[test]
    fn shipped_catalog_has_valid_pinned_downloads_and_role() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
        let manifest = ModelManifest::load(&path).unwrap();
        let script = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/download-model.sh"),
        )
        .unwrap();
        for entry in &manifest.models {
            let download_id = entry.validated_download_id().unwrap();
            assert!(
                script.contains(download_id),
                "script is missing target {download_id}"
            );
            for pinned in [
                entry.repository.as_deref(),
                entry.revision.as_deref(),
                entry.sha256.as_deref(),
                entry.tokenizer_sha256.as_deref(),
                entry.tokens_sha256.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                assert!(
                    script.contains(pinned),
                    "script/catalog pinned metadata differs for {}",
                    entry.id
                );
            }
            let purpose_dir = if entry.purpose() == Some(ModelPurpose::Reply) {
                "reply"
            } else {
                "transcript"
            };
            for artifact in entry.artifact_paths(&path) {
                assert!(artifact.starts_with(path.parent().unwrap().join(purpose_dir)));
            }
            if entry.purpose() == Some(ModelPurpose::Reply) {
                assert!(entry
                    .load_prompt(&path, None)
                    .unwrap()
                    .text
                    .contains("incident"));
            }
        }
        assert!(manifest.find("not-a-model").is_err());
    }

    #[test]
    fn prompt_validation_hash_and_override() {
        let dir = temp_dir();
        let path = dir.join("role.txt");
        std::fs::write(&path, "abc").unwrap();
        let prompt = load_prompt(&path).unwrap();
        assert_eq!(prompt.text, "abc");
        let single = load_prompt_files(std::slice::from_ref(&path)).unwrap();
        assert_eq!(single.path, prompt.path);
        assert_eq!(single.text, prompt.text);
        assert_eq!(single.sha256, prompt.sha256);
        assert_eq!(
            prompt.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let entry: ModelEntry = toml::from_str("id='reply'\nfamily='qwen2'\nmodel='m.gguf'\npurpose='reply'\nsystem_prompt='missing.txt'").unwrap();
        assert!(entry.load_prompt(&dir.join("manifest.toml"), None).is_err());
        assert_eq!(
            entry
                .load_prompt(&dir.join("manifest.toml"), Some(&path))
                .unwrap()
                .sha256,
            prompt.sha256
        );
        for bytes in [
            Vec::new(),
            b" \n".to_vec(),
            vec![0xff],
            vec![b'x'; MAX_PROMPT_CHARS + 1],
            b"x\0".to_vec(),
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(load_prompt(&path).is_err());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prompt_files_preserve_order_bytes_and_exact_separators() {
        let dir = temp_dir();
        let first = dir.join("first.txt");
        let second = dir.join("second.txt");
        std::fs::write(&first, "First α\n").unwrap();
        std::fs::write(&second, "\tSecond\r\n").unwrap();
        let prompt = load_prompt_files(&[first.clone(), second.clone()]).unwrap();
        assert_eq!(prompt.path, first);
        assert_eq!(prompt.text, "First α\n\n\n\tSecond\r\n");
        assert_eq!(
            prompt.sha256,
            format!("{:x}", Sha256::digest(prompt.text.as_bytes()))
        );
        let reversed = load_prompt_files(&[second.clone(), first.clone()]).unwrap();
        assert_eq!(reversed.path, second);
        assert_eq!(reversed.text, "\tSecond\r\n\n\nFirst α\n");
        assert_ne!(reversed.sha256, prompt.sha256);
        let repeated = load_prompt_files(&[first.clone(), first]).unwrap();
        assert_eq!(repeated.text, "First α\n\n\nFirst α\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prompt_files_reject_invalid_sources_and_excessive_file_counts() {
        let dir = temp_dir();
        let valid = dir.join("valid.txt");
        let invalid = dir.join("invalid.txt");
        std::fs::write(&valid, "valid").unwrap();
        assert!(load_prompt_files(&[]).is_err());
        assert!(load_prompt_files(&vec![valid.clone(); MAX_PROMPT_FILES + 1]).is_err());
        assert!(load_prompt_files(&vec![valid.clone(); MAX_PROMPT_FILES]).is_ok());
        for path in [dir.clone(), dir.join("missing.txt")] {
            assert!(load_prompt_files(&[valid.clone(), path]).is_err());
        }
        for bytes in [
            Vec::new(),
            b" \n\t".to_vec(),
            vec![0xff],
            b"bad\0control".to_vec(),
            b"bad\x1bcontrol".to_vec(),
            "bad\u{85}control".as_bytes().to_vec(),
            vec![b'x'; MAX_PROMPT_CHARS * 4 + 1],
        ] {
            std::fs::write(&invalid, bytes).unwrap();
            assert!(load_prompt_files(&[valid.clone(), invalid.clone()]).is_err());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prompt_files_share_character_budget_including_separators() {
        let dir = temp_dir();
        let first = dir.join("first.txt");
        let second = dir.join("second.txt");
        std::fs::write(&first, "界".repeat(MAX_PROMPT_CHARS / 2)).unwrap();
        std::fs::write(&second, "🙂".repeat(MAX_PROMPT_CHARS / 2 - 2)).unwrap();
        let paths = [first, second.clone()];
        let prompt = load_prompt_files(&paths).unwrap();
        assert_eq!(prompt.text.chars().count(), MAX_PROMPT_CHARS);
        std::fs::write(&second, "🙂".repeat(MAX_PROMPT_CHARS / 2 - 1)).unwrap();
        assert!(load_prompt(&second).is_ok());
        assert!(load_prompt_files(&paths).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn startup_choices_resolve_relative_prompts_and_accept_old_files() {
        let dir = temp_dir();
        let path = dir.join("startup.toml");
        std::fs::write(&path, "stt_model='stt'\nreply_model='reply'\n").unwrap();
        let old = StartupChoices::load(&path).unwrap();
        assert!(old.reply_prompt_files.is_empty());
        let first = dir.join("first.txt");
        let second = dir.join("second.txt");
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();
        StartupChoices {
            reply_prompt_files: vec![PathBuf::from("first.txt"), second.clone()],
            ..old
        }
        .save(&path)
        .unwrap();
        let choices = StartupChoices::load(&path).unwrap();
        assert_eq!(choices.reply_prompt_files, vec![first, second]);
        assert!(choices
            .reply_prompt_files
            .iter()
            .all(|path| path.is_absolute()));
        assert_eq!(
            load_prompt_files(&choices.reply_prompt_files).unwrap().text,
            "first\n\nsecond"
        );
        choices.save(&path).unwrap();
        assert_eq!(StartupChoices::load(&path).unwrap(), choices);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn startup_prompt_files_require_an_actual_reply_model() {
        let manifest: ModelManifest = toml::from_str(
            "[[models]]\nid='stt'\nfamily='whisper'\nmodel='stt.bin'\n\
             [[models]]\nid='reply'\nfamily='qwen2'\npurpose='reply'\nmodel='reply.gguf'\n\
             [[models]]\nid='unknown'\nfamily='new'\nmodel='unknown.bin'\n",
        )
        .unwrap();
        let mut choices = StartupChoices {
            reply_prompt_files: vec![PathBuf::from("role.txt")],
            ..Default::default()
        };
        assert!(choices.validate(&manifest).is_err());
        for id in ["stt", "unknown", "role.txt"] {
            choices.reply_model = Some(id.into());
            assert!(choices.validate(&manifest).is_err());
        }
        choices.reply_model = Some("reply".into());
        choices.validate(&manifest).unwrap();
        choices.reply_prompt_files = vec![PathBuf::from("role.txt"); MAX_PROMPT_FILES + 1];
        assert!(choices.validate(&manifest).is_err());
    }

    #[test]
    fn duplicate_ids_and_incomplete_bundle_are_rejected() {
        let dir = temp_dir();
        let path = dir.join("manifest.toml");
        std::fs::write(&path, "[[models]]\nid='m'\nfamily='whisper'\nmodel='m.bin'\n[[models]]\nid='m'\nfamily='whisper'\nmodel='m.bin'").unwrap();
        assert!(ModelManifest::load(&path).is_err());
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
        let mut entry = ModelManifest::load(&path)
            .unwrap()
            .find("zipformer-small")
            .unwrap();
        entry.tokens_sha256 = None;
        assert!(entry.validated_download_id().is_err());
        entry.download_id = Some("--help; bad".into());
        assert!(entry.validated_download_id().is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn startup_choices_roundtrip_and_purpose_validation() {
        let dir = temp_dir();
        let path = dir.join("startup.toml");
        let choices = StartupChoices {
            stt_model: Some("whisper-large-v3-turbo".into()),
            reply_model: Some("qwen2.5-1.5b-instruct-q4-k-m".into()),
            ..Default::default()
        };
        choices.save(&path).unwrap();
        assert_eq!(StartupChoices::load(&path).unwrap(), choices);
        let manifest = ModelManifest::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml"),
        )
        .unwrap();
        choices.validate(&manifest).unwrap();
        assert!(StartupChoices {
            stt_model: choices.reply_model.clone(),
            reply_model: None,
            ..Default::default()
        }
        .validate(&manifest)
        .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
