use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use va_core::registry::{ModelEntry, ModelManifest, ModelPurpose, StartupChoices};
use va_core::{ConversationConfig, ConversationModel, Engine, LoadedPrompt, Transcriber};

use crate::Args;
use va_runtime::loader::{load_stt, load_whisper};

pub struct Runtimes {
    pub engine: Option<Engine>,
    pub reply: Option<Box<dyn ConversationModel>>,
    pub prompt: Option<LoadedPrompt>,
    pub conversation: ConversationConfig,
    pub stt_load_ms: Option<f64>,
    pub reply_load_ms: Option<f64>,
}

pub fn load(args: &Args) -> Result<Runtimes> {
    ensure!(args.threads > 0, "--threads must be positive");
    ensure!(
        args.max_seconds > 0
            && args.max_review_seconds > 0
            && args.max_generation_seconds > 0
            && args.max_transcription_seconds > 0,
        "timeouts and audio duration must be positive"
    );
    let conversation = ConversationConfig {
        max_input_chars: args.max_input_chars,
        max_output_chars: args.max_output_chars,
        max_history_turns: args.max_history_turns,
        max_output_tokens: args.max_output_tokens,
        context_size: args.context_size,
        temperature: args.temperature,
    };
    conversation.validate()?;
    println!("reply settings: llama.cpp bindings 0.1.158; threads {}; context {}; max output {} tokens / {} characters; temperature {}; GPU layers {}", args.threads, conversation.context_size, conversation.max_output_tokens, conversation.max_output_chars, conversation.temperature, args.gpu_layers);
    let saved = saved_choices(args.startup_choices.as_deref())?;
    let (stt_id, reply_id) = effective_choices(args, &saved)?;
    let manifest = if stt_id.is_some() || reply_id.is_some() {
        Some(ModelManifest::load(&args.model_manifest)?)
    } else {
        None
    };
    let entry = |id: &str, purpose| -> Result<ModelEntry> {
        let model = manifest
            .as_ref()
            .expect("manifest loaded for selected IDs")
            .find(id)?;
        ensure!(
            model.purpose() == Some(purpose),
            "model `{id}` has the wrong purpose"
        );
        Ok(model)
    };
    let reply_entry = reply_id
        .as_deref()
        .map(|id| entry(id, ModelPurpose::Reply))
        .transpose()?;
    let selected_prompt = load_selected_prompt(args, &saved, reply_entry.as_ref(), &conversation)?;
    if let Some((prompt, sources)) = &selected_prompt {
        println!(
            "bound role sources (ordered): {:?} (sha256 {})",
            sources, prompt.sha256
        );
    }
    let prompt = selected_prompt.map(|(prompt, _)| prompt);
    let mut stt_load_ms = None;
    let engine = if let Some(id) = stt_id {
        let model = entry(&id, ModelPurpose::Transcript)?;
        supported_stt(&model.family)?;
        ensure_artifacts(&model, &args.model_manifest)?;
        println!("selected STT: {id}");
        let started = std::time::Instant::now();
        let engine = engine(load_stt(model, &args.model_manifest, args.threads)?, args);
        stt_load_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
        Some(engine)
    } else if let Some(path) = &args.model {
        ensure!(path.is_file(), "STT path is missing: {}", path.display());
        println!("selected STT path: {}", path.display());
        let started = std::time::Instant::now();
        let engine = engine(load_whisper(path, None, args.threads)?, args);
        stt_load_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
        Some(engine)
    } else {
        None
    };
    let mut reply_load_ms = None;
    let reply = if let Some(model) = reply_entry {
        let id = &model.id;
        ensure!(
            model.runtime.as_deref() == Some("llama.cpp")
                && model.model.extension().is_some_and(|ext| ext == "gguf"),
            "model `{id}` has no supported reply runtime"
        );
        ensure!(
            reply_model::worker_path()?.is_file(),
            "reply worker is missing; build/ship it alongside the server with `cargo build --release -p server -p reply-native --features server/whisper`, or set PHEME_VA_REPLY_WORKER"
        );
        ensure_artifacts(&model, &args.model_manifest)?;
        println!("selected reply: {id}");
        let reply = va_runtime::loader::load_reply(
            &model.resolve_artifact(&args.model_manifest),
            &conversation,
            args.threads,
            args.gpu_layers,
        )?;
        reply_load_ms = Some(reply.load_time_ms() as f64);
        println!(
            "reply model load/warmup: {} ms; native worker PID {}",
            reply.load_time_ms(),
            reply.worker_pid()
        );
        Some(Box::new(reply) as Box<dyn ConversationModel>)
    } else if let Some(path) = &args.reply_path {
        let reply =
            va_runtime::loader::load_reply(path, &conversation, args.threads, args.gpu_layers)?;
        reply_load_ms = Some(reply.load_time_ms() as f64);
        println!(
            "selected reply path: {}; load/warmup: {} ms; native worker PID {}",
            path.display(),
            reply.load_time_ms(),
            reply.worker_pid()
        );
        Some(Box::new(reply) as Box<dyn ConversationModel>)
    } else {
        None
    };
    Ok(Runtimes {
        engine,
        reply,
        prompt,
        conversation,
        stt_load_ms,
        reply_load_ms,
    })
}

