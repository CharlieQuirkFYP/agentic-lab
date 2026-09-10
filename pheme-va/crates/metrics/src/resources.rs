#[cfg(feature = "desktop")]
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::energy::EnergyAccumulator;
use crate::MetricsContext;

#[derive(Clone, Debug, PartialEq)]
pub enum Measurement<T> {
    Available(T),
    Unavailable { reason: String },
}

impl<T> Measurement<T> {
    pub fn available(value: T) -> Self {
        Self::Available(value)
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }

    pub fn as_ref(&self) -> Measurement<&T> {
        match self {
            Self::Available(value) => Measurement::Available(value),
            Self::Unavailable { reason } => Measurement::Unavailable {
                reason: reason.clone(),
            },
        }
    }
}

/// One platform resource sample. Values that the host cannot measure are
/// represented explicitly instead of being converted to zero.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceSnapshot {
    pub sampled_at_ms: u64,
    pub source: String,
    pub process_cpu_percent: Measurement<f64>,
    pub system_cpu_percent: Measurement<f64>,
    pub process_ram_bytes: Measurement<f64>,
    pub system_ram_bytes: Measurement<f64>,
    pub gpu_usage_percent: Measurement<f64>,
    pub temperature_celsius: Measurement<f64>,
    /// Cumulative battery percentage-point change, as defined by the provider.
    pub battery_drain_percent: Measurement<f64>,
    pub whole_device_power_watts: Measurement<f64>,
    pub energy_joules: Measurement<f64>,
    pub energy_watt_hours: Measurement<f64>,
}

impl ResourceSnapshot {
    pub fn unavailable(source: impl Into<String>, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            sampled_at_ms: epoch_millis(),
            source: source.into(),
            process_cpu_percent: Measurement::unavailable(reason.clone()),
            system_cpu_percent: Measurement::unavailable(reason.clone()),
            process_ram_bytes: Measurement::unavailable(reason.clone()),
            system_ram_bytes: Measurement::unavailable(reason.clone()),
            gpu_usage_percent: Measurement::unavailable(reason.clone()),
            temperature_celsius: Measurement::unavailable(reason.clone()),
            battery_drain_percent: Measurement::unavailable(reason.clone()),
            whole_device_power_watts: Measurement::unavailable(reason.clone()),
            energy_joules: Measurement::unavailable(reason.clone()),
            energy_watt_hours: Measurement::unavailable(reason),
        }
    }
}

/// Capabilities advertised by a resource provider. A capability describes the
/// provider's intended support; individual samples can still be unavailable.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResourceCapabilities {
    pub process_cpu_percent: bool,
    pub system_cpu_percent: bool,
    pub process_ram_bytes: bool,
    pub system_ram_bytes: bool,
    pub gpu_usage_percent: bool,
    pub temperature_celsius: bool,
    pub battery_drain_percent: bool,
    pub whole_device_power_watts: bool,
    pub energy_joules: bool,
    pub energy_watt_hours: bool,
}

/// Host-provided resource sampler. Native iOS and Android code can implement
/// this trait without changing the metrics event schema.
pub trait ResourceSampler: Send {
    fn source(&self) -> &str;
    fn sample(&mut self) -> ResourceSnapshot;

    fn capabilities(&self) -> ResourceCapabilities {
        ResourceCapabilities::default()
    }
}

/// Adds energy estimates to resource snapshots when a provider exposes whole-
/// device power but not an energy counter.
pub struct ResourceCollector<S> {
    sampler: S,
    energy: EnergyAccumulator,
}

impl<S> ResourceCollector<S>
where
    S: ResourceSampler,
{
    pub fn new(sampler: S) -> Self {
        Self {
            sampler,
            energy: EnergyAccumulator::new(),
        }
    }

    pub fn sampler(&self) -> &S {
        &self.sampler
    }

    pub fn sampler_mut(&mut self) -> &mut S {
        &mut self.sampler
    }

    pub fn energy(&self) -> &EnergyAccumulator {
        &self.energy
    }

    pub fn sample_and_record(&mut self, metrics: &MetricsContext) -> ResourceSnapshot {
        if !metrics.resource_sampling_enabled() {
            return ResourceSnapshot::unavailable(
                self.sampler.source(),
                "resource sampling disabled for this run",
            );
        }

        let mut snapshot = self.sampler.sample();
        let estimated_joules = self.energy.observe(&snapshot.whole_device_power_watts);
        if !snapshot.energy_joules.is_available() {
            snapshot.energy_joules = estimated_joules.clone();
        }
        if !snapshot.energy_watt_hours.is_available() {
            snapshot.energy_watt_hours = match estimated_joules {
                Measurement::Available(joules) => Measurement::Available(joules / 3_600.0),
                Measurement::Unavailable { reason } => Measurement::Unavailable { reason },
            };
        }
        metrics.record_resource_snapshot(&snapshot);
        snapshot
    }
}

