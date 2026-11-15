# Progress

_Last update: 2026-11-15 (v0.3.0)_

## Milestone status

| Phase | State |
|---|---|
| 0 Feasibility + capability matrix | ✅ [FEATURE_MATRIX.md](docs/os-capabilities/FEATURE_MATRIX.md) |
| 1 Trust + control vertical slice | ✅ desktop verified on Windows 10 19045; Android verified via JVM interop, **not yet on a device** |
| 2 Dashboard + controls | ✅ processes, app windows, now playing, disks/battery, self-unpair |
| 3 Audio engine + recovery | ✅ on Windows: real PC→PC (two agent processes) whole-output and test-tone routes, Opus/UDP with AEAD, jitter buffer, measured stats, agent-owned leases, write-ahead journal, crash reconcile (incl. real `taskkill /F`). Per-app capture is implemented for Windows ≥ 20348 and PipeWire but **not run** (this PC is 19045; no Linux host). |
| 4 3D workspace | ⏳ (the 2D route matrix exists and uses the same route API the 3D view will call) |
| 5 Notifications | ⏳ |
| 6 Advanced control | ⏳ (services and display moved here) |
| 7 Release quality | 🟡 CI pipeline + Windows installer; Linux packages CI-only |

## Phase 3 additions (v0.3.0)

* `crates/audio-engine`:
  - Opus codec (10 ms, 128 kb/s, FEC).
  - ChaCha20-Poly1305 packets with a replay window.
  - Adaptive jitter buffer: FEC/PLC, drift control, RFC 3550 jitter.
  - Sender and receiver threads, with receiver reports for RTT.
  - WASAPI backend: endpoints, sessions, endpoint loopback, process loopback (≥ 20348), render, session mute.
  - PipeWire backend: `pw-dump` / `pw-record` / `pw-play`.
* Agent (`desktop/agent/src/audio.rs`):
  - Source-route state machine exactly as specified, with write-ahead journal and leases.
  - Compare-and-swap restore and startup reconcile.
  - Safety check that undoes local silencing if it would silence the capture.
  - Sinks, arbitration (one route per source, owner-only renew).
  - The settings page lists routes and sinks with a local **Stop & restore**.
* Protocol: `audio.devices`, `audio.sink.prepare/stop`, `route.audio.start/stop`, `route.lease.renew`, `route.list`.
* Android:
  - `RouteOrchestrator`: prepare the sink → start → renew leases every 10 s → stats every 2 s → stop.
  - Audio screen: 2D route matrix, output picker, "silence on source" option (enabled only where supported), live route cards with latency, loss, jitter, bitrate, level and outcome.
  - `connectedDevice` foreground service with a Stop action while routes are live; connections are kept in the background only then.
* `ccontrol-ctl route` drives a full route between two agents from the command line.

## Verified on this machine (Windows 10 Pro 19045, x86_64; Realtek + USB speakers)

| Check | Result |
|---|---|
| `cargo fmt --check`, `cargo clippy -D warnings` (+ Linux cross-check of non-C crates) | clean |
| `cargo test --workspace` | **78 passed**, 0 failed (4 hardware tests ignored by default) |
| Hardware tests (`windows_hw -- --ignored`) | **4 passed**: real endpoints/sessions listed; loopback captured the played tone (peak 0.100 for a 0.1 tone, 81 frames in 800 ms); playing on the USB device is *not* heard on the Realtek loopback, and silence stays paced; session mute is reversible |
| Agent route tests (e2e, fake audio, real UDP/Opus/journal) | full lifecycle with true handoff (`restored`), lease expiry rollback on both agents, user-unmute respected (`left_user_change`), unconfirmed destination rolls back without muting, safety check undoes a mute that silences capture, second phone gets `busy`/`forbidden`, **crash at every one of 9 journal states reconciles correctly** |
| Two real agent processes, `ccontrol-ctl route` | Test tone → USB speakers: ACTIVE in 548 ms, 474 frames, **0 lost**, jitter 0.01 ms, est. latency ~60 ms, 165 kb/s, out peak 0.151 for 0.150 in. Real Realtek output (Windows notification chimes) → USB speakers: ACTIVE in 553 ms, 668 frames, 0 lost, peaks reproduced (0.15–0.21 → 0.17–0.24), latency 50–80 ms |
| Real crash | `taskkill /F` on the source mid-route → destination sink stopped on lease expiry ("lease expired"); after restart the source reconciled the `active` journal row to `idle`/`not_needed` ("recovered after the agent stopped during 'active'"), 0 unfinished rows |
| Settings page | live route row with stats; **Stop & restore** stopped it (outcome shown) |
| Android unit tests | **11 passed** (audio wire shapes pinned to Rust) |
| Kotlin↔Rust interop | **5 passed**, incl. a test-tone route between **two real agent processes** driven by the Kotlin encoding (playing, 0 loss, RTT reported, stop → `not_needed`) |
| Android lint, debug + minified release | passed; APK 3.2 MB, versionName 0.3.0, FGS permissions present |
| Idle footprint (release 0.3.0, 60 s, no controller, no route) | **0 ms CPU**, working set 20.6 MB, private 10.5 MB, 10 threads; binary 6.5 MB |
| Windows installer 0.3.0 | built 4.4 MB; silent install → `--version 0.3.0` → silent uninstall clean |

Artifacts in `dist/` (git-ignored) with `SHA256SUMS`. The APK is the minified release build
signed with the **local debug key** (side-load only).

## Not verified / known gaps
* **No Android device run yet.** The Audio screen, foreground service and notification have not
  been seen on a screen.
* **Per-app capture end to end** (Windows process loopback, PipeWire stream capture) and **true
  handoff with a real app muted** were not run here: this PC is build 19045, and there is no
  Linux host. A crash while a real app is muted is covered by the journal tests with a fake
  audio system and by the hardware mute/unmute test, not by one combined hardware run.
* All measurements are over loopback on one PC. **No Wi-Fi/Ethernet latency has been measured yet.**
* Drift is handled by frame drop/concealment, not resampling.
* Linux local silencing is not implemented (local playback is kept and the reason is shown).
* Phone as audio source or destination is not implemented.
* Still open from earlier phases: Tauri tray UI, Protobuf, Hilt/Room, Linux key sealing,
  Linux window listing.

## Next engineering tasks
1. Run the APK on a phone with two PCs on Wi-Fi; publish measured latency, jitter and loss for that LAN.
2. On a Windows 11 / build ≥ 20348 machine: verify process loopback and the true-handoff mute
   (does the session mute keep process-loopback audio?), then kill the agent while the app is muted.
3. Push to GitHub to run CI (Linux build, `.deb`/AppImage/rpm) and test the PipeWire backend on a Linux desktop.
4. Phase 4: 3D workspace (SceneView/Filament) wired to `RouteOrchestrator.start`, with the 2D matrix as the accessible alternative.
