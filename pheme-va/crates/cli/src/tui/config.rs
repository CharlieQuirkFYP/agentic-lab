use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TuiConfig {
    pub selected_stt_model: String,
    pub model_manifest: PathBuf,
    pub audio_directory: PathBuf,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub dictionary: Vec<String>,
    #[serde(default = "default_max_seconds")]
    pub max_seconds: u32,
    #[serde(default = "default_true")]
    pub metrics_enabled: bool,
    #[serde(default = "default_true")]
    pub resource_sampling_enabled: bool,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            selected_stt_model: "whisper-large-v3-turbo".to_owned(),
            model_manifest: default_manifest_path(),
            audio_directory: default_audio_directory(),
            language: None,
            dictionary: Vec::new(),
            max_seconds: default_max_seconds(),
            metrics_enabled: true,
            resource_sampling_enabled: true,
        }
    }
}

pub fn load() -> Result<Option<TuiConfig>> {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(contents) => toml::from_str(&contents)
            .with_context(|| format!("could not parse TUI configuration {}", path.display()))
            .map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .with_context(|| format!("could not read TUI configuration {}", path.display())),
    }
}

pub fn save(config: &TuiConfig) -> Result<()> {
    let path = config_path();
    let parent = path
        .parent()
        .context("TUI configuration path has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "could not create configuration directory {}",
            parent.display()
        )
    })?;

    let contents =
        toml::to_string_pretty(config).context("could not serialize TUI configuration")?;
    let temporary = path.with_extension(format!("toml.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)
        .with_context(|| {
            format!(
                "could not open temporary configuration {}",
                temporary.display()
            )
        })?;
    file.write_all(contents.as_bytes())
        .context("could not write TUI configuration")?;
    file.sync_all()
        .context("could not flush TUI configuration")?;
    drop(file);
    fs::rename(&temporary, &path).with_context(|| {
        format!(
            "could not replace TUI configuration {} with {}",
            path.display(),
            temporary.display()
        )
    })?;
    Ok(())
}

pub fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("pheme-va").join("tui.toml");
    }
    if cfg!(windows) {
        if let Some(path) = std::env::var_os("APPDATA") {
            return PathBuf::from(path).join("pheme-va").join("tui.toml");
        }
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join(".config").join("pheme-va").join("tui.toml"))
        .unwrap_or_else(|| PathBuf::from(".pheme-va").join("tui.toml"))
}

pub fn default_manifest_path() -> PathBuf {
    PathBuf::from("models/manifest.toml")
}

pub fn default_audio_directory() -> PathBuf {
    if Path::new("pheme-va").is_dir() {
        PathBuf::from("pheme-va/local/audio")
    } else {
        PathBuf::from("local/audio")
    }
}

fn default_max_seconds() -> u32 {
    120
}

fn default_true() -> bool {
    true
}
