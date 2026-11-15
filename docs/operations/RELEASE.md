# Releases

Tag `vX.Y.Z` → `.github/workflows/ci.yml` builds every artifact on its native runner, runs
smoke tests, writes `SHA256SUMS` and attaches everything to a GitHub release.

| Artifact | Built by | Smoke test in CI |
|---|---|---|
| `ccontrol-Setup-x64.exe` | Inno Setup on `windows-2022` | silent per-user install → `--version` → silent uninstall |
| `ccontrol-amd64.deb` | `dpkg-deb` on Ubuntu 24.04 | `apt install` → `--version` → remove |
| `ccontrol-x86_64.AppImage` | `appimagetool` 1.9.0 | `--version` |
| `ccontrol-*.x86_64.rpm` | `rpmbuild` in `fedora:42` (runs `cargo test` in `%check`) | build only |
| Arch `PKGBUILD` | recipe only | not built in CI |
| `ccontrol-android-arm64.apk` | Gradle; release-signed when secrets exist, else debug-signed with a warning | unit + interop tests |

The APK is architecture-independent Kotlin (one APK runs on arm64 and every other ABI). The
name follows the requested convention.

## Signing
* Android: secrets `CCONTROL_KEYSTORE_B64`, `CCONTROL_KEYSTORE_PASSWORD`, `CCONTROL_KEY_ALIAS`,
  `CCONTROL_KEY_PASSWORD`. Keystores are never committed (`.gitignore` blocks `*.jks`/`*.keystore`).
* Windows Authenticode: add `signtool sign` after ISCC once a certificate is provided. Unsigned
  installers trigger SmartScreen.

## Local artifacts from this machine (2026-11-15)
`dist/` holds `ccontrol-Setup-x64.exe` (built and install/uninstall-tested here) and
`ccontrol-android-arm64.apk`: the minified release build, **signed with the local Android debug
key** for side-loading only, not for distribution. Linux packages were not built locally (no
Linux host). `dist/` is git-ignored.

## Install / update / uninstall
* Windows: run the installer (per-user, no admin). Re-running a newer installer updates in place.
  Uninstall via *Apps & features*: it removes autostart, stops the agent, removes the firewall rule
  (if added) and asks whether to delete identity/pairings.
* Debian/Ubuntu: `sudo apt install ./ccontrol-amd64.deb`, then `systemctl --user enable --now ccontrol-agent`.
  Uninstall: `systemctl --user disable --now ccontrol-agent; sudo apt remove ccontrol`;
  delete `~/.local/share/ccontrol` to forget identity and pairings.
* AppImage: `chmod +x ccontrol-x86_64.AppImage && ./ccontrol-x86_64.AppImage` (runs the agent);
  `./ccontrol-x86_64.AppImage enable-autostart` registers a user unit pointing at the AppImage.
* Android: `adb install -r ccontrol-android-arm64.apk`.

## Windows 10 note
Windows 10 support ended on 2025-10-14. ccontrol runs on it, but the OS no longer gets
security updates without ESU. Per-app audio capture is unavailable below build 20348.
