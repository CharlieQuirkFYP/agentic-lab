use metrics::{MetricsContext, Stage};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IncidentReport {
    pub incident_type: String,
    pub location: String,
    pub severity: String,
    pub summary: String,
    pub recommended_action: String,
}

#[derive(Debug, Error)]
pub enum IncidentError {
    #[error("incident analysis returned an invalid report")]
    InvalidReport,
}

pub trait IncidentAnalyzer: Send {
    fn analyze(&mut self, transcript: &str) -> Result<IncidentReport, IncidentError>;
}

/// Run the pure transcript-to-report analyzer while publishing its optional
/// incident-specific timing through the supplied per-run metrics context.
pub fn analyze_with_metrics<A>(
    analyzer: &mut A,
    transcript: &str,
    metrics: &MetricsContext,
) -> Result<IncidentReport, IncidentError>
where
    A: IncidentAnalyzer + ?Sized,
{
    let timer = metrics.start_stage(Stage::IncidentAnalysis);
    let result = analyzer.analyze(transcript);
    timer.finish();
    result
}

/// A conservative, deterministic baseline for development and offline tests.
/// It only copies facts that are explicit in the transcript; a replaceable LLM
/// adapter can implement the same trait once model-backed extraction is added.
#[derive(Clone, Debug, Default)]
pub struct RuleBasedIncidentAnalyzer;

impl IncidentAnalyzer for RuleBasedIncidentAnalyzer {
    fn analyze(&mut self, transcript: &str) -> Result<IncidentReport, IncidentError> {
        let transcript = transcript.trim();
        if transcript.is_empty() {
            return Err(IncidentError::InvalidReport);
        }

        let lower = transcript.to_ascii_lowercase();
        let incident_type =
            if lower.contains("collision") || lower.contains("collid") || lower.contains("crash") {
                "collision"
            } else if lower.contains("smoke") {
                "smoke report"
            } else if lower.contains("fire") {
                "fire report"
            } else if lower.contains("injur") {
                "injury report"
            } else {
                "unknown"
            };
        let location = extract_after_marker(transcript, &["near ", "at ", "by ", "in "])
            .unwrap_or_else(|| "unknown".to_owned());
        let severity = ["low", "medium", "high"]
            .iter()
            .find(|severity| {
                lower
                    .split(|character: char| !character.is_ascii_alphabetic())
                    .any(|word| word == **severity)
            })
            .copied()
            .unwrap_or("unknown");
        let recommended_action = extract_action(transcript).unwrap_or_else(|| "unknown".to_owned());

        Ok(IncidentReport {
            incident_type: incident_type.to_owned(),
            location,
            severity: severity.to_owned(),
            summary: transcript.to_owned(),
            recommended_action,
        })
    }
}

fn extract_after_marker(transcript: &str, markers: &[&str]) -> Option<String> {
    let lower = transcript.to_ascii_lowercase();
    let (index, marker) = markers
        .iter()
        .filter_map(|marker| lower.find(marker).map(|index| (index, *marker)))
        .min_by_key(|(index, _)| *index)?;
    let start = index + marker.len();
    let value = transcript[start..]
        .split(['.', '!', '?', ',', ';'])
        .next()
        .unwrap_or_default()
        .trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn extract_action(transcript: &str) -> Option<String> {
    let lower = transcript.to_ascii_lowercase();
    let marker = ["please ", "notify ", "call ", "contact "]
        .iter()
        .filter_map(|marker| lower.find(marker).map(|index| (index, *marker)))
        .min_by_key(|(index, _)| *index)?;
    let action = transcript[marker.0..]
        .split(['.', '!', '?'])
        .next()
        .unwrap_or_default()
        .trim();
    (!action.is_empty()).then(|| {
        let mut chars = action.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_explicit_incident_facts() {
        let mut analyzer = RuleBasedIncidentAnalyzer;
        let report = analyzer
            .analyze(
                "A vehicle collided with a barrier near the west entrance. Severity is low. Please notify the site supervisor.",
            )
            .unwrap();
        assert_eq!(report.incident_type, "collision");
        assert_eq!(report.location, "the west entrance");
        assert_eq!(report.severity, "low");
        assert_eq!(
            report.recommended_action,
            "Please notify the site supervisor"
        );
    }

    #[test]
    fn keeps_unknown_values_when_facts_are_missing() {
        let mut analyzer = RuleBasedIncidentAnalyzer;
        let report = analyzer.analyze("There is smoke.").unwrap();
        assert_eq!(report.incident_type, "smoke report");
        assert_eq!(report.location, "unknown");
        assert_eq!(report.severity, "unknown");
        assert_eq!(report.recommended_action, "unknown");
    }
}
