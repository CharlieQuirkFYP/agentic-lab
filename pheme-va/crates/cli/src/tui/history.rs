//! Bounded development-console snapshots, not the incident/session database.
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::telemetry::RunReport;

pub const MAX_REPORTS: usize = 100;
pub const MAX_EVENTS: usize = 10_000;
const MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    reports: Vec<RunReport>,
}

pub(super) fn deserialize_events<'de, D>(
    deserializer: D,
) -> Result<Vec<metrics::MetricEvent>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // MetricValue's untagged wire decoder tries f64 before u64. Recover
    // integer JSON tokens explicitly so counters do not lose precision.
    let values = Vec::<serde_json::Value>::deserialize(deserializer)?;
    values
        .into_iter()
        .map(|value| {
            let integer = value.get("value").and_then(serde_json::Value::as_u64);
            let mut event: metrics::MetricEvent =
                serde_json::from_value(value).map_err(serde::de::Error::custom)?;
            if let Some(integer) = integer {
                event.value = Some(metrics::MetricValue::Integer(integer));
            }
            Ok(event)
        })
        .collect()
}

fn bound(reports: &mut Vec<RunReport>) {
    reports.truncate(MAX_REPORTS);
    for report in reports {
        let excess = report.events.len().saturating_sub(MAX_EVENTS);
        report.events.drain(..excess);
    }
}

fn load(path: &Path) -> Result<Vec<RunReport>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("history exceeds the 64 MiB limit");
    }
    let mut snapshot: Snapshot = serde_json::from_slice(&bytes)?;
    if snapshot.version != 1 {
        bail!("unsupported history version {}", snapshot.version);
    }
    let mut ids = std::collections::HashSet::new();
    if snapshot
        .reports
        .iter()
        .any(|report| !ids.insert(&report.run_id))
    {
        bail!("duplicate run IDs in history");
    }
    bound(&mut snapshot.reports);
    Ok(snapshot.reports)
}

