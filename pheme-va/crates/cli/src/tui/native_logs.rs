//! TUI-only capture of process stderr (Whisper/GGML, ALSA, and direct writes).
//!
//! Start before spawning native workers; keep alive across recording/retries and
//! until those workers have joined. Drain on each UI tick. Leave the alternate
//! screen before `finish` (or dropping the guard), so restored stderr and panic
//! diagnostics cannot overwrite it. Stdout remains ratatui's output channel.
//!
//! Do not install whisper-rs's one-shot logging hooks here: without a logging
//! backend they discard diagnostics, and they cannot restore a previous callback.
//! Descriptor redirection leaves ordinary CLI logging and native callbacks alone.

use std::io;

use super::logs::LogStore;

#[cfg(unix)]
pub use unix::NativeLogs;

#[cfg(not(unix))]
pub struct NativeLogs {
    warned: std::cell::Cell<bool>,
}

#[cfg(not(unix))]
impl NativeLogs {
    pub fn start() -> io::Result<Self> {
        Ok(Self {
            warned: std::cell::Cell::new(false),
        })
    }

    pub fn drain_into(&self, logs: &mut LogStore) {
        if !self.warned.replace(true) {
            logs.warn("native-stderr", "native stderr capture is not implemented on this platform; backend diagnostics may disturb the terminal");
        }
    }

    pub fn finish(&mut self, _logs: &mut LogStore) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
mod unix {
    use std::io::{Read, Write};
    use std::os::fd::{AsFd, AsRawFd, OwnedFd};
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::mpsc::{self, Receiver, SyncSender};
    use std::sync::Arc;
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use super::super::logs::LogEntry;
    use super::{io, LogStore};

    const QUEUE_CAPACITY: usize = 256;
    const MAX_LINE_BYTES: usize = 4096;
    const READ_BYTES: usize = 8192;
    // Bound final draining even if another library retained a duplicate stderr
    // descriptor and continues writing after restoration. Never wait for EOF.
    const FINAL_READS: usize = 128;
    static ACTIVE: AtomicBool = AtomicBool::new(false);

    // The sole OS operation not provided by std. Both arguments are live owned
    // descriptors (or process stderr); dup2 atomically replaces the destination.
    extern "C" {
        fn dup2(oldfd: std::ffi::c_int, newfd: std::ffi::c_int) -> std::ffi::c_int;
    }

    fn replace_fd(source: &impl AsRawFd, destination: i32) -> io::Result<()> {
        loop {
            // SAFETY: dup2 takes integer descriptors, not pointers. Source is
            // borrowed for this call and destination is intentionally replaced.
            if unsafe { dup2(source.as_raw_fd(), destination) } >= 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    /// Owns a process-wide stderr redirect. Only one guard may be active.
    ///
    /// This is not a per-thread capture. Other stderr writers are captured too.
    /// Native producers must be stopped/joined before explicit cleanup. Drop is
    /// a best-effort fallback; `finish` reports restoration/drain-thread errors.
    pub struct NativeLogs {
        saved: Option<OwnedFd>,
        receiver: Receiver<LogEntry>,
        dropped: Arc<AtomicU64>,
        stop: Arc<AtomicBool>,
        reader: Option<JoinHandle<()>>,
    }

    impl NativeLogs {
        pub fn start() -> io::Result<Self> {
            if ACTIVE
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "native stderr capture is already active",
                ));
            }
            let result = Self::install();
            if result.is_err() {
                ACTIVE.store(false, Ordering::Release);
            }
            result
        }

