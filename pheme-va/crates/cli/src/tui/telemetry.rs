use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::Arc;

use metrics::{MetricEvent, MetricScope, MetricValue, MetricsSubscriber};

use super::logs::LogStore;

pub const METRIC_QUEUE_CAPACITY: usize = 2_048;

pub struct MetricForwarder {
    sender: SyncSender<MetricEvent>,
    dropped: Arc<AtomicU64>,
}

impl MetricForwarder {
    pub fn new(sender: SyncSender<MetricEvent>, dropped: Arc<AtomicU64>) -> Self {
        Self { sender, dropped }
    }
}

impl MetricsSubscriber for MetricForwarder {
    fn on_event(&self, event: &MetricEvent) {
        if self.sender.try_send(event.clone()).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeriesKey {
    pub name: String,
    pub scope: MetricScope,
    pub source: String,
    pub unit: metrics::MetricUnit,
}

impl SeriesKey {
    fn of(event: &MetricEvent) -> Self {
        Self {
            name: event.name.clone(),
            scope: event.scope,
            source: event.source.clone(),
            unit: event.unit,
        }
    }
}

pub struct MetricSeries<'a> {
    pub key: SeriesKey,
    pub samples: Vec<&'a MetricEvent>,
}

impl MetricSeries<'_> {
    pub fn latest(&self) -> &MetricEvent {
        self.samples[self.samples.len() - 1]
    }

    pub fn category(&self) -> &'static str {
        match self.key.unit {
            metrics::MetricUnit::Milliseconds => "Timing",
            metrics::MetricUnit::Watts
            | metrics::MetricUnit::Joules
            | metrics::MetricUnit::WattHours => "Power / energy",
            metrics::MetricUnit::Bytes
            | metrics::MetricUnit::Percent
            | metrics::MetricUnit::Celsius => "Resources",
            metrics::MetricUnit::Count => "Counts",
            _ => "Status",
        }
    }

    // Match the whole series, not individual points, so filtering never distorts history.
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return true;
        }
        self.samples.iter().any(|event| {
            format!(
                "{} {} {:?} {} {} {} {}",
                self.category(),
                event.name,
                event.scope,
                event.source,
                TelemetryStore::unit_text(event),
                event.run_id,
                TelemetryStore::value_text(event)
            )
            .to_ascii_lowercase()
            .contains(&query)
        })
    }

    pub fn history(&self, origin_ms: u64) -> Vec<(f64, f64)> {
        self.samples
            .iter()
            .filter_map(|event| {
                numeric_value(event).map(|value| {
                    (
                        event.timestamp_ms.saturating_sub(origin_ms) as f64 / 1000.0,
                        value,
                    )
                })
            })
            .collect()
    }
}

fn numeric_value(event: &MetricEvent) -> Option<f64> {
    if event.unavailable_reason.is_some() {
        return None;
    }
    match event.value {
        Some(MetricValue::Number(value)) if value.is_finite() => Some(value),
        Some(MetricValue::Integer(value)) => Some(value as f64),
        _ => None,
    }
}

#[derive(Debug)]
pub struct TelemetryStore {
    events: VecDeque<MetricEvent>,
    max_events: usize,
    selected_run: Option<String>,
    last_dropped: u64,
    selected_series: Option<SeriesKey>,
}

