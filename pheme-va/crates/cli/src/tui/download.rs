use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};

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
    pub fn start(model_id: String, model_dir: &Path) -> Result<Self> {
        let argument = script_argument(&model_id)?;
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

        let mut command = Command::new("bash");
        command
            .arg(&script)
            .arg(argument)
            .env("PHEME_VA_MODEL_DIR", model_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().context("could not start model download")?;
        let stdout = child
            .stdout
            .take()
            .context("download stdout was not captured")?;
        let stderr = child
            .stderr
            .take()
            .context("download stderr was not captured")?;
        let (sender, receiver) = mpsc::channel();
        spawn_reader(stdout, sender.clone());
        spawn_reader(stderr, sender);
        Ok(Self {
            model_id,
            child: Some(child),
            output_rx: receiver,
            progress: DownloadProgress {
                percent: Some(0),
                message: "starting download".to_owned(),
            },
            output: String::new(),
            progress_input: String::new(),
        })
    }

    pub fn poll(&mut self) -> Result<Option<DownloadOutcome>> {
        while let Ok(message) = self.output_rx.try_recv() {
            match message {
                DownloadOutput::Text(text) => self.consume_output(&text),
                DownloadOutput::IoError(error) => self.output.push_str(&format!("\n{error}")),
            }
        }
        let child = self.child.as_mut().context("download task has no child")?;
        let Some(status) = child.try_wait().context("could not poll model download")? else {
            return Ok(None);
        };
        self.child.take();
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

    pub fn cancel(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }

    fn consume_output(&mut self, text: &str) {
        self.progress_input.push_str(text);
        if self.progress_input.len() > 512 {
            let start = self.progress_input.len() - 512;
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
            let start = self.output.len() - 8_192;
            self.output.drain(..start);
        }
    }
}

impl Drop for DownloadTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn spawn_reader<R>(mut reader: R, sender: mpsc::Sender<DownloadOutput>)
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
        let value = token.trim_end_matches('%').parse::<f32>().ok()?;
        (0.0..=100.0)
            .contains(&value)
            .then_some(value.round() as u8)
    })
}

fn script_argument(model_id: &str) -> Result<&'static str> {
    match model_id {
        "whisper-large-v3-turbo" => Ok("whisper"),
        "zipformer-small" => Ok("zipformer-small"),
        "zipformer-medium" => Ok("zipformer-medium"),
        "zipformer-large" => Ok("zipformer-large"),
        _ => bail!("automatic download is not allowlisted for model `{model_id}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_model_ids_are_downloadable() {
        assert_eq!(
            script_argument("whisper-large-v3-turbo").unwrap(),
            "whisper"
        );
        assert!(script_argument("custom-model").is_err());
        assert!(script_argument("whisper;rm -rf /").is_err());
    }

    #[test]
    fn parses_curl_percentages() {
        assert_eq!(parse_percent("################ 50.0%"), Some(50));
        assert_eq!(parse_percent("Downloaded and verified"), None);
    }
}