fn save(path: &Path, mut reports: Vec<RunReport>) -> Result<()> {
    bound(&mut reports);
    let bytes = serde_json::to_vec(&Snapshot {
        version: 1,
        reports,
    })?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("history exceeds the 64 MiB limit; previous snapshot retained");
    }
    let parent = path.parent().context("history path has no parent")?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(parent)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let result = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn clear(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {
            #[cfg(unix)]
            fs::File::open(path.parent().context("history path has no parent")?)?.sync_all()?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

enum Command {
    Save(Vec<RunReport>),
    Clear,
    Flush(mpsc::Sender<Result<(), String>>),
}

pub struct History {
    sender: Option<SyncSender<Command>>,
    pending: Option<Command>,
    outcomes: Receiver<(bool, Result<(), String>)>,
    join: Option<JoinHandle<()>>,
}

impl History {
    pub fn open(path: PathBuf) -> Result<(Self, Vec<RunReport>)> {
        let reports = load(&path).with_context(|| {
            format!(
                "could not load history {}; file left unchanged",
                path.display()
            )
        })?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let (outcomes_sender, outcomes) = mpsc::channel();
        let join = std::thread::Builder::new()
            .name("tui-history".into())
            .spawn(move || {
                let mut last_result = Ok(());
                while let Ok(command) = receiver.recv() {
                    let (clear, result) = match command {
                        Command::Save(reports) => (false, save(&path, reports)),
                        Command::Clear => (true, clear(&path)),
                        Command::Flush(reply) => {
                            let _ = reply.send(last_result.clone());
                            continue;
                        }
                    };
                    last_result =
                        result.map_err(|error| format!("history {}: {error:#}", path.display()));
                    let _ = outcomes_sender.send((clear, last_result.clone()));
                }
            })?;
        Ok((
            Self {
                sender: Some(sender),
                pending: None,
                outcomes,
                join: Some(join),
            },
            reports,
        ))
    }

    pub fn save(&mut self, reports: Vec<RunReport>) {
        self.pending = Some(Command::Save(reports));
        self.pump();
    }

    pub fn clear(&mut self) {
        self.pending = Some(Command::Clear);
        self.pump();
    }

    pub fn pump(&mut self) {
        if let Some(command) = self.pending.take() {
            match self.sender.as_ref().unwrap().try_send(command) {
                Ok(()) => {}
                Err(TrySendError::Full(command) | TrySendError::Disconnected(command)) => {
                    self.pending = Some(command)
                }
            }
        }
    }

    pub fn outcomes(&self) -> Vec<(bool, Result<(), String>)> {
        self.outcomes.try_iter().collect()
    }

    pub fn flush(&mut self) -> Result<()> {
        let sender = self.sender.as_ref().unwrap();
        if let Some(command) = self.pending.take() {
            sender
                .send(command)
                .map_err(|_| anyhow::anyhow!("history writer stopped"))?;
        }
        let (reply, result) = mpsc::channel();
        sender
            .send(Command::Flush(reply))
            .map_err(|_| anyhow::anyhow!("history writer stopped"))?;
        result
            .recv()
            .context("history writer stopped")?
            .map_err(anyhow::Error::msg)
    }
}

impl Drop for History {
    fn drop(&mut self) {
        let _ = self.flush();
        self.sender.take();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::tui::app::tests::{metric, report};
    use std::sync::atomic::{AtomicU64, Ordering};

    pub struct TestPath(pub PathBuf);
    impl TestPath {
        pub fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "pheme-history-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        pub fn file(&self) -> PathBuf {
            self.0.join("history.json")
        }
    }
    impl Drop for TestPath {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn roundtrip_restart_preserves_every_display_field_and_raw_event() {
        let path = TestPath::new();
        let (mut history, reports) = History::open(path.file()).unwrap();
        assert!(reports.is_empty());
        let mut completed = report("run-0001", vec![metric("run-0001", "cpu")]);
        completed.events[0].unavailable_reason = Some("sensor offline".into());
        completed.events[0].value = None;
        let mut counter = metric("run-0001", "counter");
        counter.value = Some(metrics::MetricValue::Integer(u64::MAX));
        completed.events.push(counter);
        let expected_events = completed.events.clone();
        let mut failed = report("run-0002", vec![]);
        failed.status = "ERROR".into();
        failed.error = Some("inference failed".into());
        let expected = serde_json::to_value(vec![failed.clone(), completed.clone()]).unwrap();
        history.save(vec![failed, completed]);
        history.flush().unwrap();
        drop(history);
        let (mut history, restored) = History::open(path.file()).unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(), expected);
        assert!(restored.iter().all(|report| report.result.is_none()));
        assert_eq!(restored[1].events, expected_events);
        history.clear();
        history.flush().unwrap();
        drop(history);
        assert!(History::open(path.file()).unwrap().1.is_empty());
        assert!(!path.file().exists());
    }

    #[test]
    fn malformed_and_unknown_versions_are_not_overwritten() {
        let path = TestPath::new();
        for contents in ["broken JSON", r#"{"version":2,"reports":[]}"#] {
            fs::write(path.file(), contents).unwrap();
            assert!(History::open(path.file()).is_err());
            assert_eq!(fs::read_to_string(path.file()).unwrap(), contents);
        }
    }

    #[test]
    fn bounded_reports_events_and_restrictive_atomic_snapshot() {
        let path = TestPath::new();
        let mut reports: Vec<_> = (0..MAX_REPORTS + 2)
            .map(|i| report(&format!("run-{i}"), vec![]))
            .collect();
        reports[0].events = (0..MAX_EVENTS + 3)
            .map(|i| {
                let mut event = metric("run-0", "cpu");
                event.sequence = i as u64;
                event
            })
            .collect();
        save(&path.file(), reports).unwrap();
        let restored = load(&path.file()).unwrap();
        assert_eq!(restored.len(), MAX_REPORTS);
        assert_eq!(restored[0].events.len(), MAX_EVENTS);
        assert_eq!(restored[0].events[0].sequence, 3);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path.file()).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(fs::read_dir(&path.0).unwrap().count(), 1);
    }

    #[test]
    fn failed_write_retains_snapshot_and_failed_clear_is_reported() {
        let path = TestPath::new();
        let (mut history, _) = History::open(path.file()).unwrap();
        history.save(vec![report("old", vec![])]);
        history.flush().unwrap();
        let original = fs::read(path.file()).unwrap();
        let temporary = path
            .file()
            .with_extension(format!("{}.tmp", std::process::id()));
        fs::create_dir(&temporary).unwrap();
        history.save(vec![report("new", vec![])]);
        assert!(history.flush().is_err());
        assert_eq!(fs::read(path.file()).unwrap(), original);
        fs::remove_file(path.file()).unwrap();
        fs::create_dir(path.file()).unwrap();
        history.clear();
        assert!(history.flush().is_err());
        assert!(history
            .outcomes()
            .iter()
            .any(|(clear, result)| *clear && result.is_err()));
    }
}
