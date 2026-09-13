use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;

use metrics::{MetricEvent, MetricScope, MetricUnit, MetricValue, MetricsSubscriber};
use va_core::TranscriptionResult;

use super::logs::LogStore;

pub const METRIC_QUEUE_CAPACITY: usize = 2_048;

pub struct MetricForwarder {
    sender: std::sync::mpsc::SyncSender<MetricEvent>,
    dropped: std::sync::Arc<AtomicU64>,
}

impl MetricForwarder {
    pub fn new(
        sender: std::sync::mpsc::SyncSender<MetricEvent>,
        dropped: std::sync::Arc<AtomicU64>,
    ) -> Self {
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
    pub category: String,
    pub scope: MetricScope,
    pub source: String,
    pub unit: MetricUnit,
}

impl SeriesKey {
    fn of(event: &MetricEvent) -> Self {
        Self {
            name: event.name.clone(),
            category: category_for(event.unit).to_owned(),
            scope: event.scope,
            source: event.source.clone(),
            unit: event.unit,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricStats {
    pub latest: Option<f64>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub average: Option<f64>,
    pub count: usize,
    pub unavailable_count: usize,
    pub first_timestamp_ms: Option<u64>,
    pub last_timestamp_ms: Option<u64>,
}

pub struct MetricSeries<'a> {
    pub key: SeriesKey,
    pub samples: Vec<&'a MetricEvent>,
}

impl MetricSeries<'_> {
    pub fn latest(&self) -> &MetricEvent {
        self.samples.last().expect("metric series is non-empty")
    }

    pub fn category(&self) -> &str {
        &self.key.category
    }

    pub fn stats(&self) -> MetricStats {
        let mut stats = MetricStats {
            count: self.samples.len(),
            ..MetricStats::default()
        };
        let mut total = 0.0;
        for event in &self.samples {
            if let Some(value) = numeric_value(event) {
                stats.latest = Some(value);
                stats.minimum = Some(stats.minimum.map_or(value, |old| old.min(value)));
                stats.maximum = Some(stats.maximum.map_or(value, |old| old.max(value)));
                total += value;
            } else {
                stats.unavailable_count += 1;
            }
            stats.first_timestamp_ms = Some(
                stats
                    .first_timestamp_ms
                    .map_or(event.timestamp_ms, |old| old.min(event.timestamp_ms)),
            );
            stats.last_timestamp_ms = Some(
                stats
                    .last_timestamp_ms
                    .map_or(event.timestamp_ms, |old| old.max(event.timestamp_ms)),
            );
        }
        let numeric_count = stats.count.saturating_sub(stats.unavailable_count);
        if numeric_count > 0 {
            stats.average = Some(total / numeric_count as f64);
        }
        stats
    }

    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        query.is_empty()
            || self.samples.iter().any(|event| {
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

    /// Numeric points only. Unavailable samples are intentionally omitted so a
    /// chart renderer can draw them as gaps rather than inventing zeroes.
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

fn category_for(unit: MetricUnit) -> &'static str {
    match unit {
        MetricUnit::Milliseconds => "Timing",
        MetricUnit::Watts | MetricUnit::Joules | MetricUnit::WattHours => "Power / Energy",
        MetricUnit::Bytes | MetricUnit::Percent | MetricUnit::Celsius => "Resources",
        MetricUnit::Count => "Counts",
        MetricUnit::Status | MetricUnit::Boolean => "Status",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetricRow {
    Category(String),
    Metric(SeriesKey),
}

#[derive(Clone, Debug)]
pub struct RunReport {
    pub run_id: String,
    pub model_id: String,
    pub model_family: String,
    pub backend: String,
    pub runtime: Option<String>,
    pub revision: Option<String>,
    pub source: String,
    pub status: String,
    pub transcript: String,
    pub raw_transcript: String,
    pub language: Option<String>,
    pub audio_duration_seconds: f32,
    pub gate_decision: String,
    pub segment_count: usize,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub error: Option<String>,
    pub result: Option<TranscriptionResult>,
    pub events: Vec<MetricEvent>,
}

impl RunReport {
    pub fn status_text(&self) -> &str {
        &self.status
    }

    pub fn samples(&self) -> &[MetricEvent] {
        &self.events
    }
}

#[derive(Debug)]
pub struct TelemetryStore {
    events: VecDeque<MetricEvent>,
    reports: VecDeque<RunReport>,
    max_events: usize,
    max_reports: usize,
    selected_run: Option<String>,
    active_run: Option<String>,
    last_dropped: u64,
    selected_series: Option<SeriesKey>,
}

impl TelemetryStore {
    pub fn new(max_events: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(max_events.min(512)),
            reports: VecDeque::with_capacity(64),
            max_events: max_events.max(1),
            max_reports: 100,
            selected_run: None,
            active_run: None,
            last_dropped: 0,
            selected_series: None,
        }
    }

    pub fn push(&mut self, event: MetricEvent) {
        if self.events.len() == self.max_events {
            self.events.pop_front();
        }
        if let Some(report) = self
            .reports
            .iter_mut()
            .find(|report| report.status == "BUSY" && report.run_id == event.run_id)
        {
            report.events.push(event.clone());
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
        self.events
            .iter()
            .filter(|event| run_id.map_or(true, |run| event.run_id == run))
            .collect()
    }

    pub fn series(&self, query: &str) -> Vec<MetricSeries<'_>> {
        self.series_from(self.events.iter(), query)
    }

    pub fn series_for_run(&self, run_id: &str) -> Vec<MetricSeries<'_>> {
        let Some(report) = self.reports.iter().find(|report| report.run_id == run_id) else {
            return self.series_from(
                self.events.iter().filter(|event| event.run_id == run_id),
                "",
            );
        };
        self.series_from(report.events.iter(), "")
    }

    fn series_from<'a, I>(&'a self, events: I, query: &str) -> Vec<MetricSeries<'a>>
    where
        I: Iterator<Item = &'a MetricEvent>,
    {
        let mut series = Vec::<MetricSeries<'a>>::new();
        for event in events {
            let key = SeriesKey::of(event);
            if let Some(existing) = series.iter_mut().find(|item| item.key == key) {
                existing.samples.push(event);
            } else {
                series.push(MetricSeries {
                    key,
                    samples: vec![event],
                });
            }
        }
        for item in &mut series {
            item.samples
                .sort_by_key(|event| (event.timestamp_ms, event.sequence));
        }
        if !query.trim().is_empty() {
            series.retain(|item| item.matches(query));
        }
        series.sort_by(|left, right| {
            left.key
                .category
                .cmp(&right.key.category)
                .then_with(|| left.key.name.cmp(&right.key.name))
                .then_with(|| {
                    format!("{:?}", left.key.scope).cmp(&format!("{:?}", right.key.scope))
                })
                .then_with(|| left.key.source.cmp(&right.key.source))
        });
        series
    }

    pub fn metric_rows(&self, _query: &str) -> Vec<MetricRow> {
        let series = self.series("");
        let mut rows = Vec::new();
        let mut category = None;
        for item in series {
            if category.as_deref() != Some(item.category()) {
                category = Some(item.category().to_owned());
                rows.push(MetricRow::Category(item.category().to_owned()));
            }
            rows.push(MetricRow::Metric(item.key));
        }
        rows
    }

    pub fn selected_metric_index(&self, rows: &[MetricRow]) -> usize {
        self.selected_series
            .as_ref()
            .and_then(|key| {
                rows.iter()
                    .position(|row| matches!(row, MetricRow::Metric(candidate) if candidate == key))
            })
            .unwrap_or_else(|| {
                rows.iter()
                    .position(|row| matches!(row, MetricRow::Metric(_)))
                    .unwrap_or(0)
            })
    }

    pub fn move_metric(&mut self, query: &str, delta: isize) {
        let rows = self.metric_rows(query);
        let metrics = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| matches!(row, MetricRow::Metric(_)).then_some(index))
            .collect::<Vec<_>>();
        if metrics.is_empty() {
            return;
        }
        let current = self.selected_metric_index(&rows);
        let position = metrics
            .iter()
            .position(|index| *index == current)
            .unwrap_or(0);
        let next = (position as isize + delta).clamp(0, metrics.len() as isize - 1) as usize;
        if let MetricRow::Metric(key) = &rows[metrics[next]] {
            self.selected_series = Some(key.clone());
        }
    }

    pub fn selected_metric(&self, query: &str) -> Option<MetricRow> {
        let rows = self.metric_rows(query);
        rows.get(self.selected_metric_index(&rows)).cloned()
    }

    pub fn series_index(&self, series: &[MetricSeries<'_>]) -> usize {
        self.selected_series
            .as_ref()
            .and_then(|key| series.iter().position(|item| &item.key == key))
            .unwrap_or(0)
    }

    #[allow(dead_code)]
    pub fn move_series(&mut self, query: &str, delta: isize) {
        self.move_metric(query, delta);
    }

    pub fn run_origin_ms(&self) -> u64 {
        self.events
            .iter()
            .map(|event| event.timestamp_ms)
            .min()
            .unwrap_or(0)
    }

    pub fn run_origin_for(&self, run_id: &str) -> u64 {
        self.series_for_run(run_id)
            .iter()
            .flat_map(|series| series.samples.iter().map(|event| event.timestamp_ms))
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
                && run_id.map_or(true, |run| event.run_id == run)
        })
    }

