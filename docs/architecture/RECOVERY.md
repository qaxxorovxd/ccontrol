# Recovery and rollback

Status (v0.3.0): **implemented** in `desktop/agent/src/audio.rs`.

## State machine (source agent)

```text
VALIDATING → STARTING_STREAM → VERIFYING_PLAYBACK → APPLYING_REVERSIBLE_SOURCE_CHANGE → ACTIVE
ACTIVE → STOPPING → RESTORING_SOURCE → IDLE
any failure → ROLLING_BACK → (IDLE | RECOVERY_REQUIRED)
```
* `ACTIVE` is reported only after the destination's receiver reports show rendered frames,
  **and** any local change has been applied and checked.
* Local silencing runs only after playback is verified. If muting the app also silences the
  capture, which is possible with some loopback implementations, the agent undoes the mute
  immediately and notes it.

## Write-ahead journal
SQLite table `route_journal` (WAL, `synchronous=FULL`), one row per (route, role). Every
transition is written **before** the agent acts. Before muting, the agent records `original`,
the mute state of every session of the app. After each successful mute it records `applied`,
the sessions it actually changed. Sinks are journaled too.

## Leases
Both the source route and the sink have a lease (default 30 s, 5–600 s allowed). The phone
renews both every lease/3 while the route is live, from a visible foreground service when the
app is in the background. A supervisor task on each agent checks every 500 ms:
* lease expired → stop and restore (`note: lease expired`);
* capture failed or ended (source app exited, device lost) → roll back with the error;
* no receiver report for 8 s → roll back ("destination stopped responding").

## Restore rules (compare-and-swap)
For each session in `applied`:
| Current state | Action | Outcome contribution |
|---|---|---|
| still muted (our change) | set back to original | `restored` |
| already back to original (user changed it) | leave it | `left_user_change` |
| session no longer exists (app exited/restarted) | nothing possible | `partial` |
| OS call failed | — | `manual_action_required` → state `RECOVERY_REQUIRED` and a UI hint |
If nothing was applied the outcome is `not_needed`. Stop and rollback are idempotent.

## Crash and restart
* `AudioManager::new` calls `reconcile` before the agent accepts connections. Every unfinished
  journal row is restored by the rules above and marked terminal with
  `note: recovered after the agent stopped during '<state>'`. Phones see it in `route.list`.
* Graceful shutdown (Ctrl-C, service stop) finishes all live routes first.
* Linux: `Restart=on-failure` in the systemd user unit restarts a crashed agent. Windows:
  autostart at sign-in; a crash is recovered at the next start.
* If every ccontrol process is dead or the computer is off, restoration waits until the agent
  starts again. That is the documented physical limit. A Windows app that was muted and then
  restarted may keep its mute in the OS mixer; the outcome is then `partial`, and the agent asks
  the user to unmute it.

## Tests
| Test | What it proves |
|---|---|
| `e2e::journal_reconciles_a_crash_at_every_route_state` | For all 9 crash points (every state, plus "original journaled but not yet muted") a fresh agent over the same DB restores exactly what was changed, reports `restored`/`not_needed`, and leaves no unfinished rows |
| `e2e::expired_lease_rolls_back_source_and_sink` | Phone vanishes → both agents roll back on their own; the app is unmuted |
| `e2e::user_change_after_routing_is_not_overwritten` | CAS: user unmute is respected (`left_user_change`) |
| `e2e::unconfirmed_destination_rolls_back_without_touching_the_app` | No playback confirmation → rollback, the app never muted |
| `e2e::silencing_that_kills_the_capture_is_undone` | Safety check undoes a mute that would silence the stream |
| `recovery_leaves_a_user_unmute_alone_and_reports_missing_sessions` | `left_user_change`, `partial`, `not_needed` |
| Real processes (PROGRESS.md) | `taskkill /F` on the source agent mid-route: the destination stopped on lease expiry; after restart the source journal row moved from `active` to `idle`/`not_needed` with the recovery note |

Not yet exercised on hardware: a crash while a **real** app session is muted. That needs
per-app capture, which needs Windows build 20348+; the mute/unmute adapter itself is verified
on hardware by `windows_hw::session_mute_is_reversible`.