impl TelemetryStore {
    pub fn new(max_events: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(max_events.min(512)),
            max_events: max_events.max(1),
            selected_run: None,
            last_dropped: 0,
            selected_series: None,
        }
    }

    pub fn push(&mut self, event: MetricEvent) {
        if self.selected_run.is_none() {
            self.selected_run = Some(event.run_id.clone());
        }
        if self.events.len() == self.max_events {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    pub fn drain(&mut self, receiver: &Receiver<MetricEvent>) {
        while let Ok(event) = receiver.try_recv() {
            self.push(event);
        }
    }

    pub fn observe_drops(&mut self, dropped: u64, logs: &mut LogStore) {
        if dropped > self.last_dropped {
            let difference = dropped - self.last_dropped;
            logs.warn(
                "metrics",
                format!("dropped {difference} metric event(s) because the queue was full"),
            );
            self.last_dropped = dropped;
        }
    }

    pub fn events(&self) -> impl Iterator<Item = &MetricEvent> {
        self.events.iter()
    }

    pub fn events_for(&self, run_id: Option<&str>) -> Vec<&MetricEvent> {
        let run_id = run_id.or(self.selected_run.as_deref());
        self.events
            .iter()
            .filter(|event| run_id.map_or(true, |run| event.run_id == run))
            .collect()
    }

    pub fn run_ids(&self) -> Vec<String> {
        let mut runs = Vec::new();
        for event in &self.events {
            if !runs.iter().any(|run| run == &event.run_id) {
                runs.push(event.run_id.clone());
            }
        }
        runs
    }

    pub fn selected_run(&self) -> Option<&str> {
        self.selected_run.as_deref()
    }

    pub fn select_run(&mut self, run_id: Option<String>) {
        self.selected_run = run_id;
    }

    pub fn selected_events(&self) -> Vec<&MetricEvent> {
        self.events_for(None)
    }

    pub fn series(&self, query: &str) -> Vec<MetricSeries<'_>> {
        let mut series: Vec<MetricSeries<'_>> = Vec::new();
        for event in self.selected_events() {
            let key = SeriesKey::of(event);
            if let Some(existing) = series.iter_mut().find(|series| series.key == key) {
                existing.samples.push(event);
            } else {
                series.push(MetricSeries {
                    key,
                    samples: vec![event],
                });
            }
        }
        for series in &mut series {
            series
                .samples
                .sort_by_key(|event| (event.timestamp_ms, event.sequence));
        }
        series.retain(|series| series.matches(query));
        series.sort_by_key(|series| {
            (
                series.category(),
                series.key.name.clone(),
                format!("{:?}", series.key.scope),
                series.key.source.clone(),
                TelemetryStore::unit_text(series.latest()),
            )
        });
        series
    }

    pub fn series_index(&self, series: &[MetricSeries<'_>]) -> usize {
        self.selected_series
            .as_ref()
            .and_then(|key| series.iter().position(|series| &series.key == key))
            .unwrap_or(0)
    }

    pub fn move_series(&mut self, query: &str, delta: isize) {
        let series = self.series(query);
        if series.is_empty() {
            return;
        }
        let index = (self.series_index(&series) as isize + delta)
            .clamp(0, series.len() as isize - 1) as usize;
        self.selected_series = Some(series[index].key.clone());
    }

    pub fn run_origin_ms(&self) -> u64 {
        self.selected_events()
            .iter()
            .map(|event| event.timestamp_ms)
            .min()
            .unwrap_or(0)
    }

    pub fn latest_value(
        &self,
        name: &str,
        scope: Option<MetricScope>,
        run_id: Option<&str>,
    ) -> Option<&MetricEvent> {
        self.events.iter().rev().find(|event| {
            event.name == name
                && scope.map_or(true, |expected| event.scope == expected)
                && run_id
                    .or(self.selected_run.as_deref())
                    .map_or(true, |run| event.run_id == run)
        })
    }

    pub fn value_text(event: &MetricEvent) -> String {
        if let Some(reason) = &event.unavailable_reason {
            return format!("unavailable: {reason}");
        }
        match &event.value {
            Some(MetricValue::Number(value)) if !value.is_finite() => {
                "unavailable: non-finite sample".to_owned()
            }
            Some(MetricValue::Number(value)) => format_number(*value),
            Some(MetricValue::Integer(value)) => value.to_string(),
            Some(MetricValue::Text(value)) => value.clone(),
            Some(MetricValue::Boolean(value)) => value.to_string(),
            None => format!(
                "unavailable: {}",
                event
                    .unavailable_reason
                    .as_deref()
                    .unwrap_or("no reason provided")
            ),
        }
    }

    pub fn unit_text(event: &MetricEvent) -> &'static str {
        match event.unit {
            metrics::MetricUnit::Milliseconds => "ms",
            metrics::MetricUnit::Count => "count",
            metrics::MetricUnit::Percent => "%",
            metrics::MetricUnit::Bytes => "bytes",
            metrics::MetricUnit::Celsius => "°C",
            metrics::MetricUnit::Joules => "J",
            metrics::MetricUnit::Watts => "W",
            metrics::MetricUnit::WattHours => "Wh",
            metrics::MetricUnit::Status => "status",
            metrics::MetricUnit::Boolean => "bool",
        }
    }
}

impl Default for TelemetryStore {
    fn default() -> Self {
        Self::new(10_000)
    }
}

