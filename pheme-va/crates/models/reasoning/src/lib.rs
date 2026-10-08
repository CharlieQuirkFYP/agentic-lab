//! Server-owned persistent stdio reply client. No llama.cpp/GGML symbols are
//! linked into this crate or its HTTP host; the native runtime is in a sibling
//! `pheme-reply-worker` executable, built normally by the workspace.

pub mod protocol;

use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use protocol::{Command, Event};
use va_core::{
    validate_messages, validate_response, ConversationConfig, ConversationError,
    ConversationMessage, ConversationModel,
};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);
const GENERATION_TIMEOUT: Duration = Duration::from_secs(300);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ReplyModel {
    child: Child,
    writer: Option<mpsc::SyncSender<Vec<u8>>>,
    events: Option<mpsc::Receiver<Result<Event, String>>>,
    reader_thread: Option<JoinHandle<()>>,
    writer_thread: Option<JoinHandle<()>>,
    alive: Arc<AtomicBool>,
    ready: bool,
    name: String,
    load_time_ms: u64,
    next_id: u64,
}

impl ReplyModel {
    /// A trusted environment override or the worker next to the server binary.
    /// This never invokes Cargo, a shell, a downloader, or a separate HTTP host.
    pub fn load(
        path: &Path,
        config: &ConversationConfig,
        threads: i32,
        gpu_layers: u32,
    ) -> Result<Self> {
        Self::load_with_worker(&worker_path()?, path, config, threads, gpu_layers)
    }

    pub fn load_with_worker(
        worker: &Path,
        path: &Path,
        config: &ConversationConfig,
        threads: i32,
        gpu_layers: u32,
    ) -> Result<Self> {
        Self::load_with_timeout(worker, path, config, threads, gpu_layers, STARTUP_TIMEOUT)
    }

    fn load_with_timeout(
        worker: &Path,
        path: &Path,
        config: &ConversationConfig,
        threads: i32,
        gpu_layers: u32,
        startup_timeout: Duration,
    ) -> Result<Self> {
        protocol::validate_config(config)?;
        if threads <= 0 {
            bail!("reply threads must be positive");
        }
        if !path.is_file() {
            bail!("reply model is not a regular file: {}", path.display());
        }
        if path.to_str().is_none() {
            bail!("reply model path is not UTF-8");
        }
        let worker = worker.canonicalize().with_context(|| format!(
            "reply worker {} is missing; build/ship pheme-reply-worker with the host (cargo build --workspace), or set PHEME_VA_REPLY_WORKER", worker.display()
        ))?;
        let mut command = ProcessCommand::new(&worker);
        #[cfg(test)]
        if worker
            .extension()
            .is_some_and(|extension| extension == "sh")
        {
            // Fixtures are scripts, not native executables. Invoke their known
            // interpreter directly to avoid exec/write races on temporary files.
            command = ProcessCommand::new("bash");
            command.arg(&worker);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("could not start reply worker {}", worker.display()))?;
        let stdin = child.stdin.take().expect("piped worker stdin");
        let stdout = child.stdout.take().expect("piped worker stdout");
        let alive = Arc::new(AtomicBool::new(true));
        let (events_tx, events) = mpsc::sync_channel(32);
        let (writer, writes) = mpsc::sync_channel::<Vec<u8>>(2);
        let reader_alive = Arc::clone(&alive);
        let reader_events = events_tx.clone();
        let reader_thread = thread::spawn(move || {
            let mut stdout = std::io::BufReader::new(stdout);
            loop {
                let error = match protocol::read_frame(&mut stdout) {
                    Ok(Some(event)) => {
                        if reader_events.send(Ok(event)).is_err() {
                            break;
                        }
                        continue;
                    }
                    Ok(None) => "reply worker closed stdout".to_owned(),
                    Err(error) => format!("invalid reply worker output: {error}"),
                };
                reader_alive.store(false, Ordering::Release);
                let _ = reader_events.send(Err(error));
                break;
            }
        });
        let writer_alive = Arc::clone(&alive);
        let writer_thread = thread::spawn(move || {
            use std::io::Write;
            let mut stdin = stdin;
            while let Ok(frame) = writes.recv() {
                if let Err(error) = stdin.write_all(&frame).and_then(|()| stdin.flush()) {
                    writer_alive.store(false, Ordering::Release);
                    let _ = events_tx.send(Err(format!("reply worker input failed: {error}")));
                    break;
                }
            }
        });
        let mut reply = Self {
            child,
            writer: Some(writer),
            events: Some(events),
            reader_thread: Some(reader_thread),
            writer_thread: Some(writer_thread),
            alive,
            ready: false,
            name: String::new(),
            load_time_ms: 0,
            next_id: 1,
        };
        reply.send(&Command::Init {
            protocol: protocol::VERSION,
            path: path.to_owned(),
            config: config.clone(),
            threads,
            gpu_layers,
        })?;
        match reply
            .events
            .as_ref()
            .expect("worker events")
            .recv_timeout(startup_timeout)
        {
            Ok(Ok(Event::Ready {
                protocol,
                name,
                load_time_ms,
            })) if protocol == protocol::VERSION && !name.trim().is_empty() => {
                reply.name = name;
                reply.load_time_ms = load_time_ms;
                reply.ready = true;
                Ok(reply)
            }
            Ok(Ok(Event::StartupError { message })) => {
                bail!("reply worker startup failed: {message}")
            }
            Ok(Err(error)) => bail!("{error}"),
            Ok(Ok(_)) => bail!("reply worker returned an invalid startup handshake"),
            Err(error) => bail!("reply worker startup did not complete: {error}"),
        }
    }