        fn install() -> io::Result<Self> {
            io::stderr().flush()?;
            let saved = io::stderr().as_fd().try_clone_to_owned()?;
            let (input, output) = UnixStream::pair()?;
            input.set_nonblocking(true)?;
            let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
            let dropped = Arc::new(AtomicU64::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let reader_stop = Arc::clone(&stop);
            let reader_dropped = Arc::clone(&dropped);
            // Spawn before redirecting, so thread creation failure never leaves
            // stderr pointing at an undrained socket.
            let reader = thread::Builder::new()
                .name("native-stderr".into())
                .spawn(move || read_native(input, sender, reader_dropped, reader_stop))?;
            if let Err(error) = replace_fd(&output, 2) {
                stop.store(true, Ordering::Release);
                let _ = reader.join();
                return Err(error);
            }
            Ok(Self {
                saved: Some(saved),
                receiver,
                dropped,
                stop,
                reader: Some(reader),
            })
        }

        /// Nonblocking and bounded even while native code produces logs faster
        /// than the UI can consume them. LogStore owns its own eviction policy.
        pub fn drain_into(&self, logs: &mut LogStore) {
            for _ in 0..QUEUE_CAPACITY {
                match self.receiver.try_recv() {
                    Ok(entry) => logs.push(entry),
                    Err(_) => break,
                }
            }
            let dropped = self.dropped.swap(0, Ordering::Relaxed);
            if dropped != 0 {
                logs.warn(
                    "native-stderr",
                    format!("dropped {dropped} native log lines (capture queue full)"),
                );
            }
        }

        /// Restore stderr, stop/join the reader without waiting for EOF, and
        /// deliver queued lines including the final unterminated fragment.
        /// Call after native workers join and the terminal is restored.
        pub fn finish(&mut self, logs: &mut LogStore) -> io::Result<()> {
            let result = self.restore();
            self.drain_into(logs);
            result
        }

        fn restore(&mut self) -> io::Result<()> {
            let Some(saved) = self.saved.as_ref() else {
                return Ok(());
            };
            io::stderr().flush()?;
            // Keep the reader alive if restoration fails: closing it while
            // stderr still targets the socket could cause SIGPIPE in C code.
            replace_fd(saved, 2)?;
            self.saved.take();
            self.stop.store(true, Ordering::Release);
            let joined = self.reader.take().map(JoinHandle::join);
            ACTIVE.store(false, Ordering::Release);
            if matches!(joined, Some(Err(_))) {
                return Err(io::Error::other("native stderr drain thread panicked"));
            }
            Ok(())
        }
    }

    impl Drop for NativeLogs {
        fn drop(&mut self) {
            // Never print cleanup errors while the terminal may still be raw.
            let _ = self.restore();
        }
    }

    struct Lines {
        pending: Vec<u8>,
        truncated: bool,
        sender: SyncSender<LogEntry>,
        dropped: Arc<AtomicU64>,
    }

    impl Lines {
        fn new(sender: SyncSender<LogEntry>, dropped: Arc<AtomicU64>) -> Self {
            Self {
                pending: Vec::with_capacity(MAX_LINE_BYTES),
                truncated: false,
                sender,
                dropped,
            }
        }

        fn feed(&mut self, bytes: &[u8]) {
            for &byte in bytes {
                if byte == b'\n' || byte == b'\r' {
                    self.emit();
                } else if self.pending.len() < MAX_LINE_BYTES {
                    self.pending.push(byte);
                } else {
                    self.truncated = true;
                }
            }
        }

        fn emit(&mut self) {
            if self.pending.is_empty() && !self.truncated {
                return;
            }
            // Lossy UTF-8 handles arbitrary C diagnostics. Never pass escape or
            // control bytes from a backend through to the terminal renderer.
            let mut message: String = String::from_utf8_lossy(&self.pending)
                .chars()
                .filter(|ch| !ch.is_control())
                .collect();
            if self.truncated {
                message.push_str(" [truncated]");
            }
            self.pending.clear();
            self.truncated = false;
            if !message.trim().is_empty() {
                // Stderr has no reliable severity metadata; don't label every
                // Whisper startup message as an error or guess from its text.
                self.send(LogEntry::info("native-stderr", message));
            }
        }

