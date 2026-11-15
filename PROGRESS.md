# Progress

_Last update: 2026-11-15 (v0.2.0)_

## Milestone status

| Phase | State |
|---|---|
| 0 Feasibility + capability matrix | ✅ [FEATURE_MATRIX.md](docs/os-capabilities/FEATURE_MATRIX.md) |
| 1 Trust + control vertical slice | ✅ desktop verified on Windows 10 19045; Android verified via JVM interop, **not yet on a device** |
| 2 Dashboard + controls | ✅ processes (list, graceful quit, confirmed force quit), app windows (list, close), now playing + session media controls, disks/battery, self-unpair, measured idle footprint. Services and display controls moved to Phase 6. |
| 3 Audio engine + recovery | ⏳ design only ([AUDIO_ROUTING.md](docs/architecture/AUDIO_ROUTING.md), [RECOVERY.md](docs/architecture/RECOVERY.md)) |
| 4 3D workspace | ⏳ |
| 5 Notifications | ⏳ |
| 6 Advanced control | ⏳ |
| 7 Release quality | 🟡 CI pipeline + Windows installer done; Linux packages CI-only |

## Phase 2 additions (v0.2.0)

| Area | Desktop agent | Android |
|---|---|---|
| Processes | `process.list` via sysinfo, CPU normalized to whole-machine %; `process.terminate` graceful first (WM_CLOSE to the app's windows / SIGTERM), force only on request; refuses the agent, PIDs ≤ 4, critical system processes and other users' processes; PID-reuse name check | Processes screen: search, sort (CPU/Memory/Name), "Mine" filter, lock icon + reason on protected rows, Quit → Force quit two-step dialog |
| Apps & windows | `app.list` (EnumWindows, Alt+Tab filter, cloaked excluded), `app.close` (WM_CLOSE after HWND/PID re-check). Linux reports unavailable with reason. | Apps & windows screen with close confirmation |
| Media | `media.status` via SMTC (Windows) / MPRIS (Linux); commands now target the current SMTC session, falling back to media keys | Now-playing card (title, artist, app, progress), play/pause state, buttons follow `can_*` |
| Telemetry | Disks (pseudo-filesystems filtered), battery % + charging | Disk meters, battery line |
| Pairing | `pairing.unpair`: a controller can remove its own trust (no scope needed) | "Forget" now also removes access on the computer when it is online |
| Dashboard | — | Quick play/pause per online computer |

## Verified on this machine (Windows 10 Pro 19045, x86_64)

| Check | Result |
|---|---|
| `cargo fmt --check`, `cargo clippy -D warnings` (Windows + Linux cross-check of adapters) | clean |
| `cargo test --workspace` | **52 passed**, 0 failed (14 end-to-end TLS tests incl. process safeguards, app scopes, self-unpair) |
| Real agent, `ccontrol-ctl` | 279 processes listed (152 terminable); `lsass.exe` force kill → `Forbidden: Critical system process`; Notepad opened → `process.terminate` graceful → `exited:true`; second Notepad → `app.close` → closed; `app.list` showed the real Chrome/Claude/ChatGPT windows with focus; `media.status` → `active:false` (nothing playing); telemetry disk `C:\` 476 GiB / 242 GiB free, no battery |
| Idle footprint (release agent, 60 s idle, no controller) | **0 ms CPU**, working set 20.8 MB, private 10.8 MB, 9 threads, 324 handles; binary 5.4 MB |
| Android unit tests | **10 passed** (Phase 2 wire shapes pinned to the Rust literals) |
| Kotlin↔Rust interop against the real agent | **4 passed** (adds: real process list, agent marked non-terminable, terminate denied without scope, disks present, self-unpair → `revoked` + untrusted) |
| Android lint, debug + minified release build | passed; release APK 3.1 MB, versionName 0.2.0 |
| Windows installer | rebuilt for 0.2.0 (`ccontrol-agent --version` → 0.2.0) |

Artifacts in `dist/` (git-ignored) with `SHA256SUMS`: `ccontrol-Setup-x64.exe` and
`ccontrol-android-arm64.apk` (minified release signed with the **local debug key**, side-load only).

## Not verified / known gaps
* **No Android device run yet** (emulator needs a hypervisor driver; no phone over adb). Keystore
  client auth, NSD, QR scanner, the API 37 local-network permission, R8-minified serialization and
  every Compose screen are compiled and lint-clean but unseen on a screen.
* SMTC with real playback (title/position, session-targeted skip) not exercised: nothing was playing.
* Linux agent never executed. MPRIS status, SIGTERM path, `/sys` battery are type-checked only.
* Linux window listing not implemented (X11 EWMH planned; Wayland has no generic API).
* Services (SCM/systemd), display, power, files, input, notifications, audio routing: not started.
* Tauri tray UI not built (loopback settings page instead); JSON instead of Protobuf; manual DI +
  AtomicFile instead of Hilt/Room (see ADRs in OVERVIEW.md).
* Linux private key is a 0600 file; Secret Service/TPM sealing is pending.

## Next engineering tasks
1. Run the APK on a physical Android device against the Windows agent.
2. Push to GitHub so CI builds `.deb`/AppImage/rpm and runs the Linux tests.
3. Phase 3: audio session enumeration (WASAPI sessions with PID, PipeWire nodes), then one real
   PC→PC Opus stream with route journal, leases and kill-at-every-state tests. This Windows 10
   machine (build 19045) can be a destination but not a per-app capture source.
