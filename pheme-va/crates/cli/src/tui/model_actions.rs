use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use anyhow::{ensure, Context, Result};
use ratatui::crossterm::event::KeyCode;
use sha2::{Digest, Sha256};

use crate::model::{self, LoadedPrompt, ModelEntry, ModelPurpose, StartupChoices};

use super::config::{self, TuiConfig};
use super::model_catalog::{CatalogEntry, ModelCatalog};

#[derive(Default)]
pub struct Group {
    pub filter: String,
    pub highlighted: Option<String>,
}

pub enum Confirmation {
    Download(String),
    Choose(String),
}

#[derive(Clone, Debug)]
pub struct RoleSelection {
    pub model_id: String,
    pub directory: PathBuf,
    pub entries: Vec<PathBuf>,
    pub selected_files: Vec<PathBuf>,
    pub cursor: usize,
    pub adding_path: bool,
    pub path_input: String,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct ModelActions {
    group: Group,
    pub role_selection: Option<RoleSelection>,
    pub filtering: bool,
    pub confirmation: Option<Confirmation>,
    pub preview: Option<LoadedPrompt>,
    pub scroll: u16,
    pub command: bool,
    pub download_status: HashMap<String, String>,
    pub verification: Option<Verification>,
    pub verified_choice: Option<ModelEntry>,
}

impl ModelActions {
    pub fn group(&self) -> &Group {
        &self.group
    }
    pub fn group_mut(&mut self) -> &mut Group {
        &mut self.group
    }

