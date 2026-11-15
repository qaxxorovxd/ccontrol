//! Process listing and termination (cross-platform, via `sysinfo`).
//!
//! Processes are *not* applications: ending a PID is a last resort. The
//! graceful path asks the program to quit (WM_CLOSE to its windows on
//! Windows, SIGTERM on Linux); force-kill is a separate, explicitly confirmed
//! request. Critical system processes, other users' processes and the agent
//! itself are never terminated.

use crate::ControlError;
use ccontrol_protocol::{ProcessInfo, TerminateMethod, TerminateOutcome};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

const GRACEFUL_WAIT: Duration = Duration::from_secs(5);
const FORCE_WAIT: Duration = Duration::from_secs(3);

#[cfg(windows)]
const CRITICAL: &[&str] = &[
    "system",
    "registry",
    "secure system",
    "memory compression",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "lsaiso.exe",
    "svchost.exe",
    "fontdrvhost.exe",
    "dwm.exe",
    "sihost.exe",
    "msmpeng.exe",
    "audiodg.exe",
];
#[cfg(not(windows))]
const CRITICAL: &[&str] = &[
    "systemd",
    "init",
    "kthreadd",
    "dbus-daemon",
    "dbus-broker",
    "xorg",
    "xwayland",
    "gnome-shell",
    "kwin_wayland",
    "kwin_x11",
    "plasmashell",
    "pipewire",
    "wireplumber",
    "pipewire-pulse",
    "pulseaudio",
    "gdm",
    "sddm",
    "lightdm",
    "polkitd",
    "networkmanager",
];

pub struct ProcessManager {
    sys: Mutex<(System, Option<Instant>)>,
    self_pid: Pid,
    cpu_count: f32,
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

fn refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_user(UpdateKind::OnlyIfNotSet)
}

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            sys: Mutex::new((System::new(), None)),
            self_pid: Pid::from_u32(std::process::id()),
            cpu_count: std::thread::available_parallelism()
                .map(|n| n.get() as f32)
                .unwrap_or(1.0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (System, Option<Instant>)> {
        self.sys.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Why a process may not be ended, or `None` if it may.
    fn protection(&self, sys: &System, p: &sysinfo::Process) -> Option<String> {
        if p.pid() == self.self_pid {
            return Some("This is the ccontrol agent".into());
        }
        if p.pid().as_u32() <= 4 {
            return Some("Core operating system process".into());
        }
        let name = p.name().to_string_lossy().to_lowercase();
        if CRITICAL.contains(&name.as_str()) {
            return Some("Critical system process".into());
        }
        if !self.same_user(sys, p) {
            return Some("Runs as another user or as the system".into());
        }
        None
    }

    fn same_user(&self, sys: &System, p: &sysinfo::Process) -> bool {
        let me = sys.process(self.self_pid).and_then(|s| s.user_id());
        matches!((me, p.user_id()), (Some(a), Some(b)) if a == b)
    }

    pub fn list(&self) -> Vec<ProcessInfo> {
        let mut g = self.lock();
        let (sys, last) = &mut *g;
        let first = last
            .map(|t| t.elapsed() > Duration::from_secs(30))
            .unwrap_or(true);
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind());
        if first {
            // CPU usage needs two samples.
            std::thread::sleep(
                sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(250)),
            );
            sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind());
        }
        *last = Some(Instant::now());
        let mut out: Vec<ProcessInfo> = sys
            .processes()
            .values()
            // Linux threads show up as tasks of their process; skip them.
            .filter(|p| p.thread_kind().is_none())
            .map(|p| {
                let protected_reason = self.protection(sys, p);
                ProcessInfo {
                    pid: p.pid().as_u32(),
                    parent_pid: p.parent().map(|x| x.as_u32()),
                    name: p.name().to_string_lossy().into_owned(),
                    exe: p.exe().map(|e| e.display().to_string()),
                    cpu_percent: (p.cpu_usage() / self.cpu_count).clamp(0.0, 100.0),
                    memory_bytes: p.memory(),
                    status: p.status().to_string(),
                    start_time: p.start_time(),
                    same_user: self.same_user(sys, p),
                    terminable: protected_reason.is_none(),
                    protected_reason,
                }
            })
            .collect();
        out.sort_by(|a, b| {
            b.cpu_percent
                .total_cmp(&a.cpu_percent)
                .then(b.memory_bytes.cmp(&a.memory_bytes))
        });
        out
    }

    fn is_running(&self, pid: Pid) -> bool {
        let mut g = self.lock();
        g.0.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        // A terminated child stays a zombie until its parent reaps it; it has
        // exited all the same.
        g.0.process(pid)
            .map(|p| !matches!(p.status(), ProcessStatus::Zombie | ProcessStatus::Dead))
            .unwrap_or(false)
    }

    fn wait_exit(&self, pid: Pid, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if !self.is_running(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        !self.is_running(pid)
    }

    /// End a process. `name` must match the live process (PID reuse guard).
    pub fn terminate(
        &self,
        pid: u32,
        name: &str,
        force: bool,
        // How to ask the process to quit politely; returns how many quit requests
        // were delivered (0 = no graceful path, e.g. a windowless process).
        graceful: &dyn Fn(u32) -> Result<usize, ControlError>,
    ) -> Result<TerminateOutcome, ControlError> {
        let spid = Pid::from_u32(pid);
        {
            let mut g = self.lock();
            let (sys, _) = &mut *g;
            sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[spid]), true, refresh_kind());
            // The agent's own entry is needed for the same-user comparison.
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[self.self_pid]),
                false,
                refresh_kind(),
            );
            let p = sys.process(spid).ok_or_else(|| {
                ControlError::Invalid(format!("process {pid} is no longer running"))
            })?;
            if !p.name().to_string_lossy().eq_ignore_ascii_case(name) {
                return Err(ControlError::Invalid(format!(
                    "process {pid} is now '{}', not '{name}'; refresh the list",
                    p.name().to_string_lossy()
                )));
            }
            if let Some(reason) = self.protection(sys, p) {
                return Err(ControlError::Forbidden(reason));
            }
            if force && !p.kill() {
                return Err(ControlError::Platform(format!(
                    "could not end process {pid}"
                )));
            }
        }
        if force {
            let exited = self.wait_exit(spid, FORCE_WAIT);
            return Ok(TerminateOutcome {
                pid,
                method: TerminateMethod::Force,
                exited,
                note: None,
            });
        }
        let delivered = graceful(pid)?;
        if delivered == 0 {
            return Err(ControlError::Unsupported(
                "This process has no window to close. Use Force quit to end it.".into(),
            ));
        }
        let exited = self.wait_exit(spid, GRACEFUL_WAIT);
        Ok(TerminateOutcome {
            pid,
            method: TerminateMethod::Graceful,
            exited,
            note: (!exited).then(|| {
                "Still running. It may be asking to save changes on the computer, or it ignored the request.".into()
            }),
        })
    }
}

