use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{MetricBatch, MetricEvent, MetricsSubscriber, METRICS_SCHEMA_VERSION};

/// A thread-safe subscriber that collects events into run-scoped batches for a
/// host/API exporter.
pub struct MetricsBatcher {
    events: Mutex<VecDeque<MetricEvent>>,
    capacity: Option<usize>,
    dropped: AtomicU64,
    next_batch_sequence: AtomicU64,
}

impl MetricsBatcher {
    pub fn new() -> Self {
        Self {
            events: Mutex::new(VecDeque::new()),
            capacity: None,
            dropped: AtomicU64::new(0),
            next_batch_sequence: AtomicU64::new(0),
        }
    }

    /// Bound retention for consumers such as FFI. Existing new() remains unbounded.
    /// At capacity the oldest retained event is dropped; event sequence gaps remain visible.
    pub fn with_capacity(max_events: usize) -> Self {
        Self {
            capacity: Some(max_events),
            ..Self::new()
        }
    }

    pub fn dropped_events(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn pending_events(&self) -> usize {
        self.events.lock().map(|events| events.len()).unwrap_or(0)
    }

    /// Drain all pending events, grouped by experiment and run.
    pub fn drain(&self) -> Vec<MetricBatch> {
        let events = self
            .events
            .lock()
            .map(|mut events| std::mem::take(&mut *events))
            .unwrap_or_default();
        if events.is_empty() {
            return Vec::new();
        }

        let mut grouped: BTreeMap<(Option<String>, String), Vec<MetricEvent>> = BTreeMap::new();
        for event in events {
            grouped
                .entry((event.experiment_id.clone(), event.run_id.clone()))
                .or_default()
                .push(event);
        }

        grouped
            .into_iter()
            .map(|((experiment_id, run_id), mut events)| {
                events.sort_by_key(|event| event.sequence);
                MetricBatch {
                    schema_version: METRICS_SCHEMA_VERSION,
                    experiment_id,
                    run_id,
                    sequence_number: self.next_batch_sequence.fetch_add(1, Ordering::Relaxed),
                    timestamp_ms: epoch_millis(),
                    events,
                }
            })
            .collect()
    }
}

impl Default for MetricsBatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsSubscriber for MetricsBatcher {
    fn on_event(&self, event: &MetricEvent) {
        if let Ok(mut events) = self.events.lock() {
            if let Some(capacity) = self.capacity {
                if events.len() >= capacity {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                    if capacity == 0 {
                        return;
                    }
                    events.pop_front();
                }
            }
            events.push_back(event.clone());
        }
    }
}

fn epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{MetricScope, MetricUnit, MetricValue};

    fn event(run_id: &str, sequence: u64) -> MetricEvent {
        MetricEvent {
            schema_version: METRICS_SCHEMA_VERSION,
            experiment_id: Some("exp-1".to_owned()),
            run_id: run_id.to_owned(),
            sequence,
            timestamp_ms: sequence,
            name: format!("metric-{sequence}"),
            value: Some(MetricValue::Integer(sequence)),
            unit: MetricUnit::Count,
            scope: MetricScope::Run,
            source: "test".to_owned(),
            unavailable_reason: None,
        }
    }

    #[test]
    fn bounded_retention_reports_loss_without_changing_batch_schema() {
        let batcher = MetricsBatcher::with_capacity(2);
        for sequence in 0..4 {
            batcher.on_event(&event("run-1", sequence));
        }
        assert_eq!(batcher.pending_events(), 2);
        assert_eq!(batcher.dropped_events(), 2);
        let batches = batcher.drain();
        assert_eq!(
            batches[0]
                .events
                .iter()
                .map(|e| e.sequence)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        let original = serde_json::to_value(&batches[0]).unwrap();
        let retry: MetricBatch = serde_json::from_value(original.clone()).unwrap();
        assert_eq!(retry.run_id, batches[0].run_id);
        assert_eq!(retry.sequence_number, batches[0].sequence_number);
        assert_eq!(retry.schema_version, METRICS_SCHEMA_VERSION);
        assert_eq!(batcher.dropped_events(), 2);
        assert!(batcher.drain().is_empty());
        let disabled = MetricsBatcher::with_capacity(0);
        disabled.on_event(&event("run-1", 0));
        assert_eq!(disabled.dropped_events(), 1);
        assert!(disabled.drain().is_empty());
    }

    #[test]
    fn groups_events_by_run_and_sorts_sequences() {
        let batcher = Arc::new(MetricsBatcher::new());
        batcher.on_event(&event("run-1", 2));
        batcher.on_event(&event("run-2", 1));
        batcher.on_event(&event("run-1", 0));
        assert_eq!(batcher.pending_events(), 3);

        let batches = batcher.drain();
        assert_eq!(batches.len(), 2);
        let first = batches
            .iter()
            .find(|batch| batch.run_id == "run-1")
            .unwrap();
        assert_eq!(
            first
                .events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(batcher.pending_events(), 0);
        assert!(batcher.drain().is_empty());
    }
}
