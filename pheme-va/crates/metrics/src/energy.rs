use std::time::{Duration, Instant};

use crate::Measurement;

/// Integrates whole-device power samples using the trapezoidal rule.
///
/// A gap caused by an unavailable power sample resets the integration baseline;
/// energy is never guessed across an unknown interval.
#[derive(Clone, Debug, Default)]
pub struct EnergyAccumulator {
    total_joules: f64,
    previous: Option<PowerSample>,
}

#[derive(Clone, Copy, Debug)]
struct PowerSample {
    at: Instant,
    watts: f64,
}

impl EnergyAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn total_joules(&self) -> f64 {
        self.total_joules
    }

    pub fn total_watt_hours(&self) -> f64 {
        self.total_joules / 3_600.0
    }

    pub fn reset(&mut self) {
        self.total_joules = 0.0;
        self.previous = None;
    }

    pub fn observe(&mut self, power_watts: &Measurement<f64>) -> Measurement<f64> {
        self.observe_at(power_watts, Instant::now())
    }

    pub fn observe_at(&mut self, power_watts: &Measurement<f64>, at: Instant) -> Measurement<f64> {
        let Measurement::Available(watts) = power_watts else {
            self.previous = None;
            return Measurement::Unavailable {
                reason: "whole-device power was unavailable".to_owned(),
            };
        };
        if !watts.is_finite() || *watts < 0.0 {
            self.previous = None;
            return Measurement::Unavailable {
                reason: "whole-device power was invalid".to_owned(),
            };
        }

        if let Some(previous) = self.previous {
            let elapsed = at.saturating_duration_since(previous.at);
            self.total_joules += (previous.watts + *watts) * 0.5 * elapsed.as_secs_f64();
        }
        self.previous = Some(PowerSample { at, watts: *watts });
        Measurement::Available(self.total_joules)
    }

    /// Returns the energy added by a constant-power interval. This is useful
    /// for hosts that already own the sampling clock.
    pub fn energy_for_interval(power_watts: f64, elapsed: Duration) -> Measurement<f64> {
        if power_watts.is_finite() && power_watts >= 0.0 {
            Measurement::Available(power_watts * elapsed.as_secs_f64())
        } else {
            Measurement::Unavailable {
                reason: "whole-device power was invalid".to_owned(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrates_power_using_trapezoids() {
        let start = Instant::now();
        let mut accumulator = EnergyAccumulator::new();
        assert_eq!(
            accumulator.observe_at(&Measurement::Available(10.0), start),
            Measurement::Available(0.0)
        );
        let energy = accumulator.observe_at(
            &Measurement::Available(20.0),
            start + Duration::from_secs(2),
        );
        assert_eq!(energy, Measurement::Available(30.0));
        assert!((accumulator.total_watt_hours() - (30.0 / 3_600.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn does_not_integrate_across_unavailable_samples() {
        let start = Instant::now();
        let mut accumulator = EnergyAccumulator::new();
        accumulator.observe_at(&Measurement::Available(10.0), start);
        let unavailable = accumulator.observe_at(
            &Measurement::Unavailable {
                reason: "not supported".to_owned(),
            },
            start + Duration::from_secs(2),
        );
        assert!(matches!(unavailable, Measurement::Unavailable { .. }));
        let energy = accumulator.observe_at(
            &Measurement::Available(10.0),
            start + Duration::from_secs(4),
        );
        assert_eq!(energy, Measurement::Available(0.0));
    }
}