    pub fn run_ids(&self) -> Vec<String> {
        self.reports
            .iter()
            .map(|report| report.run_id.clone())
            .collect()
    }

    pub fn reports(&self) -> impl Iterator<Item = &RunReport> {
        self.reports.iter()
    }

    pub fn report(&self, run_id: &str) -> Option<&RunReport> {
        self.reports.iter().find(|report| report.run_id == run_id)
    }

    pub fn selected_report(&self) -> Option<&RunReport> {
        self.selected_run
            .as_deref()
            .and_then(|run| self.report(run))
    }

    pub fn select_run(&mut self, run_id: Option<String>) {
        self.selected_run = run_id;
    }

    pub fn selected_run(&self) -> Option<&str> {
        self.selected_run.as_deref()
    }

    pub fn set_active_run(&mut self, run_id: Option<String>) {
        self.active_run = run_id;
    }

    pub fn active_run(&self) -> Option<&str> {
        self.active_run.as_deref()
    }

    pub fn upsert_report(&mut self, report: RunReport) {
        if let Some(existing) = self
            .reports
            .iter_mut()
            .find(|item| item.run_id == report.run_id)
        {
            *existing = report;
        } else {
            let run_id = report.run_id.clone();
            self.reports.push_front(report);
            if self.selected_run.is_none() {
                self.selected_run = Some(run_id);
            }
            while self.reports.len() > self.max_reports {
                self.reports.pop_back();
            }
        }
    }

