use ccontrol_protocol::Telemetry;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

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
        Telemetry {
            cpu_percent: sys.global_cpu_usage(),
            cpu_count: sys.cpus().len() as u32,
            memory_used_bytes: sys.used_memory(),
            memory_total_bytes: sys.total_memory(),
            uptime_secs: System::uptime(),
            battery_percent: None,
        }
    }
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
    }
}