    pub fn rows<'a>(&self, catalog: &'a ModelCatalog) -> Vec<&'a CatalogEntry> {
        let query = self.group().filter.to_lowercase();
        catalog
            .entries
            .iter()
            .filter(|entry| {
                entry.manifest.id.to_lowercase().contains(&query)
                    || entry.manifest.family.to_lowercase().contains(&query)
            })
            .collect()
    }

    pub fn selected<'a>(&self, catalog: &'a ModelCatalog) -> Option<&'a CatalogEntry> {
        let rows = self.rows(catalog);
        rows.iter()
            .find(|entry| Some(&entry.manifest.id) == self.group().highlighted.as_ref())
            .copied()
            .or_else(|| rows.first().copied())
    }

    pub fn move_selection(&mut self, catalog: &ModelCatalog, delta: isize) {
        let rows = self.rows(catalog);
        if rows.is_empty() {
            return;
        }
        let selected = rows
            .iter()
            .position(|entry| Some(&entry.manifest.id) == self.group().highlighted.as_ref())
            .unwrap_or(0);
        let next = (selected as isize + delta).clamp(0, rows.len() as isize - 1) as usize;
        self.group_mut().highlighted = Some(rows[next].manifest.id.clone());
        self.scroll = 0;
    }

    pub fn preview(&mut self, catalog: &ModelCatalog, config: &TuiConfig) -> Result<()> {
        let entry = self.selected(catalog).context("no selected model")?;
        ensure!(
            entry.manifest.purpose() == Some(ModelPurpose::Reply),
            "transcription models have no conversational role"
        );
        let paths = role_paths_for(&entry.manifest, &catalog.manifest_path, config)?;
        self.preview = Some(load_role_files(&paths)?);
        self.scroll = 0;
        Ok(())
    }

    pub fn open_roles(&mut self, catalog: &ModelCatalog, config: &TuiConfig) -> Result<()> {
        let entry = &self
            .selected(catalog)
            .context("no selected model")?
            .manifest;
        let default = default_role_path(entry, &catalog.manifest_path)?;
        let directory = default
            .parent()
            .context("role path has no parent")?
            .to_owned();
        let mut entries = Vec::new();
        for item in std::fs::read_dir(&directory)
            .with_context(|| format!("could not browse roles in {}", directory.display()))?
        {
            let path = item?.path();
            if path.extension().is_some_and(|extension| extension == "txt") && path.is_file() {
                entries.push(std::fs::canonicalize(path)?);
            }
        }
        let mut selected_files = role_paths_for(entry, &catalog.manifest_path, config)?;
        for path in &mut selected_files {
            if path.is_file() {
                *path = std::fs::canonicalize(&*path)?;
            }
        }
        dedup_role_files(&mut selected_files);
        entries.extend(selected_files.iter().cloned());
        entries.sort();
        entries.dedup();
        let cursor = entries
            .iter()
            .position(|path| selected_files.contains(path))
            .unwrap_or(0);
        self.role_selection = Some(RoleSelection {
            model_id: entry.id.clone(),
            directory,
            entries,
            selected_files,
            cursor,
            adding_path: false,
            path_input: String::new(),
            error: None,
        });
        self.scroll = 0;
        Ok(())
    }

    pub fn handle_roles_key(&mut self, code: KeyCode, config: &mut TuiConfig) -> Result<()> {
        self.handle_roles_key_with_save(code, config, config::save)
    }

    fn handle_roles_key_with_save(
        &mut self,
        code: KeyCode,
        config: &mut TuiConfig,
        save: impl FnOnce(&TuiConfig) -> Result<()>,
    ) -> Result<()> {
        let Some(roles) = self.role_selection.as_mut() else {
            return Ok(());
        };
        let mut close = false;
        let result = (|| -> Result<()> {
            if roles.adding_path {
                match code {
                    KeyCode::Esc => {
                        roles.adding_path = false;
                        roles.path_input.clear();
                        roles.error = None;
                    }
                    KeyCode::Char(ch) if roles.path_input.len() + ch.len_utf8() <= 4096 => {
                        roles.path_input.push(ch);
                    }
                    KeyCode::Backspace => {
                        roles.path_input.pop();
                    }
                    KeyCode::Enter => {
                        ensure!(
                            !roles.path_input.trim().is_empty(),
                            "enter a local .txt role path"
                        );
                        let path = std::fs::canonicalize(roles.path_input.trim())
                            .context("could not resolve local role path")?;
                        load_role_files(std::slice::from_ref(&path))?;
                        roles.entries.push(path.clone());
                        roles.entries.sort();
                        roles.entries.dedup();
                        roles.selected_files.push(path.clone());
                        dedup_role_files(&mut roles.selected_files);
                        roles.cursor = roles
                            .entries
                            .iter()
                            .position(|entry| entry == &path)
                            .unwrap_or(0);
                        roles.adding_path = false;
                        roles.path_input.clear();
                        roles.error = None;
                    }
                    _ => {}
                }
                return Ok(());
            }
            match code {
                KeyCode::Esc => close = true,
                KeyCode::Up | KeyCode::Char('k') => roles.cursor = roles.cursor.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    roles.cursor = (roles.cursor + 1).min(roles.entries.len().saturating_sub(1));
                }
                KeyCode::Char(' ') => {
                    if let Some(path) = roles.entries.get(roles.cursor) {
                        if roles.selected_files.contains(path) {
                            roles.selected_files.retain(|selected| selected != path);
                        } else {
                            roles.selected_files.push(path.clone());
                        }
                        roles.error = None;
                    }
                }
                KeyCode::Char('a') => {
                    roles.adding_path = true;
                    roles.path_input.clear();
                    roles.error = None;
                }
                KeyCode::Enter => {
                    load_role_files(&roles.selected_files)?;
                    let mut next = config.clone();
                    next.reply_role_files
                        .insert(roles.model_id.clone(), roles.selected_files.clone());
                    save(&next)?;
                    *config = next;
                    close = true;
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(error) = &result {
            roles.error = Some(format!("{error:#}"));
        }
        if close {
            self.role_selection = None;
        }
        result
    }
}

fn dedup_role_files(paths: &mut Vec<PathBuf>) {
    let mut seen = std::collections::HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

fn default_role_path(entry: &ModelEntry, manifest: &Path) -> Result<PathBuf> {
    ensure!(
        entry.purpose() == Some(ModelPurpose::Reply),
        "transcription models have no conversational role"
    );
    let path = manifest.parent().unwrap_or_else(|| Path::new(".")).join(
        entry
            .system_prompt
            .as_ref()
            .context("reply entry has no system_prompt")?,
    );
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()
            .context("could not resolve the role file directory")?
            .join(path))
    }
}

pub fn role_paths_for(
    entry: &ModelEntry,
    manifest: &Path,
    config: &TuiConfig,
) -> Result<Vec<PathBuf>> {
    ensure!(
        entry.purpose() == Some(ModelPurpose::Reply),
        "transcription models have no conversational role"
    );
    if let Some(paths) = config.reply_role_files.get(&entry.id) {
        ensure!(
            paths.iter().all(|path| path.is_absolute()),
            "saved role paths must be absolute"
        );
        return Ok(paths.clone());
    }
    Ok(vec![default_role_path(entry, manifest)?])
}

fn load_role_files(paths: &[PathBuf]) -> Result<LoadedPrompt> {
    ensure!(
        !paths.is_empty(),
        "select at least one trusted local role file"
    );
    for path in paths {
        ensure!(path.is_absolute(), "role paths must be absolute");
        ensure!(
            path.extension().is_some_and(|extension| extension == "txt"),
            "role prompt must be a .txt file: {}",
            path.display()
        );
        ensure!(
            std::fs::metadata(path)?.is_file(),
            "role prompt is not a regular file: {}",
            path.display()
        );
    }
    let prompt = model::load_prompt_files(paths)?;
    let max_input_chars = va_core::ConversationConfig::default().max_input_chars;
    let prompt_chars = prompt.text.chars().count();
    ensure!(
        prompt_chars < max_input_chars,
        "combined role prompt has {prompt_chars} characters; it must contain fewer than {max_input_chars} characters to leave room in the server's default input budget"
    );
    Ok(prompt)
}

pub(super) fn next_start_config(
    config: &TuiConfig,
    catalog: &ModelCatalog,
    entry: &ModelEntry,
) -> Result<TuiConfig> {
    startup_supported(entry)?;
    let mut next = config.clone();
    match entry.purpose() {
        Some(ModelPurpose::Transcript) => next.server_stt_model = Some(entry.id.clone()),
        Some(ModelPurpose::Reply) => next.server_reply_model = Some(entry.id.clone()),
        None => anyhow::bail!("unknown model purpose"),
    }
    choices(&next).validate(&model::ModelManifest {
        models: catalog
            .entries
            .iter()
            .map(|entry| entry.manifest.clone())
            .collect(),
    })?;
    if let Some(id) = next.server_reply_model.as_ref() {
        let reply = &catalog
            .entry_by_id(id)
            .context("chosen reply model is not in catalog")?
            .manifest;
        load_role_files(&role_paths_for(reply, &catalog.manifest_path, &next)?)?;
    }
    Ok(next)
}

pub fn startup_supported(entry: &ModelEntry) -> Result<()> {
    match entry.purpose() {
        Some(ModelPurpose::Transcript) => ensure!(
            model::family_supported(&entry.family),
            "no supported server STT adapter for {}",
            entry.family
        ),
        Some(ModelPurpose::Reply) => ensure!(
            entry.runtime.as_deref() == Some("llama.cpp")
                && entry.model.extension().is_some_and(|ext| ext == "gguf"),
            "no known server reply adapter for {}; downloaded does not mean supported",
            entry.family
        ),
        None => anyhow::bail!("unknown model purpose"),
    }
    Ok(())
}

pub fn choices(config: &TuiConfig) -> StartupChoices {
    StartupChoices {
        stt_model: config.server_stt_model.clone(),
        reply_model: config.server_reply_model.clone(),
        reply_prompt_files: config
            .server_reply_model
            .as_ref()
            .and_then(|id| config.reply_role_files.get(id))
            .cloned()
            .unwrap_or_default(),
    }
}

pub fn startup_command(config: &TuiConfig) -> String {
    let feature = config.server_stt_model.as_deref().and_then(|id| {
        model::ModelManifest::load(&config.model_manifest)
            .and_then(|manifest| manifest.find(id))
            .ok()
            .and_then(|entry| match entry.family.as_str() {
                "whisper" => Some("server/whisper"),
                "zipformer" => Some("server/zipformer"),
                _ => None,
            })
    });
    let mut command = "cargo build --release -p server -p reply-native".to_owned();
    if let Some(feature) = feature {
        command.push_str(&format!(" --features {feature}"));
    }
    command.push_str(&format!(
        " && ./target/release/server{} --model-manifest {}",
        std::env::consts::EXE_SUFFIX,
        shell_quote(&config.model_manifest.to_string_lossy())
    ));
    if let Some(id) = config.server_stt_model.as_deref() {
        command.push_str(&format!(" --stt-model {}", shell_quote(id)));
    }
    if let Some(id) = config.server_reply_model.as_deref() {
        command.push_str(&format!(" --reply-model {}", shell_quote(id)));
        if let Some(paths) = config.reply_role_files.get(id) {
            for path in paths {
                command.push_str(&format!(
                    " --prompt-file {}",
                    shell_quote(&path.to_string_lossy())
                ));
            }
        }
    }
    command
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Hash large weights off the input/render thread, with bounded reads and cancellation.
pub struct Verification {
    pub model_id: String,
    pub entry: ModelEntry,
    pub receiver: mpsc::Receiver<Result<(), String>>,
    stop: Arc<AtomicBool>,
}

impl Verification {
    pub fn start(entry: &CatalogEntry, manifest: &Path) -> Result<Self> {
        startup_supported(&entry.manifest)?;
        ensure!(
            entry.artifacts_available(),
            "missing artifacts; download first"
        );
        let model = entry.manifest.clone();
        let path = manifest.to_owned();
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&stop);
        let verifying = model.clone();
        std::thread::spawn(move || {
            let result =
                verify_bundle(&verifying, &path, &cancelled).map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        Ok(Self {
            model_id: model.id.clone(),
            entry: model,
            receiver,
            stop,
        })
    }
}

impl Drop for Verification {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn verify_bundle(entry: &ModelEntry, manifest: &Path, stop: &AtomicBool) -> Result<()> {
    for (artifact, checksum) in std::iter::once((&entry.model, entry.sha256.as_ref()))
        .chain(
            entry
                .tokenizer
                .iter()
                .map(|path| (path, entry.tokenizer_sha256.as_ref())),
        )
        .chain(
            entry
                .tokens
                .iter()
                .map(|path| (path, entry.tokens_sha256.as_ref())),
        )
    {
        let expected = checksum.context("no checksum recorded; cannot verify startup choice")?;
        ensure!(
            expected.len() == 64 && expected.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid artifact checksum"
        );
        let path = model::resolve_artifact(manifest, artifact);
        let mut file = std::fs::File::open(&path)?;
        ensure!(file.metadata()?.is_file(), "artifact is not a regular file");
        let mut hash = Sha256::new();
        let mut buffer = [0_u8; 65_536];
        loop {
            ensure!(!stop.load(Ordering::Relaxed), "verification cancelled");
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        ensure!(
            format!("{:x}", hash.finalize()).eq_ignore_ascii_case(expected),
            "checksum mismatch for {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelManifest;

    #[test]
    fn unified_selection_and_filter_cross_purpose_groups() {
        let entry = |id: &str, family: &str, purpose: &str| -> ModelEntry {
            toml::from_str(&format!(
                "id='{id}'\nfamily='{family}'\npurpose='{purpose}'\nmodel='a.bin'"
            ))
            .unwrap()
        };
        let catalog = ModelCatalog {
            manifest_path: "m.toml".into(),
            error: None,
            entries: vec![
                entry("stt", "whisper", "transcript"),
                entry("reply", "llama", "reply"),
            ]
            .into_iter()
            .map(|manifest| CatalogEntry {
                manifest,
                model_path: "a.bin".into(),
                missing_paths: vec![],
                adapter_compiled: false,
            })
            .collect(),
        };
        let mut actions = ModelActions::default();
        assert_eq!(actions.rows(&catalog).len(), 2);
        actions.move_selection(&catalog, 1);
        assert_eq!(actions.selected(&catalog).unwrap().manifest.id, "reply");
        actions.move_selection(&catalog, -1);
        assert_eq!(actions.selected(&catalog).unwrap().manifest.id, "stt");
        actions.group_mut().filter = "LLAMA".into();
        assert_eq!(actions.rows(&catalog).len(), 1);
        assert_eq!(actions.selected(&catalog).unwrap().manifest.id, "reply");
        actions.group_mut().filter = "st".into();
        assert_eq!(actions.selected(&catalog).unwrap().manifest.id, "stt");
        actions.group_mut().filter = "no match".into();
        actions.move_selection(&catalog, 1);
        assert!(actions.selected(&catalog).is_none());
    }

    #[test]
    fn startup_choices_are_distinct_from_standalone_and_quote_ids() {
        let config = TuiConfig {
            server_stt_model: Some("stt".into()),
            server_reply_model: Some("reply's".into()),
            ..TuiConfig::default()
        };
        assert_ne!(config.selected_stt_model, "stt");
        assert!(startup_command(&config).contains("'reply'\\''s'"));
        let manifest = ModelManifest { models: vec![] };
        assert!(choices(&config).validate(&manifest).is_err());
        let serialized = toml::to_string(&config).unwrap();
        let restored: TuiConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(choices(&config), choices(&restored));
    }

    #[test]
    fn startup_command_builds_worker_and_selected_stt_adapter() {
        let mut config = TuiConfig {
            model_manifest: Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../models/manifest.toml"),
            server_stt_model: Some("whisper-large-v3-turbo".into()),
            server_reply_model: Some("qwen2.5-1.5b-instruct-q4-k-m".into()),
            ..TuiConfig::default()
        };
        let command = startup_command(&config);
        assert!(command.contains("-p server -p reply-native"));
        assert!(command.contains("--features server/whisper"));
        assert!(command.contains("./target/release/server"));
        assert!(command.contains("--reply-model 'qwen2.5-1.5b-instruct-q4-k-m'"));
        config.server_stt_model = Some("zipformer-small".into());
        assert!(startup_command(&config).contains("--features server/zipformer"));
    }

    #[test]
    fn verification_checks_entire_artifact_bundle() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        std::fs::write(root.join("weight"), b"fixture").unwrap();
        let mut entry: ModelEntry = toml::from_str(&format!(
            "id='stt'\nfamily='whisper'\nmodel='weight'\nsha256='{:x}'",
            Sha256::digest(b"fixture")
        ))
        .unwrap();
        let stop = AtomicBool::new(false);
        verify_bundle(&entry, &root.join("manifest.toml"), &stop).unwrap();
        entry.tokens = Some("missing".into());
        assert!(verify_bundle(&entry, &root.join("manifest.toml"), &stop).is_err());
    }

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let suffix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root =
                std::env::temp_dir().join(format!("pheme-role-{}-{suffix}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            Self(std::fs::canonicalize(root).unwrap())
        }

        fn catalog(&self) -> ModelCatalog {
            std::fs::write(self.0.join("incident-reporting.txt"), "Base role\n").unwrap();
            std::fs::write(self.0.join("z-style.txt"), "Use short sentences.").unwrap();
            std::fs::write(self.0.join("ignore.md"), "not a role").unwrap();
            let manifest = toml::from_str("id='reply'\nfamily='qwen2'\npurpose='reply'\nruntime='llama.cpp'\nmodel='reply.gguf'\nsystem_prompt='incident-reporting.txt'").unwrap();
            ModelCatalog {
                manifest_path: self.0.join("manifest.toml"),
                entries: vec![CatalogEntry {
                    manifest,
                    model_path: self.0.join("reply.gguf"),
                    missing_paths: vec![],
                    adapter_compiled: false,
                }],
                error: None,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn role_picker_keeps_manifest_default_first_when_selecting_an_earlier_filename() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog();
        let earlier = fixture.0.join("a-style.txt");
        std::fs::write(&earlier, "Earlier style.").unwrap();
        let mut config = TuiConfig::default();
        let mut actions = ModelActions::default();
        actions.open_roles(&catalog, &config).unwrap();
        let roles = actions.role_selection.as_ref().unwrap();
        assert_eq!(roles.directory, fixture.0);
        assert_eq!(
            roles.entries,
            vec![
                earlier.clone(),
                fixture.0.join("incident-reporting.txt"),
                fixture.0.join("z-style.txt")
            ]
        );
        assert_eq!(
            roles.selected_files,
            vec![fixture.0.join("incident-reporting.txt")]
        );
        assert!(config.reply_role_files.is_empty());
        actions.handle_roles_key(KeyCode::Up, &mut config).unwrap();
        actions
            .handle_roles_key(KeyCode::Char(' '), &mut config)
            .unwrap();
        assert_eq!(
            actions.role_selection.as_ref().unwrap().selected_files,
            vec![fixture.0.join("incident-reporting.txt"), earlier.clone()]
        );
        assert!(config.reply_role_files.is_empty());
        let path = fixture.0.join("tui.toml");
        actions
            .handle_roles_key_with_save(KeyCode::Enter, &mut config, |next| {
                config::save_to(next, &path)
            })
            .unwrap();
        assert!(actions.role_selection.is_none());
        let selected = vec![fixture.0.join("incident-reporting.txt"), earlier];
        assert_eq!(config.reply_role_files["reply"], selected);
        let restored: TuiConfig = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(restored.reply_role_files, config.reply_role_files);
        actions.preview(&catalog, &config).unwrap();
        let prompt = actions.preview.as_ref().unwrap();
        assert_eq!(prompt.text, "Base role\n\n\nEarlier style.");
        assert_eq!(
            prompt.sha256,
            format!("{:x}", Sha256::digest(prompt.text.as_bytes()))
        );
        actions.open_roles(&catalog, &restored).unwrap();
        assert_eq!(
            actions.role_selection.as_ref().unwrap().selected_files,
            selected
        );
    }

    #[test]
    fn adding_an_earlier_filename_appends_after_default_and_deduplicates_stably() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog();
        let earlier = fixture.0.join("a-added.txt");
        std::fs::write(&earlier, "Added role.").unwrap();
        let mut config = TuiConfig::default();
        let mut actions = ModelActions::default();
        actions.open_roles(&catalog, &config).unwrap();
        for _ in 0..2 {
            actions
                .handle_roles_key(KeyCode::Char('a'), &mut config)
                .unwrap();
            actions.role_selection.as_mut().unwrap().path_input =
                earlier.to_string_lossy().into_owned();
            actions
                .handle_roles_key_with_save(KeyCode::Enter, &mut config, |_| {
                    panic!("path entry must not save")
                })
                .unwrap();
            let roles = actions.role_selection.as_ref().unwrap();
            assert_eq!(
                roles.selected_files,
                vec![fixture.0.join("incident-reporting.txt"), earlier.clone()]
            );
            assert_eq!(
                roles.entries,
                vec![
                    earlier.clone(),
                    fixture.0.join("incident-reporting.txt"),
                    fixture.0.join("z-style.txt")
                ]
            );
            assert!(config.reply_role_files.is_empty());
        }
        actions
            .handle_roles_key_with_save(KeyCode::Enter, &mut config, |_| Ok(()))
            .unwrap();
        actions.preview(&catalog, &config).unwrap();
        assert_eq!(
            actions.preview.as_ref().unwrap().text,
            "Base role\n\n\nAdded role."
        );
    }

    #[test]
    fn deselecting_and_reselecting_a_role_appends_it_after_existing_selections() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog();
        let earlier = fixture.0.join("a-style.txt");
        std::fs::write(&earlier, "Earlier style.").unwrap();
        let default = fixture.0.join("incident-reporting.txt");
        let mut config = TuiConfig::default();
        let mut actions = ModelActions::default();
        actions.open_roles(&catalog, &config).unwrap();
        actions.handle_roles_key(KeyCode::Up, &mut config).unwrap();
        actions
            .handle_roles_key(KeyCode::Char(' '), &mut config)
            .unwrap();
        assert_eq!(
            actions.role_selection.as_ref().unwrap().selected_files,
            vec![default.clone(), earlier.clone()]
        );
        actions
            .handle_roles_key(KeyCode::Down, &mut config)
            .unwrap();
        actions
            .handle_roles_key(KeyCode::Char(' '), &mut config)
            .unwrap();
        assert_eq!(
            actions.role_selection.as_ref().unwrap().selected_files,
            vec![earlier.clone()]
        );
        actions
            .handle_roles_key(KeyCode::Char(' '), &mut config)
            .unwrap();
        assert_eq!(
            actions.role_selection.as_ref().unwrap().selected_files,
            vec![earlier, default]
        );
        actions
            .handle_roles_key_with_save(KeyCode::Enter, &mut config, |_| Ok(()))
            .unwrap();
        actions.preview(&catalog, &config).unwrap();
        assert_eq!(
            actions.preview.as_ref().unwrap().text,
            "Earlier style.\n\nBase role\n"
        );
    }

    #[test]
    fn reopening_saved_roles_preserves_caller_order_and_first_duplicate_occurrence() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog();
        let earlier = fixture.0.join("a-style.txt");
        std::fs::write(&earlier, "Earlier style.").unwrap();
        let default = fixture.0.join("incident-reporting.txt");
        let style = fixture.0.join("z-style.txt");
        let saved = vec![
            style.clone(),
            default.clone(),
            earlier.clone(),
            default.clone(),
            style.clone(),
        ];
        let expected = vec![style, default, earlier];
        let mut config = TuiConfig::default();
        config
            .reply_role_files
            .insert("reply".into(), saved.clone());
        let mut actions = ModelActions::default();
        actions.open_roles(&catalog, &config).unwrap();
        assert_eq!(
            actions.role_selection.as_ref().unwrap().selected_files,
            expected
        );
        assert_eq!(config.reply_role_files["reply"], saved);
        actions
            .handle_roles_key_with_save(KeyCode::Enter, &mut config, |_| Ok(()))
            .unwrap();
        assert_eq!(config.reply_role_files["reply"], expected);
        actions.open_roles(&catalog, &config).unwrap();
        assert_eq!(
            actions.role_selection.as_ref().unwrap().selected_files,
            expected
        );
        actions.preview(&catalog, &config).unwrap();
        assert_eq!(
            actions.preview.as_ref().unwrap().text,
            "Use short sentences.\n\nBase role\n\n\nEarlier style."
        );
    }

    #[test]
    fn preview_uses_per_id_override_and_preserves_single_file_bytes() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog();
        let mut config = TuiConfig::default();
        let mut actions = ModelActions::default();
        actions.preview(&catalog, &config).unwrap();
        assert_eq!(actions.preview.as_ref().unwrap().text, "Base role\n");
        config
            .reply_role_files
            .insert("other".into(), vec![fixture.0.join("z-style.txt")]);
        assert_eq!(
            role_paths_for(
                &catalog.entries[0].manifest,
                &catalog.manifest_path,
                &config
            )
            .unwrap(),
            vec![fixture.0.join("incident-reporting.txt")]
        );
        config
            .reply_role_files
            .insert("reply".into(), vec![fixture.0.join("z-style.txt")]);
        actions.preview(&catalog, &config).unwrap();
        assert_eq!(
            actions.preview.as_ref().unwrap().text,
            "Use short sentences."
        );
        actions.open_roles(&catalog, &config).unwrap();
        actions
            .handle_roles_key(KeyCode::Char(' '), &mut config)
            .unwrap();
        actions.handle_roles_key(KeyCode::Esc, &mut config).unwrap();
        assert_eq!(
            config.reply_role_files["reply"],
            vec![fixture.0.join("z-style.txt")]
        );
    }

    #[test]
    fn role_path_entry_validates_before_adding_and_never_saves_during_input() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog();
        let outside = fixture.0.join("external");
        std::fs::create_dir(&outside).unwrap();
        let valid = outside.join("local role.txt");
        std::fs::write(&valid, "Additional trusted role").unwrap();
        let invalid_utf8 = outside.join("invalid.txt");
        std::fs::write(&invalid_utf8, [0xff]).unwrap();
        let empty = outside.join("empty.txt");
        std::fs::write(&empty, " \n").unwrap();
        let oversized = outside.join("oversized.txt");
        std::fs::write(&oversized, "x".repeat(model::MAX_PROMPT_CHARS + 1)).unwrap();
        let mut actions = ModelActions::default();
        let mut config = TuiConfig::default();
        actions.open_roles(&catalog, &config).unwrap();
        actions
            .handle_roles_key(KeyCode::Char('a'), &mut config)
            .unwrap();
        for path in [
            catalog.manifest_path.clone(),
            outside.clone(),
            invalid_utf8,
            empty,
            oversized,
        ] {
            actions.role_selection.as_mut().unwrap().path_input =
                path.to_string_lossy().into_owned();
            assert!(actions
                .handle_roles_key_with_save(KeyCode::Enter, &mut config, |_| panic!(
                    "path entry must not save"
                ))
                .is_err());
            let roles = actions.role_selection.as_ref().unwrap();
            assert!(roles.adding_path);
            assert!(roles.error.is_some());
            assert_eq!(roles.selected_files.len(), 1);
        }
        actions.role_selection.as_mut().unwrap().path_input = valid.to_string_lossy().into_owned();
        actions
            .handle_roles_key_with_save(KeyCode::Enter, &mut config, |_| {
                panic!("path entry must not save")
            })
            .unwrap();
        assert!(config.reply_role_files.is_empty());
        let roles = actions.role_selection.as_ref().unwrap();
        assert!(!roles.adding_path);
        assert!(roles.error.is_none());
        assert!(roles.selected_files.contains(&valid));
        assert_eq!(roles.entries[roles.cursor], valid);
    }

    #[test]
    fn effective_role_prompt_must_be_below_the_default_server_input_budget() {
        let fixture = Fixture::new();
        let path = fixture.0.join("budget-role.txt");
        let paths = vec![path.clone()];
        let budget = va_core::ConversationConfig::default().max_input_chars;
        for chars in [budget - 1, budget, budget + 1] {
            std::fs::write(&path, "界".repeat(chars)).unwrap();
            assert_eq!(
                model::load_prompt_files(&paths)
                    .unwrap()
                    .text
                    .chars()
                    .count(),
                chars
            );
            let result = load_role_files(&paths);
            if chars < budget {
                assert_eq!(result.unwrap().text.chars().count(), chars);
            } else {
                let error = result.unwrap_err().to_string();
                assert!(error.contains(&format!("has {chars} characters")));
                assert!(error.contains(&format!("fewer than {budget} characters")));
                assert!(error.contains("server's default input budget"));
            }
        }
    }

    #[test]
    fn combined_role_budget_counts_unicode_characters_and_join_separators() {
        let fixture = Fixture::new();
        let first = fixture.0.join("first.txt");
        let second = fixture.0.join("second.txt");
        let paths = vec![first.clone(), second.clone()];
        let budget = va_core::ConversationConfig::default().max_input_chars;
        let first_chars = budget / 2;
        let second_chars = budget - first_chars - 3;
        std::fs::write(first, "界".repeat(first_chars)).unwrap();
        std::fs::write(&second, "🙂".repeat(second_chars)).unwrap();
        assert_eq!(
            load_role_files(&paths).unwrap().text.chars().count(),
            budget - 1
        );
        std::fs::write(second, "🙂".repeat(second_chars + 1)).unwrap();
        assert_eq!(
            model::load_prompt_files(&paths)
                .unwrap()
                .text
                .chars()
                .count(),
            budget
        );
        assert!(load_role_files(&paths).is_err());
    }

    #[test]
    fn oversized_roles_for_an_already_chosen_model_preserve_config_and_do_not_save() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog();
        let path = fixture.0.join("budget-role.txt");
        let budget = va_core::ConversationConfig::default().max_input_chars;
        std::fs::write(&path, "x".repeat(budget)).unwrap();
        let mut config = TuiConfig {
            server_reply_model: Some("reply".into()),
            reply_role_files: [(
                "reply".into(),
                vec![fixture.0.join("incident-reporting.txt")],
            )]
            .into_iter()
            .collect(),
            ..TuiConfig::default()
        };
        let before = toml::to_string(&config).unwrap();
        let mut actions = ModelActions::default();
        actions.open_roles(&catalog, &config).unwrap();
        actions.role_selection.as_mut().unwrap().selected_files = vec![path.clone()];
        for chars in [budget, budget + 1] {
            std::fs::write(&path, "x".repeat(chars)).unwrap();
            assert!(actions
                .handle_roles_key_with_save(KeyCode::Enter, &mut config, |_| panic!(
                    "roles exceeding the server budget must not save"
                ))
                .is_err());
            assert_eq!(toml::to_string(&config).unwrap(), before);
            assert_eq!(
                actions.role_selection.as_ref().unwrap().selected_files,
                vec![path.clone()]
            );
            assert!(actions
                .role_selection
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("server's default input budget"));
            let mut proposed = config.clone();
            proposed
                .reply_role_files
                .insert("reply".into(), vec![path.clone()]);
            assert!(next_start_config(&proposed, &catalog, &catalog.entries[0].manifest).is_err());
        }
        std::fs::write(&path, "x".repeat(budget - 1)).unwrap();
        let saved = fixture.0.join("tui.toml");
        actions
            .handle_roles_key_with_save(KeyCode::Enter, &mut config, |next| {
                config::save_to(next, &saved)
            })
            .unwrap();
        assert!(actions.role_selection.is_none());
        assert_eq!(config.reply_role_files["reply"], vec![path]);
        assert!(next_start_config(&config, &catalog, &catalog.entries[0].manifest).is_ok());
        let restored: TuiConfig = toml::from_str(&std::fs::read_to_string(saved).unwrap()).unwrap();
        assert_eq!(restored.reply_role_files, config.reply_role_files);
    }

    #[test]
    fn invalid_combination_or_failed_save_preserves_config_and_draft() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog();
        let mut actions = ModelActions::default();
        let mut config = TuiConfig::default();
        let before = toml::to_string(&config).unwrap();
        actions.open_roles(&catalog, &config).unwrap();
        actions
            .role_selection
            .as_mut()
            .unwrap()
            .selected_files
            .clear();
        assert!(actions
            .handle_roles_key_with_save(KeyCode::Enter, &mut config, |_| panic!(
                "empty draft must not save"
            ))
            .is_err());
        let a = fixture.0.join("a.txt");
        let b = fixture.0.join("b.txt");
        std::fs::write(&a, "x".repeat(model::MAX_PROMPT_CHARS / 2)).unwrap();
        std::fs::write(&b, "y".repeat(model::MAX_PROMPT_CHARS / 2)).unwrap();
        actions.role_selection.as_mut().unwrap().selected_files = vec![a, b];
        assert!(actions
            .handle_roles_key_with_save(KeyCode::Enter, &mut config, |_| panic!(
                "oversized combination must not save"
            ))
            .is_err());
        actions.role_selection.as_mut().unwrap().selected_files =
            vec![fixture.0.join("incident-reporting.txt")];
        let blocker = fixture.0.join("not-a-directory");
        std::fs::write(&blocker, "fixture").unwrap();
        assert!(actions
            .handle_roles_key_with_save(KeyCode::Enter, &mut config, |next| config::save_to(
                next,
                &blocker.join("tui.toml")
            ))
            .is_err());
        assert!(actions.role_selection.as_ref().unwrap().error.is_some());
        assert_eq!(toml::to_string(&config).unwrap(), before);
    }

    #[test]
    fn startup_choice_revalidates_saved_combination_and_default_role() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog();
        let mut config = TuiConfig::default();
        let entry = &catalog.entries[0].manifest;
        assert_eq!(
            next_start_config(&config, &catalog, entry)
                .unwrap()
                .server_reply_model
                .as_deref(),
            Some("reply")
        );
        config.reply_role_files.insert("reply".into(), vec![]);
        assert!(next_start_config(&config, &catalog, entry).is_err());
        config
            .reply_role_files
            .insert("reply".into(), vec![fixture.0.join("z-style.txt")]);
        assert!(next_start_config(&config, &catalog, entry).is_ok());
        std::fs::write(fixture.0.join("z-style.txt"), [0xff]).unwrap();
        assert!(next_start_config(&config, &catalog, entry).is_err());
        assert!(config.server_reply_model.is_none());
    }

    #[test]
    fn roles_are_reply_only_and_saved_paths_must_be_absolute() {
        let fixture = Fixture::new();
        let mut catalog = fixture.catalog();
        let mut config = TuiConfig::default();
        config
            .reply_role_files
            .insert("reply".into(), vec!["relative.txt".into()]);
        assert!(role_paths_for(
            &catalog.entries[0].manifest,
            &catalog.manifest_path,
            &config
        )
        .is_err());
        catalog.entries[0].manifest.purpose = Some(ModelPurpose::Transcript);
        let mut actions = ModelActions::default();
        assert!(actions.open_roles(&catalog, &config).is_err());
        assert!(actions.preview(&catalog, &config).is_err());
        assert!(actions.role_selection.is_none());
    }

    #[test]
    fn command_quotes_each_role_path_only_for_the_chosen_reply_id() {
        let config = TuiConfig {
            server_reply_model: Some("reply's".into()),
            reply_role_files: [
                (
                    "reply's".into(),
                    vec![
                        PathBuf::from("/tmp/base role.txt"),
                        PathBuf::from("/tmp/operator's role.txt"),
                    ],
                ),
                ("unchosen".into(), vec![PathBuf::from("/tmp/unused.txt")]),
            ]
            .into_iter()
            .collect(),
            ..TuiConfig::default()
        };
        let command = startup_command(&config);
        assert!(command.contains("--reply-model 'reply'\\''s' --prompt-file '/tmp/base role.txt' --prompt-file '/tmp/operator'\\''s role.txt'"));
        assert!(!command.contains("unused.txt"));
        assert_eq!(
            choices(&config).reply_prompt_files,
            config.reply_role_files["reply's"]
        );
        let no_reply = TuiConfig {
            server_reply_model: None,
            ..config
        };
        assert!(choices(&no_reply).reply_prompt_files.is_empty());
        assert!(!startup_command(&no_reply).contains("--prompt-file"));
    }

    #[test]
    fn valid_saved_override_does_not_require_an_unused_default_role() {
        let fixture = Fixture::new();
        let mut catalog = fixture.catalog();
        let weight = fixture.0.join("reply.gguf");
        std::fs::write(&weight, b"fixture").unwrap();
        catalog.entries[0].manifest.sha256 = Some(format!("{:x}", Sha256::digest(b"fixture")));
        let entry = &catalog.entries[0].manifest;
        let effective = fixture.0.join("z-style.txt");
        let mut config = TuiConfig::default();
        config
            .reply_role_files
            .insert("reply".into(), vec![effective.clone()]);
        let stop = AtomicBool::new(false);
        let default = fixture.0.join("incident-reporting.txt");
        std::fs::remove_file(&default).unwrap();
        verify_bundle(entry, &catalog.manifest_path, &stop).unwrap();
        let next = next_start_config(&config, &catalog, entry).unwrap();
        assert_eq!(next.server_reply_model.as_deref(), Some("reply"));
        assert_eq!(choices(&next).reply_prompt_files, vec![effective.clone()]);
        std::fs::write(default, [0xff]).unwrap();
        verify_bundle(entry, &catalog.manifest_path, &stop).unwrap();
        assert!(next_start_config(&config, &catalog, entry).is_ok());

        std::fs::write(effective, "").unwrap();
        let before = toml::to_string(&config).unwrap();
        verify_bundle(entry, &catalog.manifest_path, &stop).unwrap();
        assert!(next_start_config(&config, &catalog, entry).is_err());
        assert_eq!(toml::to_string(&config).unwrap(), before);
        std::fs::write(weight, b"tampered").unwrap();
        assert!(verify_bundle(entry, &catalog.manifest_path, &stop).is_err());
    }
}