fn format_number(value: f64) -> String {
    if value.abs() >= 100.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.3}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics::{MetricScope, MetricUnit, METRICS_SCHEMA_VERSION};

    fn event(run_id: &str, name: &str, value: Option<MetricValue>) -> MetricEvent {
        MetricEvent {
            schema_version: METRICS_SCHEMA_VERSION,
            experiment_id: None,
            run_id: run_id.to_owned(),
            sequence: 0,
            timestamp_ms: 1,
            name: name.to_owned(),
            value,
            unit: MetricUnit::Milliseconds,
            scope: MetricScope::Run,
            source: "test".to_owned(),
            unavailable_reason: None,
        }
    }

    #[test]
    fn keeps_runs_separate_in_values_and_history() {
        let mut store = TelemetryStore::new(8);
        store.push(event(
            "run-1",
            "end_to_end_request_duration_ms",
            Some(MetricValue::Number(10.0)),
        ));
        store.push(event(
            "run-2",
            "end_to_end_request_duration_ms",
            Some(MetricValue::Number(20.0)),
        ));
        store.select_run(Some("run-1".to_owned()));

        assert_eq!(store.selected_events().len(), 1);
        assert_eq!(
            store.series("")[0].history(store.run_origin_ms()),
            vec![(0.0, 10.0)]
        );
        assert_eq!(
            store
                .latest_value("end_to_end_request_duration_ms", None, None)
                .and_then(numeric_value),
            Some(10.0)
        );
        store.select_run(Some("run-2".into()));
        assert_eq!(
            store.series("")[0].history(store.run_origin_ms()),
            vec![(0.0, 20.0)]
        );
    }

    #[test]
    fn series_identity_includes_scope_source_unit_and_selected_run() {
        let mut store = TelemetryStore::new(20);
        let base = event("run-1", "cpu", Some(MetricValue::Number(0.25)));
        store.push(base.clone());
        store.push(base.clone());
        store.push(MetricEvent {
            scope: MetricScope::Process,
            ..base.clone()
        });
        store.push(MetricEvent {
            source: "other".into(),
            ..base.clone()
        });
        store.push(MetricEvent {
            unit: MetricUnit::Percent,
            ..base.clone()
        });
        store.push(MetricEvent {
            run_id: "run-2".into(),
            ..base
        });
        let series = store.series("");
        assert_eq!(series.len(), 4);
        assert_eq!(
            series
                .iter()
                .map(|series| series.samples.len())
                .sum::<usize>(),
            5
        );
        assert_eq!(store.series("  RESOURCES ").len(), 1);
        assert_eq!(store.series("process").len(), 1);
        assert_eq!(store.series("other").len(), 1);
        assert_eq!(store.series("%").len(), 1);
        assert_eq!(store.series("run-2").len(), 0);
        store.select_run(Some("run-2".into()));
        assert_eq!(store.series("").len(), 1);
    }

    #[test]
    fn history_keeps_fractional_negative_values_and_gaps_in_time() {
        let mut store = TelemetryStore::new(20);
        for (timestamp_ms, value, reason) in [
            (4000, Some(MetricValue::Number(0.25)), None),
            (1000, Some(MetricValue::Number(-0.5)), None),
            (2000, None, Some("sensor offline".into())),
            (2500, Some(MetricValue::Number(f64::NAN)), None),
            (
                3000,
                Some(MetricValue::Number(99.0)),
                Some("invalid".into()),
            ),
        ] {
            store.push(MetricEvent {
                timestamp_ms,
                unavailable_reason: reason,
                ..event("run-1", "temperature", value)
            });
        }
        let series = store.series("offline");
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].samples.len(), 5);
        assert_eq!(
            series[0].history(store.run_origin_ms()),
            vec![(0.0, -0.5), (3.0, 0.25)]
        );
        assert_eq!(series[0].latest().timestamp_ms, 4000);
    }

    #[test]
    fn selection_is_stable_and_recovers_from_filtering_and_eviction() {
        let mut store = TelemetryStore::new(2);
        store.push(event("run-1", "alpha", None));
        store.push(event("run-1", "beta", None));
        store.move_series("", 1);
        assert_eq!(store.series_index(&store.series("")), 1);
        assert_eq!(store.series_index(&store.series("alpha")), 0);
        assert_eq!(store.series_index(&store.series("")), 1);
        store.move_series("no matches", 1);
        store.push(event("run-1", "gamma", None));
        assert_eq!(store.series_index(&store.series("")), 0);
        store.push(event("run-1", "delta", None));
        assert_eq!(store.series_index(&store.series("")), 0);
    }

    #[test]
    fn preserves_unavailable_reasons() {
        let event = MetricEvent {
            unavailable_reason: Some("sensor unavailable".to_owned()),
            ..event("run-1", "temperature_celsius", None)
        };
        assert_eq!(
            TelemetryStore::value_text(&event),
            "unavailable: sensor unavailable"
        );
    }
}
