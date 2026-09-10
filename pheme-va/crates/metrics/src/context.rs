use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::event::{MetricSample, Stage};
use crate::hub::MetricsHub;
use crate::resources::{Measurement, ResourceSampler, ResourceSnapshot};
use crate::{MetricEvent, MetricScope, MetricUnit, METRICS_SCHEMA_VERSION};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub incident_active: bool,
    pub resource_sampling: bool,
}

impl MetricsConfig {
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            incident_active: false,
            resource_sampling: false,
        }
    }
}

/// Per-run metrics publisher. It owns run metadata and centrally applies the
/// activation flags before publishing an event to the shared hub.
#[derive(Clone)]
pub struct MetricsContext {
    run_id: Arc<str>,
    experiment_id: Option<Arc<str>>,
    config: MetricsConfig,
    hub: Arc<MetricsHub>,
    next_sequence: Arc<AtomicU64>,
}

impl MetricsContext {
    pub fn new(
        run_id: impl Into<String>,
        experiment_id: Option<String>,
        config: MetricsConfig,
        hub: Arc<MetricsHub>,
    ) -> Self {
        Self {
            run_id: Arc::from(run_id.into()),
            experiment_id: experiment_id.map(Arc::from),
            config,
            hub,
            next_sequence: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn disabled() -> Self {
        Self::new(
            "disabled",
            None,
            MetricsConfig::default(),
            Arc::new(MetricsHub::new()),
        )
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn experiment_id(&self) -> Option<&str> {
        self.experiment_id.as_deref()
    }

    pub fn config(&self) -> &MetricsConfig {
        &self.config
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    pub(crate) fn resource_sampling_enabled(&self) -> bool {
        self.config.enabled && self.config.resource_sampling
    }

    /// Publish a custom event sample. Incident-only filtering is deliberately
    /// handled here rather than being scattered throughout core operations.
    pub fn record(&self, sample: MetricSample) -> Option<MetricEvent> {
        if !self.config.enabled || (sample.incident_only && !self.config.incident_active) {
            return None;
        }

        let event = MetricEvent {
            schema_version: METRICS_SCHEMA_VERSION,
            experiment_id: self
                .experiment_id
                .as_ref()
                .map(|experiment_id| experiment_id.to_string()),
            run_id: self.run_id.to_string(),
            sequence: self.next_sequence.fetch_add(1, Ordering::Relaxed),
            timestamp_ms: epoch_millis(),
            name: sample.name,
            value: sample.value,
            unit: sample.unit,
            scope: sample.scope,
            source: sample.source,
            unavailable_reason: sample.unavailable_reason,
        };
        self.hub.publish(&event);
        Some(event)
    }

    pub fn start_stage(&self, stage: Stage) -> MetricsTimer {
        MetricsTimer {
            context: self.clone(),
            stage,
            started: Instant::now(),
            finished: false,
        }
    }

    pub fn record_retry_count(&self, retries: u64) -> Option<MetricEvent> {
        self.record(MetricSample::integer(
            "retry_count",
            retries,
            MetricUnit::Count,
            MetricScope::Run,
            "core.engine",
        ))
    }

    pub fn record_transcript_status(&self, status: impl Into<String>) -> Option<MetricEvent> {
        self.record(MetricSample::text(
            "transcript_status",
            status,
            MetricUnit::Status,
            MetricScope::Run,
            "core.engine",
        ))
    }

    pub fn record_workflow_outcome(&self, success: bool) -> Option<MetricEvent> {
        self.record(MetricSample::text(
            "workflow_outcome",
            if success { "success" } else { "failure" },
            MetricUnit::Status,
            MetricScope::Run,
            "core.workflow",
        ))
    }

    pub fn sample_resources(&self, sampler: &mut dyn ResourceSampler) -> ResourceSnapshot {
        if !self.resource_sampling_enabled() {
            return ResourceSnapshot::unavailable(
                sampler.source(),
                "resource sampling disabled for this run",
            );
        }
        let snapshot = sampler.sample();
        self.record_resource_snapshot(&snapshot);
        snapshot
    }

    pub fn record_resource_snapshot(&self, snapshot: &ResourceSnapshot) {
        if !self.resource_sampling_enabled() {
            return;
        }

        self.record_resource_measurement(
            "process_cpu_percent",
            &snapshot.process_cpu_percent,
            MetricUnit::Percent,
            MetricScope::Process,
            &snapshot.source,
        );
        self.record_resource_measurement(
            "system_cpu_percent",
            &snapshot.system_cpu_percent,
            MetricUnit::Percent,
            MetricScope::System,
            &snapshot.source,
        );
        self.record_resource_measurement(
            "ram_usage_bytes",
            &snapshot.process_ram_bytes,
            MetricUnit::Bytes,
            MetricScope::Process,
            &snapshot.source,
        );
        self.record_resource_measurement(
            "ram_usage_bytes",
            &snapshot.system_ram_bytes,
            MetricUnit::Bytes,
            MetricScope::System,
            &snapshot.source,
        );
        self.record_resource_measurement(
            "gpu_usage_percent",
            &snapshot.gpu_usage_percent,
            MetricUnit::Percent,
            MetricScope::Device,
            &snapshot.source,
        );
        self.record_resource_measurement(
            "temperature_celsius",
            &snapshot.temperature_celsius,
            MetricUnit::Celsius,
            MetricScope::Device,
            &snapshot.source,
        );
        self.record_resource_measurement(
            "battery_drain_percent",
            &snapshot.battery_drain_percent,
            MetricUnit::Percent,
            MetricScope::Device,
            &snapshot.source,
        );
        self.record_resource_measurement(
            "whole_device_power_watts",
            &snapshot.whole_device_power_watts,
            MetricUnit::Watts,
            MetricScope::Device,
            &snapshot.source,
        );
        self.record_resource_measurement(
            "energy_joules",
            &snapshot.energy_joules,
            MetricUnit::Joules,
            MetricScope::Device,
            &snapshot.source,
        );
        self.record_resource_measurement(
            "energy_watt_hours",
            &snapshot.energy_watt_hours,
            MetricUnit::WattHours,
            MetricScope::Device,
            &snapshot.source,
        );
    }

    fn record_resource_measurement(
        &self,
        name: &'static str,
        measurement: &Measurement<f64>,
        unit: MetricUnit,
        scope: MetricScope,
        source: &str,
    ) {
        let sample = match measurement {
            Measurement::Available(value) => {
                MetricSample::number(name, *value, unit, scope, source)
            }
            Measurement::Unavailable { reason } => {
                MetricSample::unavailable(name, unit, scope, source, reason)
            }
        };
        self.record(sample);
    }
}

/// Records one stage duration when finished or dropped. Dropping a timer is
/// treated as completion so error returns still retain the time spent in the
/// stage.
pub struct MetricsTimer {
    context: MetricsContext,
    stage: Stage,
    started: Instant,
    finished: bool,
}

impl MetricsTimer {
    pub fn finish(mut self) {
        self.finish_in_place();
    }

    pub fn cancel(mut self) {
        self.finished = true;
    }

    fn finish_in_place(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        let elapsed_ms = self.started.elapsed().as_secs_f64() * 1_000.0;
        let (name, scope, incident_only) = self.stage.sample_details();
        let mut sample = MetricSample::number(
            name,
            elapsed_ms,
            MetricUnit::Milliseconds,
            scope,
            stage_source(self.stage),
        );
        if incident_only {
            sample = sample.incident_only();
        }
        self.context.record(sample);
    }
}

impl Drop for MetricsTimer {
    fn drop(&mut self) {
        self.finish_in_place();
    }
}

fn stage_source(stage: Stage) -> &'static str {
    match stage {
        Stage::AudioNormalization => "core.audio",
        Stage::SpeechGate => "core.audio",
        Stage::WhisperTranscription => "core.transcription",
        Stage::IncidentAnalysis => "core.incident",
        Stage::EndToEndRequest => "core.workflow",
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
    use std::sync::{Arc, Mutex};
    use std::thread;

    use super::*;
    use crate::{MetricValue, MetricsSubscriber};

    #[derive(Default)]
    struct Collector {
        events: Mutex<Vec<MetricEvent>>,
    }

    impl MetricsSubscriber for Collector {
        fn on_event(&self, event: &MetricEvent) {
            self.events.lock().unwrap().push(event.clone());
        }
    }

    #[test]
    fn filters_incident_events_centrally() {
        let hub = Arc::new(MetricsHub::new());
        let collector = Arc::new(Collector::default());
        let _subscription = hub.subscribe(Arc::clone(&collector));
        let context = MetricsContext::new(
            "run-1",
            None,
            MetricsConfig {
                enabled: true,
                incident_active: false,
                resource_sampling: false,
            },
            hub,
        );
        context.record(
            MetricSample::text(
                "incident_state",
                "active",
                MetricUnit::Status,
                MetricScope::Run,
                "test",
            )
            .incident_only(),
        );
        context.record_retry_count(0);
        let events = collector.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "retry_count");
        assert_eq!(events[0].sequence, 0);
    }

    #[test]
    fn timer_records_duration_and_metadata() {
        let hub = Arc::new(MetricsHub::new());
        let collector = Arc::new(Collector::default());
        let _subscription = hub.subscribe(Arc::clone(&collector));
        let context = MetricsContext::new(
            "run-1",
            None,
            MetricsConfig {
                enabled: true,
                incident_active: true,
                resource_sampling: false,
            },
            hub,
        );
        let timer = context.start_stage(Stage::EndToEndRequest);
        thread::sleep(std::time::Duration::from_millis(1));
        timer.finish();
        let events = collector.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "end_to_end_request_duration_ms");
        assert!(matches!(events[0].value, Some(MetricValue::Number(value)) if value >= 0.0));
    }

    #[test]
    fn status_and_workflow_events_are_scalars() {
        let hub = Arc::new(MetricsHub::new());
        let collector = Arc::new(Collector::default());
        let _subscription = hub.subscribe(Arc::clone(&collector));
        let context = MetricsContext::new(
            "run-1",
            None,
            MetricsConfig {
                enabled: true,
                incident_active: false,
                resource_sampling: false,
            },
            hub,
        );
        context.record_transcript_status("speech");
        context.record_workflow_outcome(false);
        let events = collector.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].value, Some(MetricValue::Text(ref value)) if value == "speech"));
        assert!(
            matches!(events[1].value, Some(MetricValue::Text(ref value)) if value == "failure")
        );
    }
}
