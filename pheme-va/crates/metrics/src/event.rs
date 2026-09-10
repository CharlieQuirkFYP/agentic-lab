use serde::{Deserialize, Serialize};

/// Version of the wire representation emitted by this crate.
pub const METRICS_SCHEMA_VERSION: u16 = 1;

/// A scalar metric value. Complex objects are intentionally not part of the
/// event contract so that consumers on desktop, mobile, and the Go API can
/// process events without model-specific schemas.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MetricValue {
    Number(f64),
    Integer(u64),
    Text(String),
    Boolean(bool),
}

impl MetricValue {
    pub fn number(value: f64) -> Option<Self> {
        value.is_finite().then_some(Self::Number(value))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricUnit {
    Milliseconds,
    Count,
    Percent,
    Bytes,
    Celsius,
    Joules,
    Watts,
    WattHours,
    Status,
    Boolean,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricScope {
    Run,
    Process,
    System,
    Device,
}

/// A metric before run metadata and sequence information are assigned by a
/// [`MetricsContext`](crate::MetricsContext).
#[derive(Clone, Debug, PartialEq)]
pub struct MetricSample {
    pub name: String,
    pub value: Option<MetricValue>,
    pub unit: MetricUnit,
    pub scope: MetricScope,
    pub source: String,
    pub unavailable_reason: Option<String>,
    pub incident_only: bool,
}

impl MetricSample {
    pub fn number(
        name: impl Into<String>,
        value: f64,
        unit: MetricUnit,
        scope: MetricScope,
        source: impl Into<String>,
    ) -> Self {
        if let Some(value) = MetricValue::number(value) {
            Self::available(name, value, unit, scope, source)
        } else {
            Self::unavailable(name, unit, scope, source, "metric value was not finite")
        }
    }

    pub fn integer(
        name: impl Into<String>,
        value: u64,
        unit: MetricUnit,
        scope: MetricScope,
        source: impl Into<String>,
    ) -> Self {
        Self::available(name, MetricValue::Integer(value), unit, scope, source)
    }

    pub fn text(
        name: impl Into<String>,
        value: impl Into<String>,
        unit: MetricUnit,
        scope: MetricScope,
        source: impl Into<String>,
    ) -> Self {
        Self::available(name, MetricValue::Text(value.into()), unit, scope, source)
    }

    pub fn boolean(
        name: impl Into<String>,
        value: bool,
        unit: MetricUnit,
        scope: MetricScope,
        source: impl Into<String>,
    ) -> Self {
        Self::available(name, MetricValue::Boolean(value), unit, scope, source)
    }

    pub fn unavailable(
        name: impl Into<String>,
        unit: MetricUnit,
        scope: MetricScope,
        source: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value: None,
            unit,
            scope,
            source: source.into(),
            unavailable_reason: Some(reason.into()),
            incident_only: false,
        }
    }

    pub fn incident_only(mut self) -> Self {
        self.incident_only = true;
        self
    }

    fn available(
        name: impl Into<String>,
        value: MetricValue,
        unit: MetricUnit,
        scope: MetricScope,
        source: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value: Some(value),
            unit,
            scope,
            source: source.into(),
            unavailable_reason: None,
            incident_only: false,
        }
    }
}

/// One metric event with all metadata required by the cross-process API.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MetricEvent {
    pub schema_version: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
    pub run_id: String,
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub name: String,
    pub value: Option<MetricValue>,
    pub unit: MetricUnit,
    pub scope: MetricScope,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

/// A batch ready for a host/API exporter. Batches are grouped by run by the
/// [`MetricsBatcher`](crate::MetricsBatcher).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MetricBatch {
    pub schema_version: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
    pub run_id: String,
    pub sequence_number: u64,
    pub timestamp_ms: u64,
    pub events: Vec<MetricEvent>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    AudioNormalization,
    SpeechGate,
    WhisperTranscription,
    IncidentAnalysis,
    EndToEndRequest,
}

impl Stage {
    pub(crate) fn sample_details(self) -> (&'static str, MetricScope, bool) {
        match self {
            Self::AudioNormalization => {
                ("audio_normalization_duration_ms", MetricScope::Run, false)
            }
            Self::SpeechGate => ("speech_gate_duration_ms", MetricScope::Run, false),
            Self::WhisperTranscription => {
                ("whisper_transcription_duration_ms", MetricScope::Run, false)
            }
            Self::IncidentAnalysis => ("incident_analysis_duration_ms", MetricScope::Run, true),
            Self::EndToEndRequest => ("end_to_end_request_duration_ms", MetricScope::Run, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_values_have_a_reason_and_serialize_as_null() {
        let sample = MetricSample::unavailable(
            "temperature_celsius",
            MetricUnit::Celsius,
            MetricScope::Device,
            "test",
            "sensor unavailable",
        );
        assert!(sample.value.is_none());
        assert_eq!(
            sample.unavailable_reason.as_deref(),
            Some("sensor unavailable")
        );

        let event = MetricEvent {
            schema_version: METRICS_SCHEMA_VERSION,
            experiment_id: None,
            run_id: "run-1".to_owned(),
            sequence: 0,
            timestamp_ms: 1,
            name: sample.name,
            value: sample.value,
            unit: sample.unit,
            scope: sample.scope,
            source: sample.source,
            unavailable_reason: sample.unavailable_reason,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"value\":null"));
        assert!(json.contains("sensor unavailable"));
    }

    #[test]
    fn non_finite_numbers_become_unavailable() {
        let sample = MetricSample::number(
            "latency",
            f64::NAN,
            MetricUnit::Milliseconds,
            MetricScope::Run,
            "test",
        );
        assert!(sample.value.is_none());
        assert!(sample.unavailable_reason.is_some());
    }
}
