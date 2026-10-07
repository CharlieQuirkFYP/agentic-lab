//! Go-facing inspection and request-scoped tests. No model-management routes.
use std::io::Read;
use std::path::PathBuf;

use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use reqwest::{Client, Response, Url};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use va_core::AudioBuffer;

use super::editor::MAX_INPUT_BYTES;

pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8080";
const MAX_JSON_BYTES: usize = 262_144;
pub const MAX_REPLY_BYTES: usize = 32_768;
const MAX_WAV_BYTES: usize = 16 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Deserialize)]
pub struct RuntimeStatus {
    pub name: String,
    pub ready: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Role {
    pub name: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Transcribing,
    AwaitingReview,
    Generating,
    Completed,
    Failed,
    Cancelled,
}

impl TurnStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Transcribing => "transcribing",
            Self::AwaitingReview => "awaiting_review",
            Self::Generating => "generating",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Turn {
    pub turn_id: String,
    pub status: TurnStatus,
    #[serde(default)]
    pub transcript: String,
    pub approved_text: Option<String>,
    #[serde(default)]
    pub reply: String,
    pub error: Option<ApiError>,
    #[serde(default)]
    pub timings: Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Snapshot {
    pub history: Vec<Message>,
    pub current_turn: Option<Turn>,
    pub stt: RuntimeStatus,
    pub reply: RuntimeStatus,
    pub role: Option<Role>,
    pub busy: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Transcript {
    pub status: String,
    pub raw_text: String,
    pub text: String,
    pub model_id: String,
    pub stt_backend: String,
    pub processing_time_ms: u64,
}

pub enum AudioInput {
    Wav(PathBuf),
    Microphone(AudioBuffer),
}

pub enum Command {
    Submit {
        turn_id: String,
        text: String,
    },
    Reply {
        request: u64,
        text: String,
    },
    Transcribe {
        request: u64,
        audio: AudioInput,
        max_seconds: u32,
    },
    CancelTest,
}

#[derive(Debug)]
pub enum Event {
    Submitted {
        turn_id: String,
        text: String,
        result: Result<(), String>,
    },
    Transcript {
        request: u64,
        result: Result<Transcript, String>,
    },
    ReplyStarted {
        request: u64,
    },
    ReplyDelta {
        request: u64,
        text: String,
    },
    ReplyCompleted {
        request: u64,
        text: String,
    },
    TestFailed {
        request: u64,
        error: String,
    },
}

pub struct Connection {
    sender: mpsc::Sender<Command>,
    receiver: mpsc::Receiver<Event>,
    inspection: watch::Receiver<Option<Result<Snapshot, String>>>,
    stop: watch::Sender<bool>,
    join: Option<JoinHandle<()>>,
}

impl Connection {
    pub fn start(target: &str) -> Result<Self> {
        let base = api_base(target)?;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (sender, mut commands) = mpsc::channel(8);
        let (events, receiver) = mpsc::channel(64);
        let (inspect_sender, inspection) = watch::channel(None);
        let (stop, mut stopping) = watch::channel(false);
        let join = std::thread::spawn(move || {
            runtime.block_on(async move {
                let poll_client = client.clone();
                let inspect_url = base.join("voice/inspect").expect("validated base URL");
                let poll = tokio::spawn(async move {
                    loop {
                        let result = async {
                            let response = poll_client.get(inspect_url.clone())
                                .timeout(Duration::from_secs(3)).send().await?;
                            let snapshot: Snapshot = json_response(response).await?;
                            ensure!(snapshot.history.len() <= 128, "inspection history exceeds limit");
                            Ok(snapshot)
                        }.await.map_err(|error: anyhow::Error| error.to_string());
                        inspect_sender.send_replace(Some(result));
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                });
                let mut test: Option<tokio::task::JoinHandle<()>> = None;
                let mut submit: Option<tokio::task::JoinHandle<()>> = None;
                loop {
                    tokio::select! {
                        _ = stopping.changed() => break,
                        command = commands.recv() => {
                            let Some(command) = command else { break };
                            match command {
                                Command::CancelTest => {
                                    if let Some(task) = test.take() { task.abort(); }
                                }
                                Command::Submit { turn_id, text } => {
                                    if submit.as_ref().is_some_and(|task| !task.is_finished()) {
                                        let _ = events.try_send(Event::Submitted {
                                            turn_id, text, result: Err("a submission is already pending".into()),
                                        });
                                        continue;
                                    }
                                    let client = client.clone();
                                    let events = events.clone();
                                    let mut url = base.clone();
                                    url.path_segments_mut().expect("HTTP URL")
                                        .pop_if_empty().extend(["voice", "turns", &turn_id, "submit"]);
                                    submit = Some(tokio::spawn(async move {
                                        let result = async {
                                            validate_text(&text)?;
                                            let response = client.post(url).timeout(Duration::from_secs(10))
                                                .json(&serde_json::json!({"text": text})).send().await?;
                                            check_status(response).await?;
                                            Ok(())
                                        }.await.map_err(|error: anyhow::Error| error.to_string());
                                        let _ = events.send(Event::Submitted { turn_id, text, result }).await;
                                    }));
                                }
                                command => {
                                    if let Some(task) = test.take() { task.abort(); }
                                    let client = client.clone();
                                    let base = base.clone();
                                    let events = events.clone();
                                    test = Some(tokio::spawn(async move {
                                        match command {
                                            Command::Reply { request, text } => {
                                                let result = reply_test(&client, &base, request, &text, &events).await;
                                                if let Err(error) = result {
                                                    let _ = events.send(Event::TestFailed { request, error: error.to_string() }).await;
                                                }
                                            }
                                            Command::Transcribe { request, audio, max_seconds } => {
                                                let result = async {
                                                    let wav = prepare_wav(audio, max_seconds)?;
                                                    let response = client.post(base.join("voice/transcribe")?)
                                                        .timeout(REQUEST_TIMEOUT).header("Content-Type", "audio/wav")
                                                        .body(wav).send().await?;
                                                    json_response::<Transcript>(response).await
                                                }.await.map_err(|error| error.to_string());
                                                let _ = events.send(Event::Transcript { request, result }).await;
                                            }
                                            _ => unreachable!(),
                                        }
                                    }));
                                }
                            }
                        }
                    }
                }
                poll.abort();
                if let Some(task) = test { task.abort(); }
                if let Some(task) = submit { task.abort(); }
            });
        });
        Ok(Self {
            sender,
            receiver,
            inspection,
            stop,
            join: Some(join),
        })
    }

    pub fn send(&self, command: Command) -> Result<()> {
        self.sender
            .try_send(command)
            .context("connection command queue unavailable")
    }

    pub fn inspection(&mut self) -> Option<Result<Snapshot, String>> {
        self.inspection
            .has_changed()
            .ok()
            .filter(|changed| *changed)?;
        self.inspection.borrow_and_update().clone()
    }

    pub fn event(&mut self) -> Option<Event> {
        self.receiver.try_recv().ok()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn api_base(target: &str) -> Result<Url> {
    let mut base = Url::parse(target).context("invalid server URL")?;
    ensure!(
        matches!(base.scheme(), "http" | "https") && base.host_str().is_some(),
        "use an HTTP(S) server URL"
    );
    ensure!(
        base.username().is_empty() && base.password().is_none(),
        "URL credentials are not supported"
    );
    ensure!(
        base.query().is_none() && base.fragment().is_none(),
        "server URL cannot contain a query or fragment"
    );
    let path = base.path().trim_end_matches('/');
    let path = if path.ends_with("/api/v1") {
        format!("{path}/")
    } else {
        format!("{path}/api/v1/")
    };
    base.set_path(&path);
    Ok(base)
}

async fn check_status(response: Response) -> Result<Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let bytes = limited_body(response, 8_192).await?;
    if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
        if let Some(error) = value.get("error") {
            if let Ok(error) = serde_json::from_value::<ApiError>(error.clone()) {
                bail!("{}: {} ({status})", error.code, error.message);
            }
        }
    }
    bail!("server returned {status}")
}

async fn limited_body(mut response: Response, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= limit,
            "server response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn json_response<T: serde::de::DeserializeOwned>(response: Response) -> Result<T> {
    let response = check_status(response).await?;
    let bytes = limited_body(response, MAX_JSON_BYTES).await?;
    serde_json::from_slice(&bytes).context("invalid server JSON")
}

pub fn validate_text(text: &str) -> Result<()> {
    ensure!(!text.trim().is_empty(), "enter a nonempty question first");
    ensure!(
        text.len() <= MAX_INPUT_BYTES,
        "question exceeds {MAX_INPUT_BYTES} bytes"
    );
    Ok(())
}

async fn reply_test(
    client: &Client,
    base: &Url,
    request: u64,
    text: &str,
    events: &mpsc::Sender<Event>,
) -> Result<()> {
    validate_text(text)?;
    let response = client
        .post(base.join("voice/test/reply")?)
        .timeout(REQUEST_TIMEOUT)
        .header("Accept", "text/event-stream")
        .json(&serde_json::json!({"text": text}))
        .send()
        .await?;
    let mut response = check_status(response).await?;
    ensure!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream")),
        "expected a reply SSE stream"
    );
    let mut parser = SseParser::default();
    let mut output_bytes = 0;
    let mut frames = 0;
    let mut wire_bytes = 0;
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(30), response.chunk())
            .await
            .context("reply stream stalled for 30 seconds")??;
        let Some(chunk) = chunk else {
            bail!("reply stream ended without completion")
        };
        wire_bytes += chunk.len();
        ensure!(
            wire_bytes <= 1_048_576,
            "reply stream exceeds wire size limit"
        );
        for (kind, data) in parser.feed(&chunk)? {
            frames += 1;
            ensure!(frames <= 8_192, "reply stream exceeds event limit");
            let value: Value = serde_json::from_str(&data).context("invalid reply event JSON")?;
            let event = match kind.as_str() {
                "reply.started" => Event::ReplyStarted { request },
                "reply.delta" => {
                    let text = event_text(&value)?;
                    output_bytes += text.len();
                    ensure!(
                        output_bytes <= MAX_REPLY_BYTES,
                        "reply exceeds output limit"
                    );
                    Event::ReplyDelta { request, text }
                }
                "reply.completed" => {
                    let text = event_text(&value)?;
                    ensure!(
                        !text.trim().is_empty() && text.len() <= MAX_REPLY_BYTES,
                        "invalid completed reply"
                    );
                    events.send(Event::ReplyCompleted { request, text }).await?;
                    return Ok(());
                }
                "reply.failed" | "turn.failed" => {
                    let error: ApiError = serde_json::from_value(
                        value.get("error").cloned().context("missing reply error")?,
                    )?;
                    bail!("{}: {}", error.code, error.message);
                }
                _ => continue,
            };
            events.send(event).await?;
        }
    }
}

fn event_text(value: &Value) -> Result<String> {
    value
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .context("missing event text")
}

/// Frame bytes before decoding UTF-8: HTTP chunks can split a code point.
#[derive(Default)]
struct SseParser {
    line: Vec<u8>,
    event: String,
    data: Vec<String>,
    size: usize,
    cr: bool,
}

impl SseParser {
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<(String, String)>> {
        let mut frames = Vec::new();
        for &byte in bytes {
            if self.cr && byte == b'\n' {
                self.cr = false;
                continue;
            }
            self.cr = byte == b'\r';
            if matches!(byte, b'\n' | b'\r') {
                let line = String::from_utf8(std::mem::take(&mut self.line))
                    .context("invalid SSE UTF-8")?;
                if line.is_empty() {
                    if !self.data.is_empty() {
                        frames.push((std::mem::take(&mut self.event), self.data.join("\n")));
                        self.data.clear();
                    }
                    self.event.clear();
                    self.size = 0;
                } else if !line.starts_with(':') {
                    let (field, value) = line.split_once(':').unwrap_or((&line, ""));
                    let value = value.strip_prefix(' ').unwrap_or(value);
                    match field {
                        "event" => self.event = value.to_owned(),
                        "data" => self.data.push(value.to_owned()),
                        _ => {}
                    }
                }
            } else {
                self.size += 1;
                ensure!(
                    self.size <= MAX_REPLY_BYTES + 4_096,
                    "SSE frame exceeds limit"
                );
                self.line.push(byte);
            }
        }
        Ok(frames)
    }
}

fn prepare_wav(audio: AudioInput, max_seconds: u32) -> Result<Vec<u8>> {
    let audio = match audio {
        AudioInput::Wav(path) => {
            let file = std::fs::File::open(path).context("could not open WAV")?;
            ensure!(file.metadata()?.is_file(), "choose a regular WAV file");
            let mut bytes = Vec::new();
            file.take(MAX_WAV_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() <= MAX_WAV_BYTES,
                "WAV exceeds 16 MiB upload limit"
            );
            AudioBuffer::from_wav(&bytes)?
        }
        AudioInput::Microphone(audio) => audio,
    };
    let normalized = audio.normalize(Some(max_seconds.min(120)))?;
    let size = normalized.samples.len() * 2;
    ensure!(size + 44 <= MAX_WAV_BYTES, "audio exceeds upload limit");
    let mut bytes = Vec::with_capacity(size + 44);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(size as u32 + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&16_000_u32.to_le_bytes());
    bytes.extend_from_slice(&32_000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(size as u32).to_le_bytes());
    for sample in normalized.samples {
        bytes.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "connected_tests.rs"]
mod transport_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_accepts_every_chunk_boundary_in_unicode_and_crlf() {
        let input = "event: reply.delta\r\ndata: {\"text\":\"界🙂\"}\r\n\r\n";
        for split in 0..=input.len() {
            let mut parser = SseParser::default();
            let mut frames = parser.feed(&input.as_bytes()[..split]).unwrap();
            frames.extend(parser.feed(&input.as_bytes()[split..]).unwrap());
            assert_eq!(
                frames,
                vec![("reply.delta".into(), "{\"text\":\"界🙂\"}".into())]
            );
        }
    }

    #[test]
    fn parser_bounds_frames_and_handles_heartbeat_multiline() {
        let mut parser = SseParser::default();
        assert_eq!(
            parser
                .feed(b": heartbeat\n\nevent: reply.completed\ndata: {\ndata: \"text\":\"ok\"}\n\n")
                .unwrap(),
            vec![("reply.completed".into(), "{\n\"text\":\"ok\"}".into())]
        );
        assert!(parser.feed(&vec![b'x'; MAX_REPLY_BYTES + 4_097]).is_err());
    }

    #[test]
    fn base_url_and_microphone_wav_are_valid() {
        assert_eq!(
            api_base(DEFAULT_SERVER_URL).unwrap().as_str(),
            "http://127.0.0.1:8080/api/v1/"
        );
        assert_eq!(
            api_base("http://localhost/api/v1/").unwrap().as_str(),
            "http://localhost/api/v1/"
        );
        assert!(api_base("file:///tmp/server").is_err());
        let audio = AudioBuffer::new(16_000, 1, vec![0.25; 160]).unwrap();
        let wav = prepare_wav(AudioInput::Microphone(audio), 1).unwrap();
        let decoded = AudioBuffer::from_wav(&wav).unwrap();
        assert_eq!(decoded.samples.len(), 160);
        assert_eq!(decoded.sample_rate, 16_000);
    }
}
