use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::model::ModelEntry;

use anyhow::{bail, Context, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadProgress {
    pub percent: Option<u8>,
    pub message: String,
}

enum DownloadOutput {
    Text(String),
    IoError(String),
}

pub struct DownloadTask {
    pub model_id: String,
    child: Option<Child>,
    finished: Option<(ExitStatus, Instant)>,
    output_rx: Receiver<DownloadOutput>,
    pub progress: DownloadProgress,
    output: String,
    progress_input: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DownloadOutcome {
    Completed,
}

impl DownloadTask {
    pub fn start(entry: &ModelEntry, model_dir: &Path) -> Result<Self> {
        let argument = script_download_id(entry)?;
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .context("CLI manifest has no workspace parent")?;
        let script = workspace.join("scripts/download-model.sh");
        anyhow::ensure!(
            script.is_file(),
            "model download script is missing: {}",
            script.display()
        );

        Self::start_script(entry.id.clone(), &argument, model_dir, &script)
    }

    pub(super) fn start_script(
        model_id: String,
        argument: &str,
        model_dir: &Path,
        script: &Path,
    ) -> Result<Self> {
        let mut command = Command::new("bash");
        command
            .arg(script)
            .arg(argument)
            .env("PHEME_VA_MODEL_DIR", model_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().context("could not start model download")?;
        let stdout = child
            .stdout
            .take()
            .context("download stdout was not captured")?;
        let stderr = child
            .stderr
            .take()
            .context("download stderr was not captured")?;
        let (sender, receiver) = mpsc::sync_channel(64);
        spawn_reader(stdout, sender.clone());
        spawn_reader(stderr, sender);
        Ok(Self {
            model_id,
            child: Some(child),
            finished: None,
            output_rx: receiver,
            progress: DownloadProgress {
                percent: None,
                message: "starting download".to_owned(),
            },
            output: String::new(),
            progress_input: String::new(),
        })
    }

    pub fn poll(&mut self) -> Result<Option<DownloadOutcome>> {
        let mut readers_closed = false;
        for _ in 0..64 {
            let message = match self.output_rx.try_recv() {
                Ok(message) => message,
                Err(mpsc::TryRecvError::Disconnected) => {
                    readers_closed = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            };
            match message {
                DownloadOutput::Text(text) => self.consume_output(&text),
                DownloadOutput::IoError(error) => self.output.push_str(&format!("\n{error}")),
            }
        }
        if self.finished.is_none() {
            let child = self.child.as_mut().context("download task has no child")?;
            let Some(status) = child.try_wait().context("could not poll model download")? else {
                return Ok(None);
            };
            self.child.take();
            self.finished = Some((status, Instant::now()));
        }
        let (status, exited) = self.finished.as_ref().expect("exit status recorded");
        // Reader threads can deliver final checksum diagnostics after the process exits.
        if !readers_closed && exited.elapsed() < Duration::from_secs(1) {
            return Ok(None);
        }
        if status.success() {
            self.progress.percent = Some(100);
            self.progress.message = "download verified".to_owned();
            Ok(Some(DownloadOutcome::Completed))
        } else {
            let details = self.output.trim();
            bail!(
                "model download failed ({status}){}",
                if details.is_empty() {
                    String::new()
                } else {
                    format!(": {details}")
                }
            )
        }
    }

    pub fn cancel(&mut self) -> Option<std::thread::JoinHandle<()>> {
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            // SAFETY: the unreaped child is the leader of its own process group.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGTERM);
            }
            #[cfg(not(unix))]
            let _ = child.kill();
            return Some(std::thread::spawn(move || {
                // Allow the script's TERM trap to release its shared partial-file lock.
                // Do not reap the leader before killing any surviving descendants.
                std::thread::sleep(std::time::Duration::from_millis(500));
                #[cfg(unix)]
                // SAFETY: the child remains unreaped, so its group ID cannot be reused.
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
            }));
        }
        None
    }

    pub fn output(&self) -> &str {
        &self.output
    }

    fn consume_output(&mut self, text: &str) {
        self.progress_input.push_str(text);
        if self.progress_input.len() > 512 {
            let mut start = self.progress_input.len() - 512;
            while !self.progress_input.is_char_boundary(start) {
                start += 1;
            }
            self.progress_input.drain(..start);
        }
        if let Some(percent) = parse_percent(&self.progress_input) {
            self.progress.percent = Some(percent);
        }
        let cleaned = text.replace('\r', "\n");
        for line in cleaned
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            if parse_percent(line).is_none() {
                self.progress.message = line.to_owned();
            }
            self.output.push_str(line);
            self.output.push('\n');
        }
        if self.output.len() > 8_192 {
            let mut start = self.output.len() - 8_192;
            while !self.output.is_char_boundary(start) {
                start += 1;
            }
            self.output.drain(..start);
        }
    }
}

impl Drop for DownloadTask {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}

fn spawn_reader<R>(mut reader: R, sender: mpsc::SyncSender<DownloadOutput>)
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(length) => {
                    if sender
                        .send(DownloadOutput::Text(
                            String::from_utf8_lossy(&buffer[..length]).into_owned(),
                        ))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send(DownloadOutput::IoError(error.to_string()));
                    break;
                }
            }
        }
    });
}