fn effective_choices(
    args: &Args,
    saved: &StartupChoices,
) -> Result<(Option<String>, Option<String>)> {
    ensure!(
        args.model.is_none() || args.stt_model.is_none(),
        "--model conflicts with --stt-model"
    );
    ensure!(
        args.reply_path.is_none() || args.reply_model.is_none(),
        "--reply-path conflicts with --reply-model"
    );
    ensure!(
        args.system_prompt.is_none() || args.prompt_files.is_empty(),
        "--system-prompt conflicts with --prompt-file"
    );
    ensure!(
        saved.reply_prompt_files.is_empty() || saved.reply_model.is_some(),
        "saved role prompt files require a reply_model selection"
    );
    let stt_id = if args.model.is_some() {
        None
    } else {
        args.stt_model.clone().or_else(|| saved.stt_model.clone())
    };
    let reply_id = if args.reply_path.is_some() {
        None
    } else {
        args.reply_model
            .clone()
            .or_else(|| saved.reply_model.clone())
    };
    ensure!(
        reply_id.is_some()
            || args.reply_path.is_some()
            || (args.system_prompt.is_none() && args.prompt_files.is_empty()),
        "--system-prompt/--prompt-file require a selected reply model/path"
    );
    ensure!(
        args.reply_path.is_none() || args.system_prompt.is_some() || !args.prompt_files.is_empty(),
        "--reply-path requires --system-prompt or --prompt-file; no generic role fallback"
    );
    Ok((stt_id, reply_id))
}

/// Prompt validation happens before either native runtime is loaded. Saved files
/// belong only to their saved reply ID; raw paths never inherit that binding.
fn load_selected_prompt(
    args: &Args,
    saved: &StartupChoices,
    model: Option<&ModelEntry>,
    conversation: &ConversationConfig,
) -> Result<Option<(LoadedPrompt, Vec<PathBuf>)>> {
    if let Some(model) = model {
        ensure!(
            model.purpose() == Some(ModelPurpose::Reply),
            "model `{}` has the wrong purpose",
            model.id
        );
    }
    let sources = if let Some(path) = &args.system_prompt {
        vec![path.clone()]
    } else if !args.prompt_files.is_empty() {
        args.prompt_files.clone()
    } else if args.reply_path.is_some() {
        bail!("--reply-path requires --system-prompt or --prompt-file; no generic role fallback");
    } else if let Some(model) = model {
        if saved.reply_model.as_deref() == Some(model.id.as_str())
            && !saved.reply_prompt_files.is_empty()
        {
            saved.reply_prompt_files.clone()
        } else {
            vec![args
                .model_manifest
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(
                    model.system_prompt.as_ref().with_context(|| {
                        format!("reply model `{}` has no system prompt", model.id)
                    })?,
                )]
        }
    } else {
        return Ok(None);
    };
    ensure!(
        model.is_some() || args.reply_path.is_some(),
        "--system-prompt/--prompt-file require a selected reply model/path"
    );
    let prompt = va_core::load_prompt_files(&sources)?;
    ensure!(
        prompt.text.chars().count() < conversation.max_input_chars,
        "role consumes the full aggregate input budget"
    );
    Ok(Some((prompt, sources)))
}