        fn send(&self, entry: LogEntry) {
            if self.sender.try_send(entry).is_err() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn read_native(
        mut input: UnixStream,
        sender: SyncSender<LogEntry>,
        dropped: Arc<AtomicU64>,
        stop: Arc<AtomicBool>,
    ) {
        let mut lines = Lines::new(sender, dropped);
        let mut buffer = [0; READ_BYTES];
        let mut final_reads = 0;
        loop {
            let stopping = stop.load(Ordering::Acquire);
            if stopping {
                final_reads += 1;
                if final_reads > FINAL_READS {
                    lines.send(LogEntry::warn(
                        "native-stderr",
                        "final native log drain limit reached",
                    ));
                    break;
                }
            }
            match input.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => lines.feed(&buffer[..count]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if stopping {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    lines.send(LogEntry::error(
                        "native-stderr",
                        format!("native log drain failed: {error}"),
                    ));
                    // Keep the read descriptor alive until restoration so C
                    // writers cannot receive SIGPIPE after a reader error.
                    while !stop.load(Ordering::Acquire) {
                        thread::sleep(Duration::from_millis(10));
                    }
                    break;
                }
            }
        }
        lines.emit();
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::process::{Command, Stdio};
        use std::time::Instant;

        #[test]
        fn fragments_utf8_controls_and_final_line() {
            let (sender, receiver) = mpsc::sync_channel(10);
            let mut lines = Lines::new(sender, Arc::new(AtomicU64::new(0)));
            lines.feed(b"whisper_init_");
            lines.feed(b"state: \xc3");
            lines.feed(b"\xa9\r\n\x1b\x00native\xff\nlast");
            lines.emit();
            let messages: Vec<_> = receiver.try_iter().map(|entry| entry.message).collect();
            assert_eq!(messages, ["whisper_init_state: é", "native�", "last"]);
        }

        #[test]
        fn bounds_partial_lines_and_queue_with_loss_accounting() {
            let (sender, receiver) = mpsc::sync_channel(1);
            let dropped = Arc::new(AtomicU64::new(0));
            let mut lines = Lines::new(sender, Arc::clone(&dropped));
            lines.feed(&vec![b'x'; MAX_LINE_BYTES * 100]);
            assert_eq!(lines.pending.len(), MAX_LINE_BYTES);
            lines.feed(b"\nsecond\nthird\n");
            let entry = receiver.try_recv().unwrap();
            assert_eq!(entry.message.len(), MAX_LINE_BYTES + " [truncated]".len());
            assert_eq!(dropped.load(Ordering::Relaxed), 2);
        }

        // Descriptor mutation must not affect other tests or libtest's capture.
        #[test]
        fn scoped_capture_in_subprocess() {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tui::native_logs::unix::tests::capture_child",
                    "--nocapture",
                ])
                .env("PHEME_NATIVE_LOG_CAPTURE_CHILD", "1")
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if child.try_wait().unwrap().is_some() {
                    break;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("native capture shutdown or flood blocked");
                }
                thread::sleep(Duration::from_millis(10));
            }
            let output = child.wait_with_output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "child failed: {stderr}");
            assert_eq!(stderr, "before capture\nafter capture\nafter drop\n");
        }

        #[test]
        fn capture_child() {
            if std::env::var_os("PHEME_NATIVE_LOG_CAPTURE_CHILD").is_none() {
                return;
            }
            io::stderr().write_all(b"before capture\n").unwrap();
            let mut capture = NativeLogs::start().unwrap();
            assert!(
                matches!(NativeLogs::start(), Err(error) if error.kind() == io::ErrorKind::AlreadyExists)
            );
            // Hold an extra writer through finish: EOF-based shutdown would hang.
            let retained_stderr = io::stderr().as_fd().try_clone_to_owned().unwrap();
            let mut logs = LogStore::default();
            io::stderr()
                .write_all(b"whisper_backend_init_gpu: first\n")
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while logs.filtered("first").is_empty() {
                capture.drain_into(&mut logs);
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(10));
            }
            // Exceed socket and queue capacity without draining the UI. Native
            // writes must still make progress; overload is dropped, not queued.
            for _ in 0..65536 {
                io::stderr()
                    .write_all(b"native flood diagnostic\n")
                    .unwrap();
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while capture.dropped.load(Ordering::Relaxed) == 0 {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(10));
            }
            capture.drain_into(&mut logs);
            assert!(!logs.filtered("capture queue full").is_empty());
            capture.finish(&mut logs).unwrap();
            capture.finish(&mut logs).unwrap();
            drop(retained_stderr);
            io::stderr().write_all(b"after capture\n").unwrap();

            // Re-entering and retry-style state diagnostics remain captured.
            let mut capture = NativeLogs::start().unwrap();
            io::stderr()
                .write_all(b"whisper_init_state: retry\nunterminated")
                .unwrap();
            capture.finish(&mut logs).unwrap();
            assert_eq!(logs.filtered("whisper_init_state: retry").len(), 1);
            assert_eq!(logs.filtered("unterminated").len(), 1);
            let capture = NativeLogs::start().unwrap();
            io::stderr().write_all(b"discarded on drop\n").unwrap();
            drop(capture);
            io::stderr().write_all(b"after drop\n").unwrap();
        }
    }
}