fn parse_percent(line: &str) -> Option<u8> {
    line.split_whitespace().find_map(|token| {
        let value = token.strip_suffix('%')?.parse::<f32>().ok()?;
        (0.0..=100.0)
            .contains(&value)
            .then_some(value.round() as u8)
    })
}

pub fn script_download_id(entry: &ModelEntry) -> Result<String> {
    let mut entry = entry.clone();
    if entry.download_id.is_none() {
        entry.download_id = match entry.id.as_str() {
            "whisper-large-v3-turbo" => Some("whisper".into()),
            "zipformer-small" | "zipformer-medium" | "zipformer-large" => Some(entry.id.clone()),
            _ => None,
        };
    }
    let target = entry.validated_download_id()?;
    anyhow::ensure!(
        target
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric),
        "download_id must be a model target, not a script flag"
    );
    Ok(target.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_model_ids_are_downloadable() {
        let mut entry: ModelEntry = toml::from_str(&format!(
            "id='whisper-large-v3-turbo'\nfamily='whisper'\nmodel='w.bin'\nrepository='org/repo'\nrevision='{}'\nsha256='{}'", "a".repeat(40), "b".repeat(64)
        )).unwrap();
        assert_eq!(script_download_id(&entry).unwrap(), "whisper");
        entry.download_id = Some("new-pinned-target".into());
        assert_eq!(script_download_id(&entry).unwrap(), "new-pinned-target");
        entry.download_id = Some("--help".into());
        assert!(script_download_id(&entry).is_err());
        entry.download_id = Some("whisper;rm -rf /".into());
        assert!(script_download_id(&entry).is_err());
        entry.download_id = None;
        entry.id = "custom-model".into();
        assert!(script_download_id(&entry).is_err());
        entry.id = "whisper-large-v3-turbo".into();
        entry.sha256 = None;
        assert!(script_download_id(&entry).is_err());
    }

    #[test]
    fn fake_script_receives_direct_target_and_root_and_reports_checksum_failure() {
        let root = std::env::temp_dir().join(format!("pheme-download-fake-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let script = root.join("download.sh");
        std::fs::write(&script, "printf '%s' \"$1\" > \"$PHEME_VA_MODEL_DIR/target\"\nprintf 'checksum mismatch\\n' >&2\nexit 1\n").unwrap();
        let mut task =
            DownloadTask::start_script("fixture-id".into(), "registry-target", &root, &script)
                .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            match task.poll() {
                Err(error) => {
                    assert!(error.to_string().contains("checksum mismatch"));
                    break;
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
                _ => panic!("fake checksum failure must not succeed"),
            }
            assert!(std::time::Instant::now() < deadline);
        }
        assert_eq!(
            std::fs::read_to_string(root.join("target")).unwrap(),
            "registry-target"
        );
        drop(task);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_terminates_script_group_and_allows_cleanup_for_retry() {
        let root =
            std::env::temp_dir().join(format!("pheme-download-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let script = root.join("download.sh");
        std::fs::write(&script, "trap 'rmdir \"$PHEME_VA_MODEL_DIR/lock\"; exit 143' TERM\nmkdir \"$PHEME_VA_MODEL_DIR/lock\"\nsleep 30 &\nwait\n").unwrap();
        let mut task =
            DownloadTask::start_script("fixture".into(), "target", &root, &script).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !root.join("lock").exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let reaper = task.cancel();
        while root.join("lock").exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if let Some(reaper) = reaper {
            reaper.join().unwrap();
        }
        drop(task);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_curl_percentages() {
        assert_eq!(parse_percent("################ 50.0%"), Some(50));
        assert_eq!(parse_percent("Downloaded and verified"), None);
    }
}
