use ccontrol_protocol::DiskInfo;
use ccontrol_protocol::Telemetry;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, RefreshKind, System};

/// On-demand telemetry sampler. Nothing is polled in the background: CPU
/// usage is computed from the delta since the previous request (or a short
/// 200 ms sample window on the first request).
pub struct TelemetrySampler {
    inner: Mutex<(System, Option<Instant>)>,
}

impl Default for TelemetrySampler {
    fn default() -> Self {
        Self::new()
    }
}

impl TelemetrySampler {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        Self {
            inner: Mutex::new((sys, None)),
        }
    }

    pub fn sample(&self) -> Telemetry {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let (sys, last) = &mut *g;
        let stale = last
            .map(|t| t.elapsed() > Duration::from_secs(30))
            .unwrap_or(true);
        if stale {
            sys.refresh_cpu_usage();
            std::thread::sleep(
                sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(200)),
            );
        }
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        *last = Some(Instant::now());
        let battery = battery();
        Telemetry {
            cpu_percent: sys.global_cpu_usage(),
            cpu_count: sys.cpus().len() as u32,
            memory_used_bytes: sys.used_memory(),
            memory_total_bytes: sys.total_memory(),
            uptime_secs: System::uptime(),
            battery_percent: battery.map(|b| b.0),
            battery_charging: battery.and_then(|b| b.1),
            disks: disks(),
        }
    }
}

fn disks() -> Vec<DiskInfo> {
    let mut seen = std::collections::HashSet::new();
    Disks::new_with_refreshed_list()
        .list()
        .iter()
        .filter(|d| d.total_space() > 0)
        // Skip pseudo/overlay file systems that duplicate real disks.
        .filter(|d| {
            let fs = d.file_system().to_string_lossy().to_lowercase();
            !matches!(
                fs.as_str(),
                "tmpfs" | "devtmpfs" | "overlay" | "squashfs" | "efivarfs" | "ramfs"
            )
        })
        .filter(|d| {
            seen.insert(d.name().to_string_lossy().into_owned() + &d.total_space().to_string())
        })
        .map(|d| DiskInfo {
            mount: d.mount_point().display().to_string(),
            name: d.name().to_string_lossy().into_owned(),
            total_bytes: d.total_space(),
            available_bytes: d.available_space(),
            removable: d.is_removable(),
        })
        .take(16)
        .collect()
}

/// (percent, charging) when the machine has a battery.
#[cfg(windows)]
fn battery() -> Option<(f32, Option<bool>)> {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut st = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut st) }.ok()?;
    // 128 = no system battery, 255 = unknown.
    if st.BatteryFlag & 128 != 0 || st.BatteryLifePercent == 255 {
        return None;
    }
    let charging = match st.ACLineStatus {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    };
    Some((st.BatteryLifePercent as f32, charging))
}

#[cfg(target_os = "linux")]
fn battery() -> Option<(f32, Option<bool>)> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for e in dir.flatten() {
        let p = e.path();
        if std::fs::read_to_string(p.join("type")).ok()?.trim() != "Battery" {
            continue;
        }
        let pct: f32 = std::fs::read_to_string(p.join("capacity"))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        let status = std::fs::read_to_string(p.join("status")).unwrap_or_default();
        let charging = match status.trim() {
            "Charging" | "Full" => Some(true),
            "Discharging" => Some(false),
            _ => None,
        };
        return Some((pct, charging));
    }
    None
}

#[cfg(not(any(windows, target_os = "linux")))]
fn battery() -> Option<(f32, Option<bool>)> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn sample_is_sane() {
        let t = super::TelemetrySampler::new().sample();
        assert!(t.memory_total_bytes > 0);
        assert!(t.memory_used_bytes <= t.memory_total_bytes);
        assert!(t.cpu_count > 0);
        assert!((0.0..=100.0).contains(&t.cpu_percent));
        assert!(!t.disks.is_empty(), "expected at least one disk");
        for d in &t.disks {
            assert!(d.available_bytes <= d.total_bytes);
        }
    }
}