    pub fn load_time_ms(&self) -> u64 {
        self.load_time_ms
    }
    /// Resource accounting must include the native child, not just the server.
    pub fn worker_pid(&self) -> u32 {
        self.child.id()
    }

    fn send(&self, command: &Command) -> Result<()> {
        let frame = protocol::encode_frame(command)?;
        self.writer
            .as_ref()
            .ok_or_else(|| anyhow!("reply worker is stopped"))?
            .try_send(frame)
            .map_err(|error| anyhow!("reply worker input queue failed: {error}"))
    }

    fn stop(&mut self) {
        self.ready = false;
        self.alive.store(false, Ordering::Release);
        self.writer.take();
        // Hard stop is only for drop/protocol failure/unresponsive cancellation.
        // Ordinary cancellation drains the terminal event and keeps weights.
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.events.take(); // unblock bounded producers before joining them
        if let Some(reader) = self.reader_thread.take() {
            let _ = reader.join();
        }
        if let Some(writer) = self.writer_thread.take() {
            let _ = writer.join();
        }
    }

    fn broken(&mut self, message: impl Into<String>) -> ConversationError {
        self.stop();
        ConversationError::Backend(message.into())
    }

    fn generate(
        &mut self,
        id: u64,
        config: &ConversationConfig,
        cancelled: &AtomicBool,
        emit: &mut dyn FnMut(&str),
        generation_timeout: Duration,
        cancel_timeout: Duration,
    ) -> Result<String, ConversationError> {
        let started = Instant::now();
        let mut cancellation_sent = None;
        let mut text = String::new();
        let mut chars = 0usize;
        loop {
            if cancelled.load(Ordering::Acquire) && cancellation_sent.is_none() {
                if let Err(error) = self.send(&Command::Cancel { id }) {
                    return Err(self.broken(error.to_string()));
                }
                cancellation_sent = Some(Instant::now());
            }
            if cancellation_sent.is_some_and(|time: Instant| time.elapsed() >= cancel_timeout) {
                self.stop();
                return Err(ConversationError::Cancelled);
            }
            if cancellation_sent.is_none() && started.elapsed() >= generation_timeout {
                return Err(self.broken("reply worker exceeded generation deadline"));
            }
            let event = match self
                .events
                .as_ref()
                .expect("live worker events")
                .recv_timeout(POLL_INTERVAL)
            {
                Ok(Ok(event)) => event,
                Ok(Err(error)) => return Err(self.broken(error)),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(self.broken("reply worker output channel closed"))
                }
            };
            match event {
                Event::Delta {
                    id: event_id,
                    text: delta,
                } if event_id == id => {
                    if delta
                        .chars()
                        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
                    {
                        return Err(self.broken("reply worker emitted invalid control characters"));
                    }
                    let count = delta.chars().count();
                    if count > config.max_output_chars.saturating_sub(chars) {
                        self.stop();
                        return Err(ConversationError::OutputTooLong {
                            limit: config.max_output_chars,
                        });
                    }
                    chars += count;
                    text.push_str(&delta);
                    if cancellation_sent.is_none()
                        && !cancelled.load(Ordering::Acquire)
                        && !delta.is_empty()
                    {
                        emit(&delta);
                    }
                }
                Event::Done {
                    id: event_id,
                    text: complete,
                } if event_id == id => {
                    if complete != text {
                        return Err(
                            self.broken("reply worker final text differs from streamed deltas")
                        );
                    }
                    if cancellation_sent.is_some() || cancelled.load(Ordering::Acquire) {
                        return Err(ConversationError::Cancelled);
                    }
                    validate_response(&complete, config)?;
                    return Ok(complete);
                }
                Event::Error {
                    id: event_id,
                    error,
                } if event_id == id => {
                    if cancellation_sent.is_some() || cancelled.load(Ordering::Acquire) {
                        return Err(ConversationError::Cancelled);
                    }
                    return Err(error.into());
                }
                _ => {
                    return Err(
                        self.broken("reply worker emitted a stale request ID or unexpected event")
                    )
                }
            }
        }
    }
}