    #[allow(dead_code)]
    pub fn selected_events(&self) -> Vec<&MetricEvent> {
        self.events_for(self.selected_run.as_deref())
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
            None => "unavailable: no reason provided".to_owned(),
        }
    }

    pub fn unit_text(event: &MetricEvent) -> &'static str {
        unit_text(event.unit)
    }
}

impl Default for TelemetryStore {
    fn default() -> Self {
        Self::new(10_000)
    }
}

pub fn unit_text(unit: MetricUnit) -> &'static str {
    match unit {
        MetricUnit::Milliseconds => "ms",
        MetricUnit::Count => "count",
        MetricUnit::Percent => "%",
        MetricUnit::Bytes => "bytes",
        MetricUnit::Celsius => "°C",
        MetricUnit::Joules => "J",
        MetricUnit::Watts => "W",
        MetricUnit::WattHours => "Wh",
        MetricUnit::Status => "status",
        MetricUnit::Boolean => "bool",
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
    use metrics::METRICS_SCHEMA_VERSION;

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
    fn live_series_is_not_coupled_to_selected_run() {
        let mut store = TelemetryStore::new(8);
        store.push(event("run-1", "duration", Some(MetricValue::Number(10.0))));
        store.push(event("run-2", "duration", Some(MetricValue::Number(20.0))));
        store.select_run(Some("run-1".into()));
        assert_eq!(store.series("")[0].samples.len(), 2);
    }

    #[test]
    fn stats_preserve_negative_values_and_unavailable_samples() {
        let mut store = TelemetryStore::new(8);
        store.push(event("run-1", "battery", Some(MetricValue::Number(-2.0))));
        store.push(MetricEvent {
            unavailable_reason: Some("sensor offline".into()),
            value: None,
            ..event("run-1", "battery", None)
        });
        let stats = store.series("")[0].stats();
        assert_eq!(stats.minimum, Some(-2.0));
        assert_eq!(stats.maximum, Some(-2.0));
        assert_eq!(stats.average, Some(-2.0));
        assert_eq!(stats.unavailable_count, 1);
    }

    #[test]
    fn metric_rows_include_categories() {
        let mut store = TelemetryStore::new(8);
        store.push(event("run-1", "duration", Some(MetricValue::Number(10.0))));
        let rows = store.metric_rows("");
        assert!(matches!(rows[0], MetricRow::Category(_)));
        assert!(matches!(rows[1], MetricRow::Metric(_)));
    }
}
