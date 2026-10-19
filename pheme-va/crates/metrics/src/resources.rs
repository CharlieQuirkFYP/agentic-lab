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
/// temperature is the hottest finite, currently reported component sensor, not
/// an ambient or whole-device temperature. Linux additionally reports the peak
/// utilization among readable DRM cards (not process usage) and signed battery
/// percentage-point decrease since the first valid sample of a single battery.
/// Whole-device power/energy require a provider with a verified measurement boundary.
#[cfg(feature = "desktop")]
pub struct SysinfoResourceSampler {
    system: sysinfo::System,
    pid: sysinfo::Pid,
    #[cfg(target_os = "linux")]
    sensors: crate::linux_sensors::LinuxSensors,
}

#[cfg(feature = "desktop")]
impl SysinfoResourceSampler {
    pub fn new() -> Self {
        let mut system = sysinfo::System::new_all();
        system.refresh_all();
        Self {
            system,
            pid: sysinfo::Pid::from_u32(process::id()),
            #[cfg(target_os = "linux")]
            sensors: crate::linux_sensors::LinuxSensors::new("/sys"),
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
        #[cfg(target_os = "linux")]
        {
            "sysinfo+linux-sysfs"
        }
        #[cfg(not(target_os = "linux"))]
        {
            "sysinfo"
        }
    }

    fn capabilities(&self) -> ResourceCapabilities {
        ResourceCapabilities {
            process_cpu_percent: true,
            system_cpu_percent: true,
            process_ram_bytes: true,
            system_ram_bytes: true,
            temperature_celsius: desktop_temperature().is_available(),
            #[cfg(target_os = "linux")]
            gpu_usage_percent: self.sensors.gpu_usage().is_available(),
            #[cfg(target_os = "linux")]
            battery_drain_percent: self.sensors.has_battery_capacity(),
            ..ResourceCapabilities::default()
        }
    }

    fn sample(&mut self) -> ResourceSnapshot {
        self.system.refresh_all();
        let process = self.system.process(self.pid);
        let unsupported = "no verified whole-device power meter; CPU/GPU component power and battery-terminal power are not whole-device measurements";
        #[cfg(target_os = "linux")]
        let (gpu_usage_percent, battery_drain_percent) =
            (self.sensors.gpu_usage(), self.sensors.battery_drain());
        #[cfg(not(target_os = "linux"))]
        let (gpu_usage_percent, battery_drain_percent) = (
            Measurement::unavailable("GPU utilization provider not implemented for this platform"),
            Measurement::unavailable("battery capacity provider not implemented for this platform"),
        );
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
            gpu_usage_percent,
            temperature_celsius: desktop_temperature(),
            battery_drain_percent,
            whole_device_power_watts: Measurement::unavailable(unsupported),
            energy_joules: Measurement::unavailable(unsupported),
            energy_watt_hours: Measurement::unavailable(unsupported),
        }
    }
}

#[cfg(feature = "desktop")]
fn desktop_temperature() -> Measurement<f64> {
    // Rediscover each sample so removed sensors cannot retain stale readings.
    let components = sysinfo::Components::new_with_refreshed_list();
    hottest_temperature(
        components
            .iter()
            .map(|component| f64::from(component.temperature())),
    )
}

#[cfg(any(feature = "desktop", test))]
fn hottest_temperature(values: impl Iterator<Item = f64>) -> Measurement<f64> {
    values
        .filter(|value| value.is_finite())
        .reduce(f64::max)
        .map(Measurement::available)
        .unwrap_or_else(|| Measurement::unavailable("no finite component temperature reported by sysinfo; sensors may be absent, inaccessible, or unsupported"))
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
    fn temperature_selects_hottest_finite_sensor() {
        assert_eq!(
            hottest_temperature([25.0, f64::NAN, 61.0, f64::INFINITY].into_iter()),
            Measurement::available(61.0)
        );
        assert_eq!(
            hottest_temperature([-5.0, 0.0].into_iter()),
            Measurement::available(0.0)
        );
        assert!(!hottest_temperature(std::iter::empty()).is_available());
        assert!(!hottest_temperature([f64::NAN].into_iter()).is_available());
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
