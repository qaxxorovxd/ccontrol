//! Top-level application windows (what Alt+Tab shows), via EnumWindows.

use crate::{ControlError, WindowControl};
use ccontrol_protocol::AppWindow;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, PostMessageW,
    GWL_EXSTYLE, GW_OWNER, WM_CLOSE, WS_EX_TOOLWINDOW,
};

pub struct Win32Windows;

struct Raw {
    hwnd: isize,
    pid: u32,
    title: String,
    minimized: bool,
}

fn pid_of(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

/// Alt+Tab-like filter: visible, titled, unowned, not a tool window, not cloaked.
fn is_app_window(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || GetWindowTextLengthW(hwnd) == 0 {
            return false;
        }
        if GetWindow(hwnd, GW_OWNER)
            .map(|o| !o.is_invalid())
            .unwrap_or(false)
        {
            return false;
        }
        if (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32) & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            std::mem::size_of::<u32>() as u32,
        );
        cloaked == 0
    }
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = &mut *(lparam.0 as *mut Vec<Raw>);
    if is_app_window(hwnd) {
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut buf);
        out.push(Raw {
            hwnd: hwnd.0 as isize,
            pid: pid_of(hwnd),
            title: String::from_utf16_lossy(&buf[..n.max(0) as usize]),
            minimized: IsIconic(hwnd).as_bool(),
        });
    }
    true.into()
}

fn enumerate() -> Result<Vec<Raw>, ControlError> {
    let mut v: Vec<Raw> = Vec::new();
    unsafe { EnumWindows(Some(collect), LPARAM(&mut v as *mut _ as isize)) }
        .map_err(|e| ControlError::Platform(format!("EnumWindows: {e}")))?;
    Ok(v)
}

impl WindowControl for Win32Windows {
    fn list(&self) -> Result<Vec<AppWindow>, ControlError> {
        let raw = enumerate()?;
        let fg = unsafe { GetForegroundWindow() }.0 as isize;
        let mut sys = System::new();
        let pids: Vec<Pid> = raw.iter().map(|r| Pid::from_u32(r.pid)).collect();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing(),
        );
        Ok(raw
            .into_iter()
            .map(|r| AppWindow {
                window_id: r.hwnd as u64,
                pid: r.pid,
                process_name: sys
                    .process(Pid::from_u32(r.pid))
                    .map(|p| p.name().to_string_lossy().into_owned())
                    .unwrap_or_else(|| "?".into()),
                title: r.title,
                focused: r.hwnd == fg,
                minimized: r.minimized,
            })
            .collect())
    }

    fn close(&self, window_id: u64, pid: u32) -> Result<(), ControlError> {
        let hwnd = HWND(window_id as isize as *mut core::ffi::c_void);
        // Window handles are reused; make sure it is still the same app's window.
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() || pid_of(hwnd) != pid || !is_app_window(hwnd)
        {
            return Err(ControlError::Invalid(
                "that window no longer exists; refresh the list".into(),
            ));
        }
        if pid == std::process::id() {
            return Err(ControlError::Forbidden("This is the ccontrol agent".into()));
        }
        unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }
            .map_err(|e| ControlError::Platform(format!("PostMessage: {e}")))
    }

    fn close_all_of(&self, pid: u32) -> Result<usize, ControlError> {
        let mut n = 0;
        for r in enumerate()?.into_iter().filter(|r| r.pid == pid) {
            let hwnd = HWND(r.hwnd as *mut core::ffi::c_void);
            if unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }.is_ok() {
                n += 1;
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerates_without_error() {
        // On a headless CI runner the list may be empty; it must not fail.
        let list = Win32Windows.list().unwrap();
        for w in &list {
            assert!(!w.title.is_empty());
            assert!(w.pid != 0);
        }
    }

    #[test]
    fn rejects_stale_handles() {
        assert!(matches!(
            Win32Windows.close(0xdead_beef, 1),
            Err(ControlError::Invalid(_))
        ));
    }
}