/// Desktop-first provider using `sysinfo` on Linux, macOS, and Windows. The
/// provider intentionally reports only process/system CPU and RAM today. GPU,
/// temperature, battery, whole-device power, and energy require host-specific
/// providers and are returned as unavailable measurements.
#[cfg(feature = "desktop")]
pub struct SysinfoResourceSampler {
    system: sysinfo::System,
    pid: sysinfo::Pid,
}

#[cfg(feature = "desktop")]
impl SysinfoResourceSampler {
    pub fn new() -> Self {
        let mut system = sysinfo::System::new_all();
        system.refresh_all();
        Self {
            system,
            pid: sysinfo::Pid::from_u32(process::id()),
        }
    }

    pub fn with_pid(pid: u32) -> Self {
        let mut sampler = Self::new();
        sampler.pid = sysinfo::Pid::from_u32(pid);
        sampler
    }
}

#[cfg(feature = "desktop")]
impl Default for SysinfoResourceSampler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "desktop")]
impl ResourceSampler for SysinfoResourceSampler {
    fn source(&self) -> &str {
        "sysinfo"
    }

    fn capabilities(&self) -> ResourceCapabilities {
        ResourceCapabilities {
            process_cpu_percent: true,
            system_cpu_percent: true,
            process_ram_bytes: true,
            system_ram_bytes: true,
            ..ResourceCapabilities::default()
        }
    }

    fn sample(&mut self) -> ResourceSnapshot {
        self.system.refresh_all();
        let process = self.system.process(self.pid);
        let unsupported = "not provided by the desktop sampler";
        ResourceSnapshot {
            sampled_at_ms: epoch_millis(),
            source: self.source().to_owned(),
            process_cpu_percent: process
                .map(|process| Measurement::available(process.cpu_usage() as f64))
                .unwrap_or_else(|| Measurement::unavailable("current process was not found")),
            system_cpu_percent: Measurement::available(
                self.system.global_cpu_info().cpu_usage() as f64
            ),
            process_ram_bytes: process
                .map(|process| Measurement::available(process.memory() as f64))
                .unwrap_or_else(|| Measurement::unavailable("current process was not found")),
            system_ram_bytes: Measurement::available(self.system.used_memory() as f64),
            gpu_usage_percent: Measurement::unavailable(unsupported),
            temperature_celsius: Measurement::unavailable(unsupported),
            battery_drain_percent: Measurement::unavailable(unsupported),
            whole_device_power_watts: Measurement::unavailable(unsupported),
            energy_joules: Measurement::unavailable(unsupported),
            energy_watt_hours: Measurement::unavailable(unsupported),
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
    use crate::{MetricsConfig, MetricsHub};

    struct FakeSampler {
        snapshots: Vec<ResourceSnapshot>,
    }

    impl ResourceSampler for FakeSampler {
        fn source(&self) -> &str {
            "fake"
        }

        fn sample(&mut self) -> ResourceSnapshot {
            self.snapshots.remove(0)
        }
    }

    fn snapshot(power: Measurement<f64>) -> ResourceSnapshot {
        let mut snapshot = ResourceSnapshot::unavailable("fake", "not measured");
        snapshot.whole_device_power_watts = power;
        snapshot
    }

    #[test]
    fn collector_integrates_power_and_records_energy() {
        let hub = Arc::new(MetricsHub::new());
        let metrics = MetricsContext::new(
            "run-1",
            Some("exp-1".to_owned()),
            MetricsConfig {
                enabled: true,
                incident_active: false,
                resource_sampling: true,
            },
            hub,
        );
        let sampler = FakeSampler {
            snapshots: vec![snapshot(Measurement::Available(10.0))],
        };
        let mut collector = ResourceCollector::new(sampler);
        let result = collector.sample_and_record(&metrics);
        assert_eq!(result.energy_joules, Measurement::Available(0.0));
    }
}
