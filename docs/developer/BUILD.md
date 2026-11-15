# Building ccontrol

## Toolchains

| Part | Needs | Verified here |
|---|---|---|
| Rust workspace | Rust stable ≥ 1.80; Windows: MSVC Build Tools (C++ workload, for `ring`/SQLite); Linux: `gcc`, `pkg-config` | Rust 1.9x stable + VS 2022 Build Tools 14.44 on Windows 10 |
| Android | JDK 17+ (Android Studio JBR works), Android SDK platform 37 + build-tools 36, Gradle 9.6 (wrapper) | JBR/OpenJDK 25, AGP 9.5.1, Kotlin 2.3.21 |
| Windows installer | Inno Setup 6 (`winget install JRSoftware.InnoSetup`) | yes |
| Linux packages | `dpkg-deb`, `appimagetool`, `rpmbuild`, `makepkg` | CI only |

## Desktop agent

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p ccontrol-agent
```

Run it in the foreground with a throwaway data dir:

```bash
target/release/ccontrol-agent --data-dir ./.agent-dev run
```

It prints the settings URL (`http://127.0.0.1:47801/#token=…`). `ccontrol-agent ui` opens it.

Pair and call actions with the developer controller:

```bash
target/release/ccontrol-ctl --state ./.ctl pair 127.0.0.1:47800
target/release/ccontrol-ctl --state ./.ctl call 127.0.0.1:47800 volume.get
```

## Android

```bash
cd android
./gradlew :core:testDebugUnitTest :app:assembleDebug
```

Kotlin↔Rust interop tests (run the real agent binary):

```bash
CCONTROL_AGENT_BIN=$PWD/../target/debug/ccontrol-agent ./gradlew :core:testDebugUnitTest
```

Install on a phone:

```bash
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

The emulator needs CPU acceleration: install the *Android Emulator hypervisor driver* via the
SDK Manager or enable *Windows Hypervisor Platform*. Both need admin rights and a reboot. In
the emulator, the host's agent is reachable at `10.0.2.2:47800` (mDNS does not cross the NAT).

## Packages

```bash
iscc /DAppVersion=0.1.0 packaging\windows\ccontrol.iss          # → dist/ccontrol-Setup-x64.exe
packaging/debian/build-deb.sh target/release/ccontrol-agent 0.1.0 dist
packaging/appimage/build-appimage.sh target/release/ccontrol-agent dist
rpmbuild -ba packaging/fedora/ccontrol.spec   # needs source tarball in ~/rpmbuild/SOURCES
(cd packaging/arch && makepkg)                # needs source tarball next to PKGBUILD
scripts/checksums.sh dist
```
