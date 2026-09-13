//! Development-checkout adapter builds. Call `restart` only after restoring the
//! terminal, joining workers, and finishing native-log capture. The caller owns the
//! warning that restarting resets conversation/history; model downloads are handled
//! separately by the allowlisted TUI download task.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{bail, Context, Result};

use super::config::{self, TuiConfig};

pub struct BuildTask {
    pub model_id: String,
    child: Option<Child>,
    binary: PathBuf,
    outcome: Option<std::result::Result<PathBuf, String>>,
    preparation: Option<BuildPreparation>,
}

struct BuildPreparation {
    workspace: PathBuf,
    target: PathBuf,
    features: Vec<&'static str>,
    output_rx: std::sync::mpsc::Receiver<std::io::Result<String>>,
    output: Option<std::io::Result<String>>,
}

impl BuildTask {
    pub fn start(family: &str, model_id: String) -> Result<Self> {
        let features = features(family)?;
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .context("CLI manifest has no workspace parent")?;
        anyhow::ensure!(
            workspace.join("Cargo.toml").is_file()
                && workspace.join("Cargo.lock").is_file()
                && workspace.join("crates/cli/Cargo.toml").is_file(),
            "adapter rebuilding requires the original development checkout at {}",
            workspace.display()
        );
        let workspace = workspace
            .canonicalize()
            .context("could not resolve workspace")?;
        let target = workspace.join("target/tui-adapters");

        // Probe asynchronously: even rustup/toolchain discovery must not block
        // the event loop. An explicit host triple overrides cross-target config.
        let mut command = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()));
        command
            .current_dir(&workspace)
            .arg("-vV")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let child =
            spawn_isolated(&mut command).context("could not query the native Rust target")?;
        let mut task = Self {
            model_id,
            child: Some(child),
            binary: PathBuf::new(),
            outcome: None,
            preparation: None,
        };
        let output_rx = probe_output(task.child.as_mut().expect("probe child exists"))?;
        task.preparation = Some(BuildPreparation {
            workspace,
            target,
            features,
            output_rx,
            output: None,
        });
        Ok(task)
    }

    /// Nonblocking; a completed task returns the same outcome on subsequent polls.
    pub fn poll(&mut self) -> Result<Option<PathBuf>> {
        if let Some(outcome) = &self.outcome {
            return match outcome {
                Ok(binary) => Ok(Some(binary.clone())),
                Err(message) => bail!("{message}"),
            };
        }
        if let Some(preparation) = self.preparation.as_mut() {
            if preparation.output.is_none() {
                preparation.output = Some(match preparation.output_rx.try_recv() {
                    Ok(output) => output,
                    Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(None),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(std::io::Error::other(
                        "Rust target reader stopped unexpectedly",
                    )),
                });
            }
        }
        let child = self.child.as_mut().context("adapter build has no child")?;
        let status = match child.try_wait() {
            Ok(None) => return Ok(None),
            Ok(Some(status)) => status,
            Err(error) => {
                self.cancel();
                self.outcome = Some(Err(format!("could not poll Cargo adapter build: {error}")));
                return self.poll();
            }
        };
        self.child.take();
        if let Some(preparation) = self.preparation.take() {
            let next = (|| -> Result<Child> {
                anyhow::ensure!(
                    status.success(),
                    "native Rust target query failed ({status})"
                );
                let output = preparation
                    .output
                    .context("missing Rust target query output")?
                    .context("could not read native Rust target")?;
                let host = native_host(&output)?;
                self.binary = preparation
                    .target
                    .join(host)
                    .join("release")
                    .join(format!("cli{}", std::env::consts::EXE_SUFFIX));
                let mut command = build_command(
                    &preparation.workspace,
                    &preparation.target,
                    &preparation.features,
                    host,
                );
                spawn_isolated(&mut command).context("could not start Cargo adapter build")
            })();
            match next {
                Ok(child) => {
                    self.child = Some(child);
                    return Ok(None);
                }
                Err(error) => {
                    self.outcome = Some(Err(format!("{error:#}")));
                    return self.poll();
                }
            }
        }
        self.outcome = Some(if !status.success() {
            Err(format!(
                "Cargo adapter build failed ({status}); see native logs"
            ))
        } else if !self.binary.is_file() {
            Err(format!(
                "Cargo succeeded but executable is missing: {}",
                self.binary.display()
            ))
        } else {
            Ok(self.binary.clone())
        });
        self.poll()
    }

    /// Kill the isolated process group immediately, then reap off the UI thread.
    /// Idempotent; cancelling an already completed build preserves its outcome.
    pub fn cancel(&mut self) {
        if let Some(mut child) = self.child.take() {
            terminate(&mut child);
            // Never wait on Cargo (or a stuck filesystem) in the event loop.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            self.outcome = Some(Err("adapter build cancelled".to_owned()));
        }
    }
}

