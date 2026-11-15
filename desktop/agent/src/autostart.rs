//! Opt-in start-at-login registration for the per-user agent.
//!
//! * Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value. The
//!   agent runs in the interactive user session (never Session 0), which the
//!   Core Audio and input APIs require.
//! * Linux: systemd *user* unit (`~/.config/systemd/user/ccontrol-agent.service`).

use anyhow::Context;

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const VALUE: &str = "ccontrol-agent";

pub fn enable() -> anyhow::Result<String> {
    // Inside an AppImage, current_exe() points into a temporary mount; the
    // runtime exports the real bundle path as $APPIMAGE.
    let exe = match std::env::var_os("APPIMAGE") {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::current_exe().context("locating agent executable")?,
    };
    #[cfg(windows)]
    {
        let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(RUN_KEY)?;
        key.set_value(VALUE, &format!("\"{}\" run", exe.display()))?;
        Ok("registered in HKCU Run".into())
    }
    #[cfg(target_os = "linux")]
    {
        let dir = directories::BaseDirs::new()
            .context("no home directory")?
            .config_dir()
            .join("systemd/user");
        std::fs::create_dir_all(&dir)?;
        let unit = format!(
            "[Unit]\nDescription=ccontrol desktop agent\nAfter=graphical-session.target pipewire.service\nPartOf=graphical-session.target\n\n\
             [Service]\nExecStart={} run\nRestart=on-failure\nRestartSec=3\n\n[Install]\nWantedBy=graphical-session.target default.target\n",
            exe.display()
        );
        std::fs::write(dir.join("ccontrol-agent.service"), unit)?;
        let st = std::process::Command::new("systemctl")
            .args(["--user", "enable", "--now", "ccontrol-agent.service"])
            .status();
        Ok(match st {
            Ok(s) if s.success() => "systemd user unit enabled".into(),
            _ => "unit written; run `systemctl --user enable --now ccontrol-agent` manually".into(),
        })
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = exe;
        anyhow::bail!("autostart not supported on this OS")
    }
}

pub fn disable() -> anyhow::Result<String> {
    #[cfg(windows)]
    {
        let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_SET_VALUE) {
            let _ = key.delete_value(VALUE);
        }
        Ok("removed from HKCU Run".into())
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "--now", "ccontrol-agent.service"])
            .status();
        if let Some(b) = directories::BaseDirs::new() {
            let _ =
                std::fs::remove_file(b.config_dir().join("systemd/user/ccontrol-agent.service"));
        }
        Ok("systemd user unit removed".into())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    anyhow::bail!("autostart not supported on this OS")
}
