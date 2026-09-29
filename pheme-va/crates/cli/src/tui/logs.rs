use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub timestamp_ms: u64,
    pub level: LogLevel,
    pub component: String,
    pub message: String,
    pub run_id: Option<String>,
    pub model_id: Option<String>,
}

impl LogEntry {
    pub fn new(
        level: LogLevel,
        component: impl Into<String>,
        message: impl Into<String>,
        run_id: Option<String>,
        model_id: Option<String>,
    ) -> Self {
        Self {
            timestamp_ms: epoch_millis(),
            level,
            component: component.into(),
            message: message.into(),
            run_id,
            model_id,
        }
    }

    pub fn info(component: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(LogLevel::Info, component, message, None, None)
    }

    pub fn warn(component: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(LogLevel::Warn, component, message, None, None)
    }

    pub fn error(component: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(LogLevel::Error, component, message, None, None)
    }
}

#[derive(Debug)]
pub struct LogStore {
    entries: VecDeque<LogEntry>,
    capacity: usize,
}

impl LogStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity.min(256)),
            capacity: capacity.max(1),
        }
    }

    pub fn push(&mut self, entry: LogEntry) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn info(&mut self, component: impl Into<String>, message: impl Into<String>) {
        self.push(LogEntry::info(component, message));
    }

    pub fn warn(&mut self, component: impl Into<String>, message: impl Into<String>) {
        self.push(LogEntry::warn(component, message));
    }

    pub fn error(&mut self, component: impl Into<String>, message: impl Into<String>) {
        self.push(LogEntry::error(component, message));
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn filtered(&self, query: &str) -> Vec<&LogEntry> {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return self.entries.iter().collect();
        }
        self.entries
            .iter()
            .filter(|entry| {
                entry.level.as_str().to_ascii_lowercase().contains(&query)
                    || entry.component.to_ascii_lowercase().contains(&query)
                    || entry.message.to_ascii_lowercase().contains(&query)
                    || entry
                        .run_id
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains(&query)
                    || entry
                        .model_id
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains(&query)
            })
            .collect()
    }
}

impl Default for LogStore {
    fn default() -> Self {
        Self::new(2_000)
    }
}

pub fn format_timestamp(timestamp_ms: u64) -> String {
    let seconds = timestamp_ms / 1_000 % 86_400;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        seconds / 3_600,
        seconds / 60 % 60,
        seconds % 60,
        timestamp_ms % 1_000
    )
}

fn epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_old_entries_and_filters_text() {
        let mut store = LogStore::new(2);
        store.push(LogEntry::info("worker", "first"));
        store.push(LogEntry::warn("metrics", "second"));
        store.push(LogEntry::error("worker", "third"));

        assert_eq!(store.len(), 2);
        assert_eq!(store.filtered("metrics").len(), 1);
        assert_eq!(store.filtered("first").len(), 0);
        assert_eq!(store.filtered("third").len(), 1);
    }
}
