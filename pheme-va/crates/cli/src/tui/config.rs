use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TuiConfig {
    pub selected_stt_model: String,
    #[serde(default)]
    pub server_stt_model: Option<String>,
    #[serde(default)]
    pub server_reply_model: Option<String>,
    #[serde(default)]
    pub reply_role_files: BTreeMap<String, Vec<PathBuf>>,
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
            server_stt_model: None,
            server_reply_model: None,
            reply_role_files: BTreeMap::new(),
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
        Ok(contents) => parse_config(&contents, &path).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .with_context(|| format!("could not read TUI configuration {}", path.display())),
    }
}

fn parse_config(contents: &str, path: &Path) -> Result<TuiConfig> {
    let mut config: TuiConfig = toml::from_str(contents)
        .with_context(|| format!("could not parse TUI configuration {}", path.display()))?;
    let parent = path
        .parent()
        .context("TUI configuration path has no parent")?;
    let base = fs::canonicalize(parent).with_context(|| {
        format!(
            "could not resolve configuration directory {}",
            parent.display()
        )
    })?;
    for role in config.reply_role_files.values_mut().flatten() {
        if role.is_relative() {
            *role = base.join(&*role);
        }
    }
    Ok(config)
}

pub fn save(config: &TuiConfig) -> Result<()> {
    save_to(config, &config_path())
}

pub(super) fn save_to(config: &TuiConfig, path: &Path) -> Result<()> {
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
    fs::rename(&temporary, path).with_context(|| {
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

pub fn history_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from) {
        if path.is_absolute() {
            return Ok(path.join("pheme-va/tui-history.json"));
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .context("TUI history requires an absolute XDG_STATE_HOME or HOME")?;
    Ok(home.join(".local/state/pheme-va/tui-history.json"))
}

pub fn default_manifest_path() -> PathBuf {
    PathBuf::from("models/manifest.toml")
}

pub fn default_audio_directory() -> PathBuf {
    if Path::new("pheme-va").is_dir() {
        PathBuf::from("pheme-va/samples")
    } else {
        PathBuf::from("samples")
    }
}

fn default_max_seconds() -> u32 {
    120
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_configuration_defaults_to_no_reply_role_overrides() {
        let config: TuiConfig = toml::from_str(
            "selected_stt_model='fixture'\nmodel_manifest='models/manifest.toml'\naudio_directory='samples'",
        ).unwrap();
        assert!(config.reply_role_files.is_empty());
        assert!(config.server_reply_model.is_none());
    }

    #[test]
    fn relative_roles_resolve_against_saved_configuration_directory() {
        let path = std::env::temp_dir().join("tui.toml");
        let config = parse_config("selected_stt_model='fixture'\nmodel_manifest='models/manifest.toml'\naudio_directory='samples'\n[reply_role_files]\nreply=['roles/base.txt', '/tmp/style.txt']", &path).unwrap();
        assert_eq!(
            config.reply_role_files["reply"],
            vec![
                fs::canonicalize(std::env::temp_dir())
                    .unwrap()
                    .join("roles/base.txt"),
                PathBuf::from("/tmp/style.txt"),
            ]
        );
    }

    #[test]
    fn reply_role_overrides_round_trip_per_model_with_absolute_paths() {
        let config = TuiConfig {
            reply_role_files: [
                (
                    "reply-a".into(),
                    vec![
                        PathBuf::from("/tmp/base role.txt"),
                        PathBuf::from("/tmp/style.txt"),
                    ],
                ),
                ("reply-b".into(), vec![PathBuf::from("/tmp/other.txt")]),
            ]
            .into_iter()
            .collect(),
            ..TuiConfig::default()
        };
        let serialized = toml::to_string_pretty(&config).unwrap();
        let restored: TuiConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(restored.reply_role_files, config.reply_role_files);
        assert!(restored
            .reply_role_files
            .values()
            .flatten()
            .all(|path| path.is_absolute()));
    }
}
