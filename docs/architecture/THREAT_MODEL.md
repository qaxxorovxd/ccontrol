# Threat model

## Assets
Ability to control a computer (volume today; input, files and power later); notification
contents (future); device private keys; trust store; audio/screen streams (future).

## Trust boundaries
1. LAN ↔ agent control port (TLS, mutually authenticated, private-range source IPs only by default).
2. Local browser/processes ↔ loopback settings UI (127.0.0.1 only, token + Host check).
3. Agent ↔ OS APIs (runs as the signed-in user, never as SYSTEM/root).
4. Phone app ↔ Android Keystore (non-exportable key).

## Threats and defenses

| Threat | Defense | Test |
|---|---|---|
| Unpaired LAN client sends commands | Server only accepts `pair_*` before trust; requests before pairing close the session | `e2e::unpaired_client_cannot_send_requests` |
| Spoofed mDNS advertisement | TXT records are hints; trust = TLS fingerprint. QR pins the fingerprint; SAS + owner approval otherwise | `tls::pinned_fingerprint_must_match`, interop wrong-pin |
| MITM during first pairing | Commit/reveal SAS bound to both certificate fingerprints | `sas` tests, `e2e::bad_commitment_fails_pairing` |
| Agent impersonation after pairing | Pinned SPKI on every reconnect; mismatch surfaces as "Identity changed" | interop `IdentityChanged` |
| Stolen paired phone | Owner revokes on the desktop; revocation kicks live sessions and rejects the key | `e2e::revoked_controller_is_disconnected_and_rejected`, interop revoke |
| Replay of a request | TLS 1.3 records are replay-protected; request ids deduplicated per session | `e2e::duplicate_request_ids_are_rejected` |
| Oversized / malformed payloads | 1 MiB frame cap checked before allocation; closed message enum; range validation | `frame` tests, `e2e::oversized_frame_closes_session` |
| Pairing spam / brute force | 6 attempts/IP/10 min, 4 pending max, 120 s timeout, owner must accept | rate limiter code |
| Web page attacks the settings UI (CSRF, DNS rebinding) | Loopback bind, per-install 192-bit token header, `Host` must be `127.0.0.1:<port>`/`localhost:<port>`; strict CSP; all remote strings rendered with `textContent` | manual (`curl` 403 checks in PROGRESS.md) |
| Internet exposure | Default deny for non-private source IPs; no UPnP/port forwarding; Windows firewall rule scoped to the Private profile | `net_policy` tests |
| Over-privileged agent | Per-user process; high-risk scopes opt-in per controller; no shell endpoint | design |
| Paired phone kills critical/other users' processes | `process.terminate` scope (high-risk, never pre-ticked); agent refuses itself, PIDs ≤ 4, a critical list and every process not owned by the agent's user; never elevates; name must match PID | `processes::tests`, `e2e::process_list_and_terminate_with_safeguards`, manual lsass check |
| Window-handle reuse closes the wrong window | `app.close` re-checks that the HWND still exists, belongs to the given PID and is an app window | `apps::tests::rejects_stale_handles` |
| Key theft from disk | DPAPI on Windows; Keystore on Android; Linux 0600 (TPM/Secret Service planned) | `protect` tests |
| Log leakage | Logs carry short fingerprints, action names and outcomes only; no tokens, keys or payloads | review |

## Residual risks (known)
* Linux agent private key is not hardware-protected yet.
* A malicious process running as the same desktop user can read the admin token and approve
  pairings. That attacker already has the user's privileges.
* Windows 10 reached end of support on 2025-10-14. The agent runs there, but the OS no longer
  receives security updates without ESU.