impl Drop for BuildTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn features(family: &str) -> Result<Vec<&'static str>> {
    let requested = match family {
        "whisper" => "whisper",
        "zipformer" => "zipformer",
        _ => bail!("unsupported adapter family: {family}"),
    };
    let mut features = vec![requested];
    for (enabled, feature) in [
        (cfg!(feature = "whisper"), "whisper"),
        (cfg!(feature = "whisper-metal"), "whisper-metal"),
        (cfg!(feature = "whisper-coreml"), "whisper-coreml"),
        (cfg!(feature = "zipformer"), "zipformer"),
    ] {
        if enabled && !features.contains(&feature) {
            features.push(feature);
        }
    }
    Ok(features)
}

fn probe_output(child: &mut Child) -> Result<std::sync::mpsc::Receiver<std::io::Result<String>>> {
    let stdout = child
        .stdout
        .take()
        .context("missing Rust target query stdout")?;
    let (sender, receiver) = std::sync::mpsc::channel();
    // A compiler wrapper may leave descendants holding stdout. Never read that
    // pipe on the UI thread, even after the immediate child has exited.
    std::thread::Builder::new()
        .name("adapter-host-query".into())
        .spawn(move || {
            let mut output = String::new();
            let result = stdout
                .take(16 * 1024)
                .read_to_string(&mut output)
                .map(|_| output);
            let _ = sender.send(result);
        })
        .context("could not start Rust target output reader")?;
    Ok(receiver)
}

fn native_host(output: &str) -> Result<&str> {
    let host = output
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .context("Rust compiler did not report its native host target")?;
    anyhow::ensure!(
        !host.is_empty()
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            && host != "."
            && host != "..",
        "Rust compiler reported an invalid host target"
    );
    Ok(host)
}

fn build_command(workspace: &Path, target: &Path, features: &[&str], host: &str) -> Command {
    let mut command = Command::new("cargo");
    command
        .current_dir(workspace)
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            "cli",
            "--color",
            "never",
        ])
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(target)
        // An empty build.target array builds *nothing*. Use the compiler's host
        // explicitly; Cargo writes target/<host>/release/cli in this mode.
        .args(["--target", host, "--features"])
        .arg(features.join(","))
        .env_remove("CARGO_BUILD_TARGET")
        .env("TERM", "dumb")
        .env("CARGO_TERM_COLOR", "never")
        .env("CARGO_TERM_PROGRESS_WHEN", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    command
}

fn spawn_isolated(command: &mut Command) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn()
}

fn terminate(child: &mut Child) {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: std::ffi::c_int, signal: std::ffi::c_int) -> std::ffi::c_int;
        }
        // SAFETY: spawn_isolated makes the child's PID its PGID. The child has
        // not been reaped, so that ID cannot have been reused by another group.
        // SIGKILL (9 on Unix) prevents descendants ignoring TERM from surviving.
        unsafe {
            kill(-(child.id() as std::ffi::c_int), 9);
        }
    }
    // Also handles non-Unix hosts and a failed group signal without waiting.
    let _ = child.kill();
}

/// Saves effective settings and replaces this process on Unix. Must be invoked
/// only after the caller has cleaned up its terminal, threads, and log capture.
/// On other platforms waits for the replacement and reports a nonzero status.
pub fn restart(binary: &Path, config: &TuiConfig, model_id: &str) -> Result<()> {
    let binary = binary
        .canonicalize()
        .with_context(|| format!("could not resolve rebuilt executable {}", binary.display()))?;
    anyhow::ensure!(binary.is_file(), "rebuilt executable is not a file");
    let mut effective = config.clone();
    effective.selected_stt_model = model_id.to_owned();
    config::save(&effective).context("could not save settings before TUI restart")?;
    let mut command = restart_command(&binary, &effective, model_id);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec()).context("could not replace process with rebuilt TUI")
    }
    #[cfg(not(unix))]
    {
        let status = command.status().context("could not relaunch rebuilt TUI")?;
        anyhow::ensure!(status.success(), "rebuilt TUI failed ({status})");
        Ok(())
    }
}