/// Linux graceful stop: SIGTERM.
#[cfg(not(windows))]
pub fn sigterm(pid: u32) -> Result<usize, ControlError> {
    let mut sys = System::new();
    let spid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[spid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    match sys
        .process(spid)
        .and_then(|p| p.kill_with(sysinfo::Signal::Term))
    {
        Some(true) => Ok(1),
        Some(false) => Err(ControlError::Platform("SIGTERM was not delivered".into())),
        None => Err(ControlError::Unsupported("SIGTERM unsupported".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_self_as_protected() {
        let pm = ProcessManager::new();
        let list = pm.list();
        let me = list
            .iter()
            .find(|p| p.pid == std::process::id())
            .expect("own process listed");
        assert!(!me.terminable);
        assert_eq!(
            me.protected_reason.as_deref(),
            Some("This is the ccontrol agent")
        );
        assert!(me.same_user);
    }

    #[test]
    fn refuses_to_kill_self_and_mismatched_names() {
        let pm = ProcessManager::new();
        let me = pm
            .list()
            .into_iter()
            .find(|p| p.pid == std::process::id())
            .unwrap();
        let none = |_: u32| Ok(0usize);
        assert!(matches!(
            pm.terminate(me.pid, &me.name, true, &none),
            Err(ControlError::Forbidden(_))
        ));
        assert!(matches!(
            pm.terminate(me.pid, "other.exe", true, &none),
            Err(ControlError::Invalid(_))
        ));
        assert!(matches!(
            pm.terminate(u32::MAX - 7, "x", true, &none),
            Err(ControlError::Invalid(_))
        ));
    }

    #[test]
    fn force_kills_owned_child() {
        #[cfg(windows)]
        let mut child = std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        #[cfg(not(windows))]
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pm = ProcessManager::new();
        let info = pm
            .list()
            .into_iter()
            .find(|p| p.pid == child.id())
            .expect("child listed");
        assert!(info.terminable, "{:?}", info.protected_reason);
        let none = |_: u32| Ok(0usize);
        // Windowless console process: graceful path is reported as unavailable.
        #[cfg(windows)]
        assert!(matches!(
            pm.terminate(info.pid, &info.name, false, &none),
            Err(ControlError::Unsupported(_))
        ));
        let out = pm.terminate(info.pid, &info.name, true, &none).unwrap();
        assert!(out.exited);
        let _ = child.wait();
    }
}
