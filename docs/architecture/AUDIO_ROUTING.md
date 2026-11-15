# Audio routing (design; not implemented yet)

Status: **not implemented.** The agent reports `audio.route` and `audio.per_app_capture`
as unavailable, with the OS-specific reason. This document fixes the design so Phase 3 can
be built against it.

## Terms (kept distinct)
* **Session**: an OS audio session/stream belonging to an app (Windows `IAudioSessionControl2`
  with PID; PipeWire node with `application.process.id`).
* **Capture**: obtaining that session's PCM (not the whole mix).
* **Local silencing**: stopping local playback of *only* that session, reversibly.
* **Remote playback**: rendering a received stream on the destination's chosen endpoint.

## Per-platform plan

| Source | Capture | Local silencing (reversible) |
|---|---|---|
| Windows ≥ build 20348 | Process loopback (`ActivateAudioInterfaceAsync`, `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE`) | `ISimpleAudioVolume::SetMute(TRUE)` on that session, with the original mute state journaled. Process loopback captures *before* the session volume is applied, so the captured stream stays audible while local output is muted. **To verify on hardware** before shipping. |
| Windows < 20348 (e.g. this machine, 19045) | ⛔ per-app capture. Offer only "whole output device" capture/forward, labelled as such, with local playback kept. | — |
| Linux PipeWire | `pw_stream` capture linked to the app's output node | Move the app's node to a ccontrol null sink via WirePlumber `target.object` metadata, saving the original target. Never touch the default sink. |
| Android | Only apps that permit `AudioPlaybackCapture`, after MediaProjection consent, in a `mediaProjection` foreground service | ⛔ (cannot silence other apps) |

Destinations: WASAPI shared render / `pw_stream` playback / `AudioTrack` on Android.

## Stream engine
* Opus 48 kHz, 10 ms frames, RTP over UDP (keyed with a per-route secret exchanged on the TLS
  control channel; SRTP via `webrtc-rs` or DTLS-SRTP is the candidate), separate from the control channel.
* Adaptive jitter buffer (target 20–60 ms), Opus PLC, drift compensation by adaptive resampling.
* Metrics per route: one-way latency (via clock-offset estimation on ping/pong), jitter, loss,
  underruns, bitrate, CPU.
* Latency target is empirical: measured on wired/5 GHz LAN and published with hardware details.

## Route state machine
`IDLE → VALIDATING → PREPARING_DESTINATION → STARTING_STREAM → VERIFYING_PLAYBACK →
APPLYING_REVERSIBLE_SOURCE_CHANGE → ACTIVE → STOPPING → RESTORING_SOURCE → IDLE`;
any failure → `ROLLING_BACK → IDLE | RECOVERY_REQUIRED`. `ACTIVE` is reported only after the
destination confirms that rendered frames are flowing **and** the source change is applied.
See [RECOVERY.md](RECOVERY.md).

The 3D drag-and-drop gesture and the 2D route matrix both call the same `route.audio.start`
action. Neither has a separate code path.
