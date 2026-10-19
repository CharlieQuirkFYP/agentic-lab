use std::path::PathBuf;

use crate::model::{self, ModelEntry, ModelManifest};

#[derive(Clone, Debug)]
pub struct CatalogEntry {
    pub manifest: ModelEntry,
    pub model_path: PathBuf,

    pub missing_paths: Vec<PathBuf>,
    pub adapter_compiled: bool,
}

impl CatalogEntry {
    pub fn artifacts_available(&self) -> bool {
        self.missing_paths.is_empty()
    }

    pub fn selectable(&self) -> bool {
        self.manifest.purpose() == Some(model::ModelPurpose::Transcript)
            && self.adapter_compiled
            && self.artifacts_available()
    }
}

#[derive(Clone, Debug)]
pub struct ModelCatalog {
    pub manifest_path: PathBuf,
    pub entries: Vec<CatalogEntry>,
    pub error: Option<String>,
}

impl ModelCatalog {
    pub fn load(manifest_path: PathBuf) -> Self {
        match ModelManifest::load(&manifest_path) {
            Ok(manifest) => Self::from_manifest(manifest_path, manifest),
            Err(error) => Self {
                manifest_path,
                entries: Vec::new(),
                error: Some(error.to_string()),
            },
        }
    }

    fn from_manifest(manifest_path: PathBuf, manifest: ModelManifest) -> Self {
        let mut entries = manifest
            .models
            .into_iter()
            .map(|model| {
                let mut required_paths =
                    vec![model::resolve_artifact(&manifest_path, &model.model)];
                if let Some(tokenizer) = model.tokenizer.as_ref() {
                    required_paths.push(model::resolve_artifact(&manifest_path, tokenizer));
                }
                if let Some(tokens) = model.tokens.as_ref() {
                    required_paths.push(model::resolve_artifact(&manifest_path, tokens));
                }
                let missing_paths = required_paths
                    .iter()
                    .filter(|path| !path.is_file())
                    .cloned()
                    .collect::<Vec<_>>();
                let model_path = required_paths[0].clone();
                let adapter_compiled = model::family_compiled(&model.family);
                CatalogEntry {
                    manifest: model,
                    model_path,
                    missing_paths,
                    adapter_compiled,
                }
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| match entry.manifest.purpose() {
            Some(model::ModelPurpose::Transcript) => 0,
            Some(model::ModelPurpose::Reply) => 1,
            None => 2,
        });
        Self {
            manifest_path,
            entries,
            error: None,
        }
    }

    pub fn refresh(&mut self) {
        *self = Self::load(self.manifest_path.clone());
    }

    pub fn selected_index(&self, model_id: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.manifest.id == model_id)
    }

    pub fn entry(&self, index: usize) -> Option<&CatalogEntry> {
        self.entries.get(index)
    }

    pub fn entry_by_id(&self, model_id: &str) -> Option<&CatalogEntry> {
        self.entries
            .iter()
            .find(|entry| entry.manifest.id == model_id)
    }

    pub fn missing_summary(entry: &CatalogEntry) -> String {
        entry
            .missing_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn groups_by_manifest_purpose_and_preserves_order_within_each_group() {
        let manifest = toml::from_str(
            r#"
            [[models]]
            id = "reply-first"
            family = "whisper"
            purpose = "reply"
            model = "reply-first.gguf"
            [[models]]
            id = "unclassified"
            family = "unknown"
            model = "unknown.bin"
            [[models]]
            id = "transcript-first"
            family = "custom-stt"
            purpose = "transcript"
            model = "first.bin"
            [[models]]
            id = "reply-second"
            family = "custom-llm"
            purpose = "reply"
            model = "reply-second.gguf"
            [[models]]
            id = "legacy-transcript"
            family = "zipformer"
            model = "legacy.bin"
            [[models]]
            id = "transcript-last"
            family = "custom-stt"
            purpose = "transcript"
            model = "last.bin"
            "#,
        )
        .unwrap();
        let catalog = ModelCatalog::from_manifest("manifest.toml".into(), manifest);
        let ids = catalog
            .entries
            .iter()
            .map(|entry| entry.manifest.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "transcript-first",
                "legacy-transcript",
                "transcript-last",
                "reply-first",
                "reply-second",
                "unclassified",
            ]
        );
        assert_eq!(catalog.selected_index("reply-first"), Some(3));
        assert_eq!(catalog.entry(3).unwrap().manifest.id, "reply-first");
        assert!(!catalog.entry(3).unwrap().selectable());
    }

    #[test]
    fn classifies_artifacts_separately_from_compiled_adapters() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after the Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("pheme-va-catalog-test-{suffix}"));
        fs::create_dir_all(&directory).unwrap();
        let manifest = directory.join("manifest.toml");
        fs::write(
            &manifest,
            r#"[[models]]
            id = "fixture"
            family = "unknown"
            model = "fixture.bin"
            "#,
        )
        .unwrap();
        fs::write(directory.join("fixture.bin"), b"fixture").unwrap();

        let catalog = ModelCatalog::load(manifest);
        let entry = catalog.entry(0).expect("manifest entry should exist");
        assert!(entry.artifacts_available());
        assert!(!entry.adapter_compiled);
        assert!(!entry.selectable());
        assert!(!model::family_supported(&entry.manifest.family));

        fs::remove_dir_all(directory).unwrap();
    }
}
