//! Optional Linux sysfs readings. Never interpret component or battery-terminal
//! power as whole-device power. No driver libraries or subprocesses are needed.
use std::fs;
use std::path::{Path, PathBuf};

use crate::Measurement;

pub(crate) struct LinuxSensors {
    root: PathBuf,
    battery_baseline: Option<(PathBuf, f64)>,
}

impl LinuxSensors {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            battery_baseline: None,
        }
    }

    pub(crate) fn gpu_usage(&self) -> Measurement<f64> {
        let cards = match directories(&self.root.join("class/drm")) {
            Ok(cards) => cards,
            Err(reason) => return Measurement::unavailable(reason),
        };
        let mut peak: Option<f64> = None;
        for card in cards {
            let name = card
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if !name.strip_prefix("card").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            }) {
                continue;
            }
            if let Ok(value) = percentage(&card.join("device/gpu_busy_percent")) {
                peak = Some(peak.map_or(value, |previous| previous.max(value)));
            }
        }
        peak.map(Measurement::available).unwrap_or_else(|| {
            Measurement::unavailable(
                "no readable valid DRM gpu_busy_percent sensor; driver may not expose utilization (not process GPU usage)",
            )
        })
    }

    /// Signed percentage-point decrease since the first valid sample from one
    /// system battery. Charging can produce negative values. A missing/invalid
    /// sample resets the baseline rather than bridging unknown battery changes.
    pub(crate) fn battery_drain(&mut self) -> Measurement<f64> {
        match self.battery_capacity() {
            Ok((path, capacity)) => {
                let (_, baseline) = self
                    .battery_baseline
                    .get_or_insert_with(|| (path.clone(), capacity));
                let value = *baseline - capacity;
                if self
                    .battery_baseline
                    .as_ref()
                    .is_some_and(|(old, _)| *old != path)
                {
                    self.battery_baseline = Some((path, capacity));
                    Measurement::available(0.0)
                } else {
                    Measurement::available(value)
                }
            }
            Err(reason) => {
                self.battery_baseline = None;
                Measurement::unavailable(reason)
            }
        }
    }

    pub(crate) fn has_battery_capacity(&self) -> bool {
        self.battery_capacity().is_ok()
    }

    fn battery_capacity(&self) -> Result<(PathBuf, f64), String> {
        let supplies = directories(&self.root.join("class/power_supply"))?;
        let batteries: Vec<_> = supplies
            .into_iter()
            .filter(|path| {
                text(&path.join("type")).is_ok_and(|value| value == "Battery")
                    && !text(&path.join("scope")).is_ok_and(|value| value == "Device")
                    && !text(&path.join("present")).is_ok_and(|value| value == "0")
            })
            .collect();
        if batteries.len() != 1 {
            return Err("battery drain requires exactly one present system battery; absent or multiple batteries are not aggregated".into());
        }
        let path = batteries.into_iter().next().unwrap();
        let capacity = percentage(&path.join("capacity"))?;
        Ok((path, capacity))
    }
}

fn directories(path: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(path).map_err(|error| format!("{}: {error}", path.display()))?;
    entries
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| format!("{}: {error}", path.display()))
        })
        .collect()
}

fn text(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn percentage(path: &Path) -> Result<f64, String> {
    let value = text(path)?.parse::<f64>().ok();
    match value {
        Some(value) if value.is_finite() && (0.0..=100.0).contains(&value) => Ok(value),
        _ => Err(format!(
            "{}: invalid percentage (expected 0..=100)",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "pheme-metrics-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write(&self, path: &str, value: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, value).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn gpu_uses_peak_valid_card_not_connectors_or_render_aliases() {
        let fixture = Fixture::new();
        let sensors = LinuxSensors::new(&fixture.0);
        assert!(!sensors.gpu_usage().is_available());
        fixture.write("class/drm/card0/device/gpu_busy_percent", "12\n");
        fixture.write("class/drm/card1/device/gpu_busy_percent", "42");
        fixture.write("class/drm/card1-DP-1/device/gpu_busy_percent", "99");
        fixture.write("class/drm/renderD128/device/gpu_busy_percent", "100");
        fixture.write("class/drm/card2/device/gpu_busy_percent", "NaN");
        assert_eq!(sensors.gpu_usage(), Measurement::available(42.0));
        fixture.write("class/drm/card1/device/gpu_busy_percent", "101");
        fixture.write("class/drm/card0/device/gpu_busy_percent", "0");
        assert_eq!(sensors.gpu_usage(), Measurement::available(0.0));
        fixture.write("class/drm/card0/device/gpu_busy_percent", "-1");
        assert!(!sensors.gpu_usage().is_available());
    }

    #[test]
    fn battery_tracks_signed_drain_and_resets_after_gaps() {
        let fixture = Fixture::new();
        let mut sensors = LinuxSensors::new(&fixture.0);
        assert!(!sensors.battery_drain().is_available());
        fixture.write("class/power_supply/BAT0/type", "Battery\n");
        fixture.write("class/power_supply/BAT0/capacity", "60");
        assert!(sensors.has_battery_capacity());
        assert_eq!(sensors.battery_drain(), Measurement::available(0.0));
        fixture.write("class/power_supply/BAT0/capacity", "58");
        assert_eq!(sensors.battery_drain(), Measurement::available(2.0));
        fixture.write("class/power_supply/BAT0/capacity", "62");
        assert_eq!(sensors.battery_drain(), Measurement::available(-2.0));
        fixture.write("class/power_supply/BAT0/capacity", "NaN");
        assert!(!sensors.battery_drain().is_available());
        fixture.write("class/power_supply/BAT0/capacity", "59");
        assert_eq!(sensors.battery_drain(), Measurement::available(0.0));
        fixture.write("class/power_supply/BAT1/type", "Battery");
        fixture.write("class/power_supply/BAT1/capacity", "50");
        assert!(!sensors.battery_drain().is_available());
        fixture.write("class/power_supply/BAT1/scope", "Device");
        assert_eq!(sensors.battery_drain(), Measurement::available(0.0));
        fixture.write("class/power_supply/BAT0/present", "0");
        assert!(!sensors.battery_drain().is_available());
    }
}