fn restart_command(binary: &Path, config: &TuiConfig, model_id: &str) -> Command {
    let mut command = Command::new(binary);
    // No current_dir: relative manifest/audio paths retain their original meaning.
    // Explicit global arguments override inherited PHEME_VA_* environment values.
    command
        .arg("--stt-model")
        .arg(model_id)
        .arg("--model-manifest")
        .arg(&config.model_manifest)
        .arg("tui");
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_and_compiled_feature_union() {
        for invalid in [
            "",
            "Whisper",
            "whisper,zipformer",
            "--all-features",
            "whisper;exit",
        ] {
            assert!(features(invalid).is_err());
        }
        for family in ["whisper", "zipformer"] {
            let selected = features(family).unwrap();
            assert!(selected.contains(&family));
            for (enabled, feature) in [
                (cfg!(feature = "whisper"), "whisper"),
                (cfg!(feature = "whisper-metal"), "whisper-metal"),
                (cfg!(feature = "whisper-coreml"), "whisper-coreml"),
                (cfg!(feature = "zipformer"), "zipformer"),
            ] {
                if enabled {
                    assert!(selected.contains(&feature));
                }
                assert!(selected.iter().filter(|value| **value == feature).count() <= 1);
            }
        }
    }

    #[test]
    fn cargo_command_is_native_isolated_and_colorless() {
        let workspace = Path::new("/checkout with spaces");
        let target = workspace.join("target/tui-adapters");
        let command = build_command(
            workspace,
            &target,
            &["zipformer", "whisper-metal"],
            "x86_64-unknown-linux-gnu",
        );
        assert_eq!(command.get_program(), "cargo");
        assert_eq!(command.get_current_dir(), Some(workspace));
        let args: Vec<_> = command
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "build",
                "--release",
                "--locked",
                "-p",
                "cli",
                "--color",
                "never",
                "--manifest-path",
                "/checkout with spaces/Cargo.toml",
                "--target-dir",
                "/checkout with spaces/target/tui-adapters",
                "--target",
                "x86_64-unknown-linux-gnu",
                "--features",
                "zipformer,whisper-metal"
            ]
        );
        let env: Vec<_> = command.get_envs().collect();
        assert!(env.contains(&(std::ffi::OsStr::new("CARGO_BUILD_TARGET"), None)));
        assert!(env.contains(&(
            std::ffi::OsStr::new("TERM"),
            Some(std::ffi::OsStr::new("dumb"))
        )));
    }

    #[test]
    fn parses_only_a_safe_explicit_native_target() {
        assert_eq!(
            native_host("rustc 1.98.1\nhost: x86_64-unknown-linux-gnu\nrelease: 1.98.1").unwrap(),
            "x86_64-unknown-linux-gnu"
        );
        for output in [
            "",
            "host: ",
            "host: ../other",
            "host: ..",
            "host: /tmp/target",
            "host: --target other",
        ] {
            assert!(native_host(output).is_err());
        }
    }

    #[test]
    fn restart_overrides_model_and_manifest_without_changing_cwd() {
        let config = TuiConfig {
            model_manifest: PathBuf::from("relative manifest.toml"),
            ..TuiConfig::default()
        };
        let command = restart_command(Path::new("/rebuilt/cli"), &config, "selected-model");
        assert_eq!(command.get_current_dir(), None);
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "--stt-model",
                "selected-model",
                "--model-manifest",
                "relative manifest.toml",
                "tui"
            ]
        );
    }

    // A fake Cargo tree made from this test executable, never a shell or a build.
    #[test]
    #[ignore = "subprocess fixture used by cancellation tests"]
    fn subprocess_fixture() {
        match std::env::var("PHEME_REBUILD_TEST_CHILD").as_deref() {
            Ok("parent") => {
                let mut command = fixture_command("leaf");
                let mut child = command.spawn().unwrap();
                let _ = child.wait();
            }
            Ok("failure") => std::process::exit(17),
            Ok("leaf") => {
                println!("ready");
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                }
            }
            _ => {}
        }
    }

    fn fixture_command(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--ignored", "--nocapture", "--exact"])
            .arg(format!(
                "{}::subprocess_fixture",
                module_path!().split_once("::").unwrap().1
            ))
            .env("PHEME_REBUILD_TEST_CHILD", mode);
        command
    }

    #[test]
    fn failed_or_invalid_host_probe_never_starts_cargo() {
        use std::time::{Duration, Instant};
        for mode in ["failure", "success"] {
            let mut command = fixture_command(mode);
            command.stdout(Stdio::piped()).stderr(Stdio::null());
            let mut child = spawn_isolated(&mut command).unwrap();
            let output_rx = probe_output(&mut child).unwrap();
            let mut task = BuildTask {
                model_id: "fake".into(),
                child: Some(child),
                binary: PathBuf::new(),
                outcome: None,
                preparation: Some(BuildPreparation {
                    workspace: PathBuf::new(),
                    target: PathBuf::new(),
                    features: vec!["whisper"],
                    output_rx,
                    output: None,
                }),
            };
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match task.poll() {
                    Ok(None) => {
                        assert!(Instant::now() < deadline, "probe fixture did not finish");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => {
                        assert!(error.to_string().contains(if mode == "failure" {
                            "native Rust target query failed"
                        } else {
                            "did not report its native host"
                        }));
                        assert!(task.child.is_none());
                        break;
                    }
                    Ok(Some(_)) => panic!("probe was mistaken for a completed build"),
                }
            }
        }
    }

    #[test]
    fn poll_reports_success_failure_and_missing_binary() {
        use std::time::{Duration, Instant};
        for (mode, present) in [("success", true), ("failure", true), ("success", false)] {
            let mut command = fixture_command(mode);
            command.stdout(Stdio::null()).stderr(Stdio::null());
            let binary = if present {
                std::env::current_exe().unwrap()
            } else {
                // A file cannot have children, so this path cannot exist.
                std::env::current_exe().unwrap().join("missing")
            };
            let mut task = BuildTask {
                model_id: "fake".into(),
                child: Some(spawn_isolated(&mut command).unwrap()),
                binary: binary.clone(),
                outcome: None,
                preparation: None,
            };
            let deadline = Instant::now() + Duration::from_secs(5);
            let result = loop {
                let result = task.poll();
                if !matches!(result, Ok(None)) {
                    break result;
                }
                assert!(Instant::now() < deadline, "fixture did not finish");
                std::thread::sleep(Duration::from_millis(5));
            };
            if mode == "success" && present {
                assert_eq!(result.unwrap(), Some(binary.clone()));
                assert_eq!(task.poll().unwrap(), Some(binary));
            } else {
                let message = result.unwrap_err().to_string();
                assert!(message.contains(if present { "failed" } else { "missing" }));
                assert_eq!(task.poll().unwrap_err().to_string(), message);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn cancel_and_drop_kill_descendants_without_waiting() {
        use std::io::{BufRead, BufReader, Read};
        use std::time::{Duration, Instant};
        for drop_task in [false, true] {
            let mut command = fixture_command("parent");
            command.stdout(Stdio::piped()).stderr(Stdio::null());
            let mut child = spawn_isolated(&mut command).unwrap();
            let stdout = child.stdout.take().unwrap();
            let mut task = BuildTask {
                model_id: "fake".into(),
                child: Some(child),
                binary: PathBuf::from("unused"),
                outcome: None,
                preparation: None,
            };
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let reader = std::thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap() != 0 {
                    if line.trim() == "ready" {
                        ready_tx.send(()).unwrap();
                        break;
                    }
                    line.clear();
                }
                let mut rest = Vec::new();
                reader.read_to_end(&mut rest).unwrap();
                let _ = done_tx.send(());
            });
            ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(task.poll().unwrap().is_none());
            let start = Instant::now();
            if drop_task {
                drop(task);
            } else {
                task.cancel();
                task.cancel();
                assert!(task.poll().unwrap_err().to_string().contains("cancelled"));
            }
            assert!(start.elapsed() < Duration::from_secs(1));
            // EOF requires both parent and grandchild to close their pipe handles.
            done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            reader.join().unwrap();
        }
    }
}
