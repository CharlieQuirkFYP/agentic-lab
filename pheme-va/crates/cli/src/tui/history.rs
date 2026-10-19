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
    #[serde(default)]
    conversations: Vec<va_core::chat::ChatSnapshot>,
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

fn load_snapshot(path: &Path) -> Result<Snapshot> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Snapshot {
                version: 2,
                reports: Vec::new(),
                conversations: Vec::new(),
            })
        }
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("history exceeds the 64 MiB limit");
    }
    let mut snapshot: Snapshot = serde_json::from_slice(&bytes)?;
    if !matches!(snapshot.version, 1 | 2) {
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
    let mut conversation_ids = std::collections::HashSet::new();
    if snapshot
        .conversations
        .iter()
        .any(|s| !conversation_ids.insert(s.conversation_id.clone()))
    {
        bail!("duplicate conversation IDs in history");
    }
    let mut run_ids = std::collections::HashSet::new();
    if snapshot
        .conversations
        .iter()
        .flat_map(|s| &s.runs)
        .any(|r| !run_ids.insert(r.run_id.clone()))
    {
        bail!("duplicate stage run IDs in history");
    }
    if snapshot
        .conversations
        .iter()
        .any(|s| s.turns.len() > va_core::chat::MAX_CHAT_TURNS)
    {
        bail!("conversation exceeds the 32-turn limit");
    }
    if snapshot.reports.len()
        + snapshot
            .conversations
            .iter()
            .map(|s| s.runs.len().max(1))
            .sum::<usize>()
        > MAX_REPORTS
    {
        bail!("history exceeds the 100-run limit");
    }
    for run in snapshot.conversations.iter_mut().flat_map(|s| &mut s.runs) {
        let excess = run.events.len().saturating_sub(MAX_EVENTS);
        run.events.drain(..excess);
        run.dropped_events += excess as u64;
    }
    Ok(snapshot)
}

#[cfg(test)]
fn load(path: &Path) -> Result<Vec<RunReport>> {
    Ok(load_snapshot(path)?.reports)
}

fn save_snapshot(
    path: &Path,
    mut reports: Vec<RunReport>,
    mut conversations: Vec<va_core::chat::ChatSnapshot>,
) -> Result<()> {
    bound(&mut reports);
    conversations.retain(|s| s.conversation_id != "legacy-standalone");
    loop {
        let total = reports.len()
            + conversations
                .iter()
                .map(|s| s.runs.len().max(1))
                .sum::<usize>();
        if total <= MAX_REPORTS {
            break;
        }
        if let Some(index) = conversations
            .iter()
            .enumerate()
            .filter(|(_, s)| s.status != "active" && s.status != "finishing")
            .min_by_key(|(_, s)| s.started_at_ms)
            .map(|(i, _)| i)
        {
            conversations.remove(index);
        } else if !reports.is_empty() {
            reports.pop();
        } else {
            bail!(
                "active conversation exceeds the 100-run archive limit; previous snapshot retained"
            );
        }
    }
    let mut snapshot = Snapshot {
        version: 2,
        reports,
        conversations,
    };
    let bytes = loop {
        let bytes = serde_json::to_vec(&snapshot)?;
        if bytes.len() as u64 <= MAX_BYTES {
            break bytes;
        }
        if let Some(index) = snapshot
            .conversations
            .iter()
            .enumerate()
            .filter(|(_, s)| s.status != "active" && s.status != "finishing")
            .min_by_key(|(_, s)| s.started_at_ms)
            .map(|(i, _)| i)
        {
            snapshot.conversations.remove(index);
        } else {
            bail!("history exceeds the 64 MiB limit; previous snapshot retained");
        }
    };
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

#[cfg(test)]
fn save(path: &Path, reports: Vec<RunReport>) -> Result<()> {
    save_snapshot(path, reports, Vec::new())
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
    Save(Vec<RunReport>, Vec<va_core::chat::ChatSnapshot>),
    Clear,
    Flush(mpsc::Sender<Result<(), String>>),
}

pub struct History {
    pub conversations: Vec<va_core::chat::ChatSnapshot>,
    sender: Option<SyncSender<Command>>,
    pending: Option<Command>,
    outcomes: Receiver<(bool, Result<(), String>)>,
    join: Option<JoinHandle<()>>,
}

impl History {
    pub fn open(path: PathBuf) -> Result<(Self, Vec<RunReport>)> {
        let snapshot = load_snapshot(&path).with_context(|| {
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
                        Command::Save(reports, conversations) => {
                            (false, save_snapshot(&path, reports, conversations))
                        }
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
                conversations: snapshot.conversations,
                sender: Some(sender),
                pending: None,
                outcomes,
                join: Some(join),
            },
            snapshot.reports,
        ))
    }

    #[cfg(test)]
    pub fn save(&mut self, reports: Vec<RunReport>) {
        self.save_conversations(reports, Vec::new());
    }

    pub fn save_conversations(
        &mut self,
        reports: Vec<RunReport>,
        conversations: Vec<va_core::chat::ChatSnapshot>,
    ) {
        self.pending = Some(Command::Save(reports, conversations));
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
        for contents in ["broken JSON", r#"{"version":3,"reports":[]}"#] {
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
    #[test]
    fn v1_migrates_without_fake_turns_and_v2_prunes_whole_closed_conversations() {
        let path = TestPath::new();
        fs::write(
            path.file(),
            serde_json::to_vec(
                &serde_json::json!({"version":1,"reports":[report("legacy", vec![])]}),
            )
            .unwrap(),
        )
        .unwrap();
        let original = load_snapshot(&path.file()).unwrap();
        assert!(original.conversations.is_empty());
        let mut conversations = Vec::new();
        for index in 0..102 {
            let mut snapshot =
                crate::tui::app::chat::legacy_snapshot(&[report(&format!("run-{index}"), vec![])])
                    .unwrap();
            snapshot.conversation_id = format!("conv-{index}");
            snapshot.started_at_ms = index;
            snapshot.status = if index == 0 { "active" } else { "finished" }.into();
            conversations.push(snapshot);
        }
        save_snapshot(&path.file(), vec![], conversations).unwrap();
        let restored = load_snapshot(&path.file()).unwrap();
        assert_eq!(restored.version, 2);
        assert_eq!(restored.conversations.len(), 100);
        assert!(restored
            .conversations
            .iter()
            .any(|s| s.conversation_id == "conv-0"));
        assert!(!restored
            .conversations
            .iter()
            .any(|s| s.conversation_id == "conv-1" || s.conversation_id == "conv-2"));
        assert!(restored.conversations.iter().all(|s| s.turns.is_empty()));
    }
}