impl Drop for ReplyModel {
    fn drop(&mut self) {
        if self.is_ready() {
            let _ = self.send(&Command::Shutdown);
            self.writer.take();
            let started = Instant::now();
            while matches!(self.child.try_wait(), Ok(None))
                && started.elapsed() < Duration::from_millis(500)
            {
                thread::sleep(POLL_INTERVAL);
            }
        }
        self.stop();
    }
}

impl ConversationModel for ReplyModel {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_ready(&self) -> bool {
        self.ready && self.alive.load(Ordering::Acquire)
    }
    fn respond_stream(
        &mut self,
        messages: &[ConversationMessage],
        config: &ConversationConfig,
        cancelled: &AtomicBool,
        emit: &mut dyn FnMut(&str),
    ) -> Result<String, ConversationError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(ConversationError::Cancelled);
        }
        protocol::validate_config(config)?;
        validate_messages(messages, config)?;
        if !self.is_ready() {
            return Err(self.broken("reply worker is not ready"));
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| self.broken("reply request ID exhausted"))?;
        if let Err(error) = self.send(&Command::Generate {
            id,
            messages: messages.to_owned(),
            config: config.clone(),
        }) {
            return Err(self.broken(error.to_string()));
        }
        self.generate(
            id,
            config,
            cancelled,
            emit,
            GENERATION_TIMEOUT,
            CANCEL_TIMEOUT,
        )
    }
}

#[cfg(all(test, unix))]
mod ipc_tests;

pub fn worker_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PHEME_VA_REPLY_WORKER") {
        if path.is_empty() {
            bail!("PHEME_VA_REPLY_WORKER may not be empty");
        }
        return Ok(PathBuf::from(path));
    }
    let executable = std::env::current_exe().context("could not locate host executable")?;
    let mut directory = executable
        .parent()
        .ok_or_else(|| anyhow!("host executable has no parent directory"))?;
    // Cargo's unit-test executables live one level below the normal binaries.
    if directory.file_name().is_some_and(|name| name == "deps") {
        directory = directory
            .parent()
            .ok_or_else(|| anyhow!("test executable has no build directory"))?;
    }
    Ok(directory.join(format!(
        "pheme-reply-worker{}",
        std::env::consts::EXE_SUFFIX
    )))
}
