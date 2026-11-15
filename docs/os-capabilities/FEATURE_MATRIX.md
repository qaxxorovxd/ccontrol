# Feature matrix

Legend: **✅ verified** = implemented and exercised on real hardware/OS in this repo's
test runs · **🟡 implemented, unverified** = code exists and type-checks/unit-tests but has
not run on that platform yet · **📐 designed** = documented API path, no code yet ·
**⛔ not possible** = blocked by platform policy for a normal app.

Last updated: 2026-11-15 (Phase 1). Test machine: Windows 10 Pro 22H2 (build 19045), x86_64.

## Platform capability analysis (Phase 0)

The capabilities below are deliberately kept separate. They are **not** interchangeable.

| Capability | Windows 10 (≤19045) | Windows 10 21H2 Server/11 (≥20348) | Linux PipeWire | Linux PulseAudio-only | Android (normal app) |
|---|---|---|---|---|---|
| **Capture one app's audio** | ⛔ no documented per-process loopback; only whole-endpoint loopback | 📐 `ActivateAudioInterfaceAsync` + `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` (include/exclude process tree) | 📐 `pw_stream` capture targeting the app's output node (`target.object`) | 📐 monitor + `move-sink-input` to a null sink (coarser) | ⛔ for arbitrary apps. 📐 `AudioPlaybackCapture` (API 29+) only for apps that allow capture (usage MEDIA/GAME/UNKNOWN, `allowAudioPlaybackCapture`), after MediaProjection consent |
| **Silence/reroute one app locally** | ⛔ no documented per-app endpoint routing API (Settings "App volume and device preferences" uses undocumented `IAudioPolicyConfigFactory`) — only per-session volume/mute via `ISimpleAudioVolume` | same; mute the session via `ISimpleAudioVolume` while capturing with process loopback | 📐 re-link the stream's node to a null sink via the session manager (WirePlumber metadata `target.object`) | 📐 `move-sink-input` | ⛔ |
| **Play a remote stream** | 📐 WASAPI shared-mode render | 📐 same | 📐 `pw_stream` playback | 📐 libpulse simple API | 📐 `AudioTrack` (foreground service) |
| **Master volume / mute** | ✅ `IAudioEndpointVolume` | 🟡 same code | 🟡 `wpctl` → `pactl` fallback | 🟡 `pactl` | n/a |
| **Media keys** | 🟡 `SendInput(VK_MEDIA_*)` (interactive session only) | 🟡 | 🟡 MPRIS over D-Bus (`zbus`) | 🟡 | n/a |
| **Observe notifications** | 📐 `UserNotificationListener` (needs package identity + user consent; *other apps'* toasts only via this API) | 📐 same | 📐 not universal: only via the notification server (e.g. GNOME/KDE extension or `org.freedesktop.Notifications` monitor with `BecomeMonitor`, which may be denied) | same | 📐 `NotificationListenerService` (user-granted) |
| **Reply to notifications** | ⛔ for other apps' toasts (no documented API) | ⛔ | ⛔ generally; action invocation only if the server exposes it | ⛔ | 📐 `RemoteInput` actions exposed by the source app |
| **Remote input** | 📐 `SendInput` (not on secure desktop / UAC prompts / lock screen) | 📐 | 📐 X11: XTEST. Wayland: RemoteDesktop portal (user prompt) or libei | same | n/a |
| **Screen capture** | 📐 Windows.Graphics.Capture / DXGI Desktop Duplication | 📐 | 📐 X11: XShm. Wayland: ScreenCast portal + PipeWire (user prompt) | same | 📐 MediaProjection (consent each session) |
| **Telemetry (CPU/RAM/uptime)** | ✅ sysinfo | 🟡 | 🟡 sysinfo (/proc) | 🟡 | n/a |
| **Disks / battery** | ✅ sysinfo disks; `GetSystemPowerStatus` (this desktop: no battery → omitted) | 🟡 | 🟡 sysinfo; `/sys/class/power_supply` | 🟡 | n/a |
| **Now playing (media session)** | ✅ SMTC `GlobalSystemMediaTransportControlsSessionManager` (1809+; only apps that publish to SMTC) | 🟡 | 🟡 MPRIS `Metadata`/`PlaybackStatus` | 🟡 | n/a |
| **Process list / end process** | ✅ sysinfo + WM_CLOSE graceful / TerminateProcess force; own-user only | 🟡 | 🟡 sysinfo + SIGTERM / SIGKILL | 🟡 | n/a |
| **App windows list / close** | ✅ EnumWindows (Alt+Tab filter, cloaked windows excluded) + WM_CLOSE | 🟡 | ⏳ X11 EWMH planned; ⛔ no generic Wayland API (reported unavailable) | same | n/a |
| **LAN discovery** | ✅ mDNS advertise (`mdns-sd`) | 🟡 | 🟡 | 🟡 | 🟡 `NsdManager` browse |

Sources: Microsoft "Application loopback audio capture" sample (requires Windows 10 build
20348+), Core Audio `IAudioEndpointVolume`/`ISimpleAudioVolume` docs; PipeWire `pw_stream`
docs; XDG Desktop Portal RemoteDesktop/ScreenCast specs; Android `AudioPlaybackCaptureConfiguration`,
`NotificationListenerService`, `ACCESS_LOCAL_NETWORK` (API 37) docs.

## What ships in Phase 1

| Feature | Windows agent | Linux agent | Android app |
|---|---|---|---|
| Stable device identity (P-256, SPKI fingerprint) | ✅ key sealed with DPAPI | 🟡 key file 0600 (Secret Service planned) | 🟡 Android Keystore (JVM test uses a software key) |
| Mutually authenticated TLS 1.3 | ✅ | 🟡 (same code, CI) | ✅ via JVM interop test against the real agent; 🟡 on device |
| SAS commit/reveal pairing + owner approval | ✅ | 🟡 | ✅ JVM interop / 🟡 UI on device |
| Reconnect without re-pairing (key-bound, IP-independent) | ✅ | 🟡 | ✅ interop / 🟡 UI |
| Revoke → live session kicked, key rejected | ✅ | 🟡 | ✅ interop |
| Scope enforcement per controller | ✅ | 🟡 | ✅ interop |
| Capability manifest with reasons | ✅ | 🟡 | ✅ rendered |
| Master volume get/set/mute | ✅ real Realtek endpoint | 🟡 | 🟡 UI |
| Media keys | 🟡 | 🟡 | 🟡 UI |
| Telemetry (+ disks, battery) | ✅ | 🟡 | 🟡 UI |
| Now playing + session-targeted media controls | ✅ SMTC (verified "no session" state; playback not exercised) | 🟡 MPRIS | 🟡 UI |
| Process list, graceful quit, confirmed force quit | ✅ (Notepad closed gracefully; lsass refused) | 🟡 | 🟡 UI; ✅ list + denial via interop |
| App windows list + close | ✅ (real Chrome/Claude/Notepad windows) | reported unavailable with reason | 🟡 UI |
| Controller self-unpair ("Forget" removes access on the computer) | ✅ | 🟡 | ✅ interop |
| Settings UI (loopback, token-protected) | ✅ browser page | 🟡 | n/a |
| Autostart (opt-in) | 🟡 HKCU Run | 🟡 systemd user unit | n/a |
| Installer | ✅ Inno Setup .exe built + silent install/uninstall tested | 🟡 .deb/.AppImage/.rpm/PKGBUILD recipes (CI) | ✅ APK built (no device test) |

Everything else in the product brief (audio routing, notifications, 3D workspace, input,
files, services, power, custom commands, Tauri tray UI) is **not implemented yet**; the agent
advertises those capabilities as unavailable with a reason, and the Android UI shows them
disabled.
