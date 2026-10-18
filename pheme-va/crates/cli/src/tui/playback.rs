use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{ensure, Context, Result};

use super::connected::MAX_REPLY_BYTES;

pub struct Playback {
    child: Option<Child>,
    started: Instant,
}

impl Playback {
    pub fn start(text: &str) -> Result<Self> {
        Self::start_with_fallback(Path::new("espeak-ng"), Path::new("espeak"), text)
    }

    fn start_with_fallback(primary: &Path, fallback: &Path, text: &str) -> Result<Self> {
        match Self::start_program(primary, text) {
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                Self::start_program(fallback, text)
            }
            result => result,
        }
    }

    pub(super) fn start_program(program: &Path, text: &str) -> Result<Self> {
        ensure!(
            !text.trim().is_empty() && text.len() <= MAX_REPLY_BYTES,
            "no completed answer to speak"
        );
        let mut command = Command::new(program);
        command
            .arg("--stdin")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .context("local speech unavailable: install espeak-ng or espeak")?;
        let mut stdin = child.stdin.take().context("espeak stdin unavailable")?;
        let text = text.to_owned();
        std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        });
        Ok(Self {
            child: Some(child),
            started: Instant::now(),
        })
    }

    pub fn poll(&mut self) -> Result<bool> {
        let Some(child) = self.child.as_mut() else {
            return Ok(true);
        };
        if let Some(status) = child.try_wait()? {
            self.child.take();
            ensure!(
                status.success(),
                "espeak failed ({status}); reply text is retained"
            );
            return Ok(true);
        }
        ensure!(
            self.started.elapsed() < Duration::from_secs(120),
            "local speech timed out after 120 seconds"
        );
        Ok(false)
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            // SAFETY: an unreaped child leads its own process group.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn fake_synthesizer_uses_literal_stdin_and_failure_keeps_the_answer() {
        let root = std::env::temp_dir().join(format!("pheme-tts-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let program = root.join("fake-espeak");
        let captured = root.join("captured");
        std::fs::write(
            &program,
            format!("#!/bin/sh\ncat > '{}'\nexit 7\n", captured.display()),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let answer = "Unicode 界🙂 and $(do-not-execute)";
        let mut task =
            Playback::start_with_fallback(&root.join("missing-ng"), &program, answer).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match task.poll() {
                Err(error) => {
                    assert!(error.to_string().contains("reply text is retained"));
                    break;
                }
                Ok(false) => std::thread::sleep(Duration::from_millis(10)),
                Ok(true) => panic!("fake must fail"),
            }
            assert!(Instant::now() < deadline);
        }
        assert_eq!(std::fs::read_to_string(captured).unwrap(), answer);
        assert!(Playback::start_program(&root.join("missing"), answer).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn timeout_and_stop_are_local_and_bounded() {
        let root = std::env::temp_dir().join(format!("pheme-tts-stop-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let program = root.join("fake-espeak");
        std::fs::write(&program, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut task = Playback::start_program(&program, "answer").unwrap();
        task.started = Instant::now() - Duration::from_secs(121);
        assert!(task.poll().unwrap_err().to_string().contains("timed out"));
        let time = Instant::now();
        drop(task);
        assert!(time.elapsed() < Duration::from_secs(1));
        std::fs::remove_dir_all(root).unwrap();
    }
}
