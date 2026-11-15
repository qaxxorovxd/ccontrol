# Recovery and rollback (design; not implemented yet)

Status: **not implemented.** No Phase 1 action changes audio routing. Master volume is a
direct user command, not a temporary change, so nothing needs restoring. The persistence
layer (SQLite WAL, `synchronous=FULL`) is already the intended journal store.

## Write-ahead route journal
Before every state-changing step the agent writes a row to a `route_journal` table:
`route_id, owner_controller, step, original_state (JSON: session id, mute, volume, endpoint/target),
applied_state, lease_expiry, status`. Only then does it touch the OS.

## Leases
The controller renews a route lease every 10 s (lease 30 s). The agent, not the phone, owns
active routes. Closing the app or losing Wi-Fi lets the lease expire, and the agent then rolls back.

## Rollback rules
* Idempotent: each step's undo can run any number of times.
* Compare-and-swap: before restoring, read the current value. If it differs from `applied_state`,
  the human changed it on purpose, so **do not overwrite**. Report `Not needed` or `Partial recovery`.
* Outcomes shown to the user: `Restored`, `Not needed`, `Partial recovery`, `Manual action required`.

## Crash and restart
* On startup the agent replays unfinished journal rows before opening the listener.
* Windows: a tiny restartable supervisor (or Task Scheduler "restart on failure") relaunches a
  crashed agent. Linux: `Restart=on-failure` in the systemd user unit, already shipped.
* If every ccontrol process is dead or the computer is off, restoration happens at the next
  start. Immediate rollback is physically impossible in that case, and the docs say so.

## Required tests (Phase 3)
Kill the agent (`taskkill /F` / `kill -9`) at every state transition, cut the network during
each transition, unplug the destination headset, and exit the source app. After each, assert
the journal-driven outcome and that other apps' audio is untouched.