fn saved_choices(explicit: Option<&Path>) -> Result<StartupChoices> {
    if let Some(path) = explicit {
        return StartupChoices::load(path);
    }
    // The TUI persists its model pair and per-reply-model roles in XDG settings.
    let path = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".pheme-va"));
    load_tui_choices(&path.join("pheme-va/tui.toml"))
}

fn load_tui_choices(path: &Path) -> Result<StartupChoices> {
    #[derive(Default, Deserialize)]
    #[serde(default)]
    struct TuiChoices {
        server_stt_model: Option<String>,
        server_reply_model: Option<String>,
        reply_role_files: BTreeMap<String, Vec<PathBuf>>,
    }
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let choices: TuiChoices = toml::from_str(&text)
                .with_context(|| format!("invalid saved TUI choices {}", path.display()))?;
            let mut reply_prompt_files = choices
                .server_reply_model
                .as_ref()
                .and_then(|id| choices.reply_role_files.get(id))
                .cloned()
                .unwrap_or_default();
            // The TUI writes absolute paths. Resolve hand-edited relative paths
            // against tui.toml as well, using the direct startup-file rule.
            if reply_prompt_files.iter().any(|path| path.is_relative()) {
                let absolute = if path.is_absolute() {
                    path.to_owned()
                } else {
                    std::env::current_dir()
                        .context("could not resolve saved TUI choices directory")?
                        .join(path)
                };
                let directory = absolute.parent().expect("TUI choices file has a directory");
                for prompt in &mut reply_prompt_files {
                    if prompt.is_relative() {
                        *prompt = directory.join(&*prompt);
                    }
                }
            }
            Ok(StartupChoices {
                stt_model: choices.server_stt_model,
                reply_model: choices.server_reply_model,
                reply_prompt_files,
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(StartupChoices::default()),
        Err(error) => Err(error).context("could not read saved TUI choices"),
    }
}

fn ensure_artifacts(entry: &ModelEntry, manifest: &Path) -> Result<()> {
    if entry
        .artifact_paths(manifest)
        .iter()
        .all(|path| path.is_file())
    {
        return Ok(());
    }
    let target = entry.validated_download_id()?;
    ensure!(
        entry
            .license
            .as_deref()
            .is_some_and(|license| !license.trim().is_empty()),
        "selected download has no recorded license"
    );
    let root = manifest.parent().unwrap_or_else(|| Path::new("."));
    let workspace = root.parent().unwrap_or_else(|| Path::new("."));
    let script = workspace.join("scripts/download-model.sh");
    ensure!(
        script.is_file(),
        "startup downloader is missing: {}",
        script.display()
    );
    println!("explicit startup selection {}: downloading missing artifacts (license {}; source {}@{}; expected bytes {:?})", entry.id, entry.license.as_deref().unwrap_or("unavailable"), entry.repository.as_deref().unwrap_or("unavailable"), entry.revision.as_deref().unwrap_or("unavailable"), entry.model_size_bytes);
    let status = std::process::Command::new("bash")
        .arg(&script)
        .arg(target)
        .env("PHEME_VA_MODEL_DIR", root)
        .status()
        .with_context(|| format!("could not invoke {}", script.display()))?;
    ensure!(status.success(), "download failed for {}", entry.id);
    ensure!(
        entry
            .artifact_paths(manifest)
            .iter()
            .all(|path| path.is_file()),
        "download did not install the selected artifact bundle for {}",
        entry.id
    );
    Ok(())
}

fn engine(transcriber: Box<dyn Transcriber>, args: &Args) -> Engine {
    let config = va_core::EngineConfig {
        max_audio_seconds: Some(args.max_seconds),
        language: args.language.clone(),
        dictionary: va_core::DictionaryHints::with_terms(args.dictionary.clone()),
        decoder: va_core::DecoderConfig {
            threads: args.threads,
            ..Default::default()
        },
        ..Default::default()
    };
    Engine::with_boxed_config(transcriber, config).with_cleaner(va_core::RuleBasedFormatter)
}

fn supported_stt(family: &str) -> Result<()> {
    match family {
        "whisper" if cfg!(feature = "whisper") => Ok(()),
        "zipformer" if cfg!(feature = "zipformer") => Ok(()),
        "whisper" | "zipformer" => bail!("selected STT adapter `{family}` is not compiled; rebuild server with --features {family}"),
        _ => bail!("unsupported STT family `{family}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(".fixtures")
                .join(crate::new_run_id());
            std::fs::create_dir_all(path.join("models")).unwrap();
            std::fs::create_dir_all(path.join("scripts")).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture_entry() -> ModelEntry {
        toml::from_str(&format!("id='fixture'\nfamily='whisper'\npurpose='transcript'\nmodel='transcript/fixture/model.bin'\ntokenizer='transcript/fixture/tokenizer.bin'\ntokens='transcript/fixture/tokens.txt'\ndownload_id='fixture-target'\nlicense='mit'\nrepository='org/repo'\nrevision='{}'\nsha256='{}'\ntokenizer_sha256='{}'\ntokens_sha256='{}'", "a".repeat(40), "b".repeat(64), "c".repeat(64), "d".repeat(64))).unwrap()
    }

    #[test]
    fn startup_download_uses_validated_argument_and_checks_complete_bundle() {
        let fixture = Fixture::new();
        let manifest = fixture.0.join("models/manifest.toml");
        let script = fixture.0.join("scripts/download-model.sh");
        std::fs::write(&script, "set -eu\ntest \"$1\" = fixture-target\nprintf '%s' \"$1\" > \"$PHEME_VA_MODEL_DIR/invocation\"\nmkdir -p \"$PHEME_VA_MODEL_DIR/transcript/fixture\"\nfor file in model.bin tokenizer.bin tokens.txt; do printf fixture > \"$PHEME_VA_MODEL_DIR/transcript/fixture/$file\"; done\n").unwrap();
        let entry = fixture_entry();
        ensure_artifacts(&entry, &manifest).unwrap();
        assert_eq!(
            std::fs::read_to_string(fixture.0.join("models/invocation")).unwrap(),
            "fixture-target"
        );
        std::fs::remove_file(script).unwrap();
        ensure_artifacts(&entry, &manifest).unwrap(); // Existing bundle never invokes a script.
        std::fs::remove_file(entry.artifact_paths(&manifest)[1].clone()).unwrap();
        assert!(ensure_artifacts(&entry, &manifest).is_err());
    }

    #[test]
    fn startup_download_refuses_incomplete_metadata_and_saved_file_is_explicit() {
        let fixture = Fixture::new();
        let manifest = fixture.0.join("models/manifest.toml");
        let mut entry = fixture_entry();
        entry.license = None;
        assert!(ensure_artifacts(&entry, &manifest)
            .unwrap_err()
            .to_string()
            .contains("license"));
        entry.license = Some("mit".into());
        entry.download_id = Some("fixture; touch arbitrary".into());
        assert!(ensure_artifacts(&entry, &manifest)
            .unwrap_err()
            .to_string()
            .contains("unsafe"));
        let path = fixture.0.join("startup.toml");
        std::fs::write(
            &path,
            "stt_model='chosen-stt'\nreply_model='chosen-reply'\n",
        )
        .unwrap();
        assert_eq!(
            saved_choices(Some(&path)).unwrap(),
            StartupChoices {
                stt_model: Some("chosen-stt".into()),
                reply_model: Some("chosen-reply".into()),
                ..Default::default()
            }
        );
    }

    #[test]
    fn saved_pair_is_used_and_each_explicit_slot_overrides_it() {
        let saved = StartupChoices {
            stt_model: Some("stt".into()),
            reply_model: Some("reply".into()),
            ..Default::default()
        };
        let args = Args::try_parse_from(["server"]).unwrap();
        assert_eq!(
            effective_choices(&args, &saved).unwrap(),
            (saved.stt_model.clone(), saved.reply_model.clone())
        );
        let args = Args::try_parse_from(["server", "--model", "local.bin", "--reply-model", "new"])
            .unwrap();
        assert_eq!(
            effective_choices(&args, &saved).unwrap(),
            (None, Some("new".into()))
        );
    }

    fn reply_entry(id: &str) -> ModelEntry {
        toml::from_str(&format!(
            "id='{id}'\nfamily='qwen2'\npurpose='reply'\nmodel='reply/missing.gguf'\n\
             runtime='llama.cpp'\nsystem_prompt='default-role.txt'\n"
        ))
        .unwrap()
    }

    fn prompt_args(fixture: &Fixture, flags: &[&str]) -> Args {
        let mut args = Args::try_parse_from(flags).unwrap();
        args.model_manifest = fixture.0.join("models/manifest.toml");
        args
    }

    fn select_prompt(
        args: &Args,
        saved: &StartupChoices,
    ) -> Result<Option<(LoadedPrompt, Vec<PathBuf>)>> {
        let (_, id) = effective_choices(args, saved)?;
        let model = id.as_deref().map(reply_entry);
        load_selected_prompt(args, saved, model.as_ref(), &ConversationConfig::default())
    }

    #[test]
    fn saved_roles_follow_only_the_matching_effective_reply_id() {
        let fixture = Fixture::new();
        let first = fixture.0.join("first.txt");
        let second = fixture.0.join("second.txt");
        let default = fixture.0.join("models/default-role.txt");
        std::fs::write(&first, "alpha").unwrap();
        std::fs::write(&second, "beta").unwrap();
        std::fs::write(&default, "manifest default").unwrap();
        let mut saved = StartupChoices {
            reply_model: Some("saved".into()),
            reply_prompt_files: vec![first.clone(), second.clone()],
            ..Default::default()
        };
        for flags in [vec!["server"], vec!["server", "--reply-model", "saved"]] {
            let args = prompt_args(&fixture, &flags);
            let (prompt, sources) = select_prompt(&args, &saved).unwrap().unwrap();
            assert_eq!(sources, vec![first.clone(), second.clone()]);
            assert_eq!(prompt.path, first);
            assert_eq!(prompt.text, "alpha\n\nbeta");
            assert_eq!(
                prompt.sha256,
                "67c7f39a47ef6ce0009b2dfe644d6f1625530f95a7a030d05ce8e5eb90921566"
            );
        }
        // A different explicit ID must not even read the stale binding.
        saved.reply_prompt_files = vec![fixture.0.join("missing-stale-role.txt")];
        let args = prompt_args(&fixture, &["server", "--reply-model", "different"]);
        assert_eq!(
            effective_choices(&args, &saved).unwrap().1,
            Some("different".into())
        );
        let (prompt, sources) = select_prompt(&args, &saved).unwrap().unwrap();
        assert_eq!(sources, vec![default]);
        assert_eq!(prompt.text, "manifest default");
        let args = prompt_args(&fixture, &["server", "--reply-path", "missing.gguf"]);
        assert!(effective_choices(&args, &saved)
            .unwrap_err()
            .to_string()
            .contains("requires --system-prompt or --prompt-file"));
    }

    #[test]
    fn explicit_prompt_flags_override_saved_files_and_manifest_defaults() {
        let fixture = Fixture::new();
        let first = fixture.0.join("first.txt");
        let second = fixture.0.join("second.txt");
        let reference = fixture.0.join("reference.txt");
        std::fs::write(&first, "alpha").unwrap();
        std::fs::write(&second, "beta").unwrap();
        std::fs::write(&reference, "alpha\n\nbeta").unwrap();
        let saved = StartupChoices {
            reply_model: Some("saved".into()),
            reply_prompt_files: vec![fixture.0.join("missing-stale-role.txt")],
            ..Default::default()
        };
        for flags in [
            vec!["server", "--reply-model", "saved"],
            vec!["server", "--reply-model", "different"],
            vec!["server", "--reply-path", "missing.gguf"],
        ] {
            let mut args = prompt_args(&fixture, &flags);
            args.prompt_files = vec![first.clone(), second.clone()];
            let (prompt, sources) = select_prompt(&args, &saved).unwrap().unwrap();
            assert_eq!(sources, args.prompt_files);
            assert_eq!(prompt.text, "alpha\n\nbeta");
            assert_eq!(
                prompt.sha256,
                va_core::load_prompt(&reference).unwrap().sha256
            );
            args.prompt_files.clear();
            args.system_prompt = Some(second.clone());
            let (prompt, sources) = select_prompt(&args, &saved).unwrap().unwrap();
            assert_eq!(sources, vec![second.clone()]);
            assert_eq!(prompt.text, "beta");
            assert_eq!(prompt.sha256, va_core::load_prompt(&second).unwrap().sha256);
        }
    }

    #[test]
    fn explicit_relative_prompt_paths_are_host_relative_not_manifest_relative() {
        let relative_dir = PathBuf::from(".fixtures").join(crate::new_run_id());
        let fixture = Fixture(std::env::current_dir().unwrap().join(&relative_dir));
        std::fs::create_dir_all(fixture.0.join("models")).unwrap();
        let first = relative_dir.join("first.txt");
        let second = relative_dir.join("second.txt");
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();
        let mut args = prompt_args(&fixture, &["server", "--reply-model", "reply"]);
        args.prompt_files = vec![first.clone(), second.clone()];
        let (prompt, sources) = select_prompt(&args, &StartupChoices::default())
            .unwrap()
            .unwrap();
        assert_eq!(sources, vec![first.clone(), second.clone()]);
        assert_eq!(prompt.path, first);
        assert_eq!(prompt.text, "first\n\nsecond");
        args.prompt_files.clear();
        args.system_prompt = Some(second.clone());
        let (prompt, sources) = select_prompt(&args, &StartupChoices::default())
            .unwrap()
            .unwrap();
        assert_eq!(sources, vec![second]);
        assert_eq!(prompt.text, "second");
    }

    #[test]
    fn orphaned_saved_role_files_are_not_silently_ignored() {
        let args = Args::try_parse_from(["server"]).unwrap();
        let saved = StartupChoices {
            reply_prompt_files: vec![PathBuf::from("role.txt")],
            ..Default::default()
        };
        assert!(effective_choices(&args, &saved)
            .unwrap_err()
            .to_string()
            .contains("require a reply_model selection"));
    }

    #[test]
    fn startup_relative_roles_are_choices_relative_not_manifest_relative() {
        let fixture = Fixture::new();
        let path = fixture.0.join("startup.toml");
        let first = fixture.0.join("first.txt");
        let second = fixture.0.join("second.txt");
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();
        std::fs::write(
            &path,
            "reply_model='saved'\nreply_prompt_files=['second.txt','first.txt']\n",
        )
        .unwrap();
        let saved = saved_choices(Some(&path)).unwrap();
        assert_eq!(
            saved.reply_prompt_files,
            vec![second.clone(), first.clone()]
        );
        let args = prompt_args(&fixture, &["server"]);
        let (prompt, sources) = select_prompt(&args, &saved).unwrap().unwrap();
        assert_eq!(sources, vec![second, first]);
        assert_eq!(prompt.text, "second\n\nfirst");
    }

    #[test]
    fn tui_roles_map_only_the_saved_reply_model_and_old_toml_is_compatible() {
        #[derive(serde::Serialize)]
        struct TuiFixture {
            server_stt_model: String,
            server_reply_model: Option<String>,
            reply_role_files: BTreeMap<String, Vec<PathBuf>>,
        }
        let fixture = Fixture::new();
        let path = fixture.0.join("tui.toml");
        std::fs::write(
            &path,
            "server_stt_model='stt'\nserver_reply_model='saved'\n",
        )
        .unwrap();
        let old = load_tui_choices(&path).unwrap();
        assert_eq!(old.stt_model.as_deref(), Some("stt"));
        assert_eq!(old.reply_model.as_deref(), Some("saved"));
        assert!(old.reply_prompt_files.is_empty());
        let first = fixture.0.join("first.txt");
        let second = fixture.0.join("second.txt");
        let selected = vec![second, first];
        let mut choices = TuiFixture {
            server_stt_model: "stt".into(),
            server_reply_model: Some("saved".into()),
            reply_role_files: BTreeMap::from([
                ("saved".into(), selected.clone()),
                ("other".into(), vec![fixture.0.join("other.txt")]),
            ]),
        };
        std::fs::write(&path, toml::to_string(&choices).unwrap()).unwrap();
        assert_eq!(
            load_tui_choices(&path).unwrap().reply_prompt_files,
            selected
        );
        choices.server_reply_model = Some("no-custom-role".into());
        std::fs::write(&path, toml::to_string(&choices).unwrap()).unwrap();
        assert!(load_tui_choices(&path)
            .unwrap()
            .reply_prompt_files
            .is_empty());
        choices.server_reply_model = None;
        std::fs::write(&path, toml::to_string(&choices).unwrap()).unwrap();
        assert!(load_tui_choices(&path)
            .unwrap()
            .reply_prompt_files
            .is_empty());
    }

    #[test]
    fn hand_edited_tui_relative_role_paths_use_tui_file_directory() {
        let fixture = Fixture::new();
        let path = fixture.0.join("tui.toml");
        std::fs::write(
            &path,
            "server_reply_model='saved'\n[reply_role_files]\nsaved=['second.txt','first.txt']\n",
        )
        .unwrap();
        assert_eq!(
            load_tui_choices(&path).unwrap().reply_prompt_files,
            vec![fixture.0.join("second.txt"), fixture.0.join("first.txt")]
        );
    }

    #[test]
    fn prompt_selection_keeps_manifest_default_and_checks_model_purpose() {
        let fixture = Fixture::new();
        let default = fixture.0.join("models/default-role.txt");
        std::fs::write(&default, "manifest default").unwrap();
        let args = prompt_args(&fixture, &["server", "--reply-model", "reply"]);
        let saved = StartupChoices::default();
        let (prompt, sources) = select_prompt(&args, &saved).unwrap().unwrap();
        assert_eq!(sources, vec![default]);
        assert_eq!(prompt.text, "manifest default");
        assert!(load_selected_prompt(
            &args,
            &saved,
            Some(&fixture_entry()),
            &ConversationConfig::default()
        )
        .is_err());
        let args = prompt_args(&fixture, &["server"]);
        assert!(select_prompt(&args, &saved).unwrap().is_none());
    }

    #[test]
    fn prompt_errors_and_aggregate_budget_are_checked_before_native_loading() {
        let fixture = Fixture::new();
        let path = fixture.0.join("startup.toml");
        StartupChoices::default().save(&path).unwrap();
        for flag in ["--prompt-file", "--system-prompt"] {
            let mut args = prompt_args(
                &fixture,
                &["server", "--model", "missing.bin", flag, "missing.txt"],
            );
            args.startup_choices = Some(path.clone());
            let error = load(&args).err().unwrap().to_string();
            assert!(
                error.contains("require a selected reply model/path"),
                "{error}"
            );
        }
        let first = fixture.0.join("first.txt");
        let second = fixture.0.join("second.txt");
        std::fs::write(&first, "abc").unwrap();
        std::fs::write(&second, "def").unwrap();
        let mut args = prompt_args(
            &fixture,
            &[
                "server",
                "--model",
                "missing.bin",
                "--reply-path",
                "missing.gguf",
            ],
        );
        args.startup_choices = Some(path);
        args.prompt_files = vec![first, second];
        args.max_input_chars = 8; // Three + two separator characters + three.
        let error = load(&args).err().unwrap().to_string();
        assert!(error.contains("full aggregate input budget"), "{error}");
        let conversation = ConversationConfig {
            max_input_chars: 9,
            ..Default::default()
        };
        assert!(
            load_selected_prompt(&args, &StartupChoices::default(), None, &conversation)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn conflicting_paths_and_ids_are_rejected() {
        for flags in [
            vec!["server", "--model", "local", "--stt-model", "id"],
            vec!["server", "--reply-path", "local", "--reply-model", "id"],
        ] {
            assert!(Args::try_parse_from(flags).is_err());
        }
    }
}
