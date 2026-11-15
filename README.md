# ccontrol

Local-first control of your Windows and Linux computers from an Android phone, with
mutually authenticated pairing, per-phone permissions, and honest capability reporting.

> **Status: v0.2 — Phases 1–2 (trust, pairing, device controls) are working.** Audio routing,
> notifications, the 3D workspace, remote input, files, power and custom commands are **not
> built yet**. See [PROGRESS.md](PROGRESS.md) and the
> [feature matrix](docs/os-capabilities/FEATURE_MATRIX.md).

## What works today
* Desktop agent (Rust, ~5.5 MB) runs per user and keeps running when the settings page is closed.
  It advertises itself over mDNS and exposes one TLS 1.3 port that requires a client certificate.
* Pairing: QR code or LAN discovery or manual IP. A 6-digit commit/reveal code appears on both
  screens, and the desktop owner approves and picks the permissions.
* Phones reconnect by key, not IP, with no QR again. Revoking on the desktop kicks the phone immediately.
* Controls: master volume and mute (Windows Core Audio verified; Linux via PipeWire/Pulse), now playing
  with session media controls, processes (graceful quit, confirmed force quit, protected system
  processes), app windows (Windows), disks/battery,
  media keys, CPU/RAM/uptime telemetry, capability manifest.
* Android app (Kotlin + Compose, minSdk 29, targetSdk 37): dashboard, add/pair, device screen
  with honest disabled states.

## Quick start (Windows)
1. Install `dist/ccontrol-Setup-x64.exe` (or build: see [BUILD.md](docs/developer/BUILD.md)).
2. Let it start the agent and open the settings page.
3. Install the APK on your phone, tap **Add computer**, scan the QR code, check that the codes
   match, and click **Accept** on the computer.

## Repository
```
crates/       protocol, security, persistence, capabilities, system-control, discovery
desktop/agent ccontrol-agent (daemon + loopback settings UI + CLI), ccontrol-ctl (dev controller)
android/      core (protocol/TLS/pairing/discovery), app (Compose UI)
packaging/    windows (Inno Setup), debian, appimage, fedora (.spec), arch (PKGBUILD), linux (systemd unit)
docs/         architecture, protocols, capabilities, security, operations
```

## Documentation
[Overview](docs/architecture/OVERVIEW.md) · [Threat model](docs/architecture/THREAT_MODEL.md) ·
[Control protocol](docs/protocols/CONTROL_PROTOCOL.md) · [Pairing](docs/protocols/DEVICE_PAIRING.md) ·
[Audio routing design](docs/architecture/AUDIO_ROUTING.md) · [Recovery design](docs/architecture/RECOVERY.md) ·
[Build](docs/developer/BUILD.md) · [Troubleshooting](docs/operations/TROUBLESHOOTING.md) ·
[Release](docs/operations/RELEASE.md) · [Privacy](docs/security/PRIVACY.md)
