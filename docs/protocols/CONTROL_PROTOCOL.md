# Control protocol (v1)

Transport: TCP → TLS 1.3 (client certificate mandatory) → frames.
Frame: `u32` big-endian length (1..=1 048 576) + UTF-8 JSON object with a `"type"` field.
Oversized, empty or malformed frames close the session with `invalid_request`.

## Session

| Step | Direction | Message |
|---|---|---|
| 1 | C→A | `{"type":"hello","protocol_version":1,"min_protocol_version":1,"device_id":"<own spki fp>","peer_device_id":"<agent fp as seen>","device_name":"Pixel 9","platform":"android","app_version":"0.1.0"}` |
| 2 | A→C | `{"type":"hello_ack","protocol_version":1,"device_id":"…","device_name":"…","paired":false,"granted_scopes":[]}` |
| 3 | pairing | see [DEVICE_PAIRING.md](DEVICE_PAIRING.md); ends with `pair_result` + a second `hello_ack` (`paired:true`) |
| 4 | C→A | `{"type":"request","id":1,"action":"volume.set","level":0.4}` |
| 5 | A→C | `{"type":"response","id":1,"result":{"kind":"volume","level":0.4,"muted":false,"endpoint_name":"Speakers"}}` |
| – | A→C | `{"type":"event","event":"revoked","reason":"…"}` then close |
| – | C→A / A→C | `{"type":"ping","seq":n}` / `{"type":"pong","seq":n}` (controller pings every 30 s; agent drops sessions idle > 95 s) |
| – | A→C | `{"type":"error","error":{"code":"…","message":"…"}}` fatal, then close |

The agent rejects a `hello` whose `device_id`/`peer_device_id` disagree with the certificates
actually presented (`identity_mismatch`), or whose version is outside `[1, 1]` (`incompatible_protocol`).

## Actions (v1)

| `action` | Params | Scope | Result `kind` |
|---|---|---|---|
| `device.info` | – | `device.read` | `manifest` |
| `telemetry.get` | – | `telemetry.read` | `telemetry` |
| `volume.get` | – | `media.control` | `volume` |
| `volume.set` | `level` 0.0–1.0 | `media.control` | `volume` |
| `volume.mute` | `muted` bool | `media.control` | `volume` |
| `media.key` | `key`: `play_pause`/`next`/`previous`/`stop` | `media.control` | `done` |
| `media.status` | – | `media.control` | `media_status` (`active:false` when nothing plays) |
| `process.list` | – | `device.read` | `processes` (pid, name, exe?, cpu % of machine, memory, `same_user`, `terminable`, `protected_reason`) |
| `process.terminate` | `pid`, `name`, `force` | `process.terminate` | `terminated` (`method`, `exited`, `note`) |
| `app.list` | – | `device.read` | `apps` (`window_id`, `pid`, `process_name`, `title`, `focused`, `minimized`) |
| `app.close` | `window_id`, `pid` | `process.terminate` | `done` |
| `pairing.unpair` | – | none (affects the caller only) | `done`, then `event: revoked` and close |

### Process and window semantics (v0.2)

* **Applications, windows and processes are different things.** `app.close` asks one window to
  close (WM_CLOSE, like clicking X). `process.terminate` with `force:false` asks the whole program
  to quit: WM_CLOSE to all its windows on Windows, SIGTERM on Linux. The agent waits up to 5 s and
  reports `exited`. A windowless process has no graceful path on Windows (`unsupported`).
* `force:true` kills immediately. The app only sends it after a second, explicit confirmation.
* Guards: `name` must match the live PID (reuse protection). The agent itself, PIDs ≤ 4, a
  critical-process list (csrss, lsass, winlogon, svchost, dwm, … / systemd, dbus, the compositor,
  PipeWire, …) and **any process owned by another OS user** are refused with `forbidden`. The
  agent never elevates.

Unknown actions fail deserialization; there is no shell or free-form command endpoint.

## Errors

`incompatible_protocol, not_paired, identity_mismatch, pairing_rejected, pairing_timeout, forbidden,
pairing_failed, scope_denied, unsupported, invalid_request, rate_limited, busy, timeout, internal`.

## Request semantics

* Request ids are unique per session; a repeated id within the last 512 is rejected without executing.
* ≤ 8 requests in flight per session (`busy` beyond); platform calls time out after 10 s.
* Trust and scopes are re-read for every request.

## Capability negotiation

`hello_ack.manifest` (sent when `device.read` is granted) lists every capability with
`available`, `reason` and the needed `scope`. Controllers render unknown or unavailable
capabilities as disabled. Newer agents may add keys; unknown JSON fields are ignored on Android.

## Wire-compatibility tests

* Rust: `crates/protocol/src/messages.rs` tests pin exact JSON strings.
* Kotlin: `android/core/src/test/.../ProtocolTest.kt` asserts the same strings.
* Interop: `AgentInteropTest.kt` drives the real agent binary from the Kotlin stack.
