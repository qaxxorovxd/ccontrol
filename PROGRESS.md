# Progress

_Last update: 2026-11-15_

## Milestone status

| Phase | State |
|---|---|
| 0 Feasibility + capability matrix | ✅ [FEATURE_MATRIX.md](docs/os-capabilities/FEATURE_MATRIX.md) |
| 1 Trust + control vertical slice | ✅ desktop verified on Windows 10 19045; Android verified via JVM interop, **not yet on a device** |
| 2 Dashboard + controls | 🟡 dashboard, device screen, volume/media/telemetry done; processes/apps/services not started |
| 3 Audio engine + recovery | ⏳ design only ([AUDIO_ROUTING.md](docs/architecture/AUDIO_ROUTING.md), [RECOVERY.md](docs/architecture/RECOVERY.md)) |
| 4 3D workspace | ⏳ |
| 5 Notifications | ⏳ |
| 6 Advanced control | ⏳ |
| 7 Release quality | 🟡 CI pipeline + Windows installer done; Linux packages CI-only |

## Verified on this machine (Windows 10 Pro 19045, x86_64)

| Check | Command | Result |
|---|---|---|
| Rust format/lint | `cargo fmt --check; cargo clippy --workspace --all-targets -- -D warnings` | clean |
| Rust tests | `cargo test --workspace` | **40 passed**, 0 failed (incl. 10 end-to-end TLS tests) |
| Linux adapters type-check | `cargo clippy --target x86_64-unknown-linux-gnu -p ccontrol-system-control -p ccontrol-capabilities -p ccontrol-discovery -p ccontrol-protocol` | clean (not executed on Linux) |
| Real agent + dev controller | `ccontrol-agent run` + `ccontrol-ctl pair/call` | pairing with matching SAS (`768633` on both sides), approval via settings API, `volume.get` → `Speakers (Realtek(R) Audio)` level 0.10, `volume.set 0.1` OK, telemetry OK, `volume.set 2` → `InvalidRequest` |
| Settings API hardening | `curl` without token / with `Host: evil.com` | 403 / 403 |
| Android unit tests | `./gradlew :core:testDebugUnitTest` | **9 passed** (wire shapes match Rust, SAS golden vector, framing, QR parser) |
| Kotlin↔Rust interop | `CCONTROL_AGENT_BIN=… ./gradlew :core:testDebugUnitTest` | **3 passed**: pair with matching SAS, scope denial, reconnect without pairing, wrong pin → IdentityChanged, revoke → live `revoked` event + untrusted on reconnect, reject path, admin token |
| Android lint | `./gradlew :app:lintDebug` | passed |
| APK | `./gradlew :app:assembleDebug :app:assembleRelease` | built; targetSdk 37, minSdk 29; release minified 3.0 MB |
| Windows installer | `ISCC packaging\windows\ccontrol.iss` | built 4.0 MB; silent per-user install → `ccontrol-agent --version` → silent uninstall left no files/shortcuts |

Artifacts in `dist/` (git-ignored), SHA-256 in `dist/SHA256SUMS`:
* `ccontrol-Setup-x64.exe`: real installer.
* `ccontrol-android-arm64.apk`: minified release build signed with the **local debug key** (side-load only).

## Not verified / known gaps
* **No Android device run yet.** The emulator needs a hypervisor driver (AEHD or Windows
  Hypervisor Platform, admin + reboot) and no phone was connected over adb. Keystore-backed TLS
  client auth, NSD discovery, the QR scanner, the API 37 local-network permission flow, and
  R8-minified serialization still need a device run.
* Linux agent never executed (no Linux host). The `.deb`/AppImage/rpm/PKGBUILD recipes run only
  in CI, which has not run yet (no remote repository).
* Tauri tray UI not built; the settings UI is a loopback web page served by the agent (see ADR 5).
* Protobuf replaced by test-pinned JSON for v1 (ADR 4). Hilt/Room replaced by manual DI + AtomicFile JSON (ADR 6).
* Linux private key is a 0600 file; Secret Service/TPM sealing is pending.
* Windows media keys (`SendInput`) are implemented but have not been triggered from the phone.
* Phone-side "Forget" does not revoke on the desktop (no `pairing.unpair` action yet).
* No idle CPU/RAM benchmark published yet.

## Next engineering tasks
1. Run the APK on a physical Android device against the Windows agent: Keystore client auth,
   NSD, QR, local-network permission, release build.
2. Run CI on GitHub (push repo) to produce `.deb`/AppImage/rpm and the Linux test pass.
3. Phase 2: process/app list (separate concepts), media metadata (SMTC/MPRIS), `pairing.unpair`
   action, idle resource measurements.
4. Phase 3: audio session enumeration on both OSes, then one real PC→PC Opus stream with the
   route journal and kill-at-every-state tests (Windows destination first; this Windows 10 machine
   can be a destination but not a per-app source).
