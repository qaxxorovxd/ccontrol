# Device pairing and trust lifecycle

## Identities

* Agent: ECDSA P-256 key + self-signed certificate created on first run, stored in the agent
  data directory. The private key is sealed with DPAPI (CurrentUser) on Windows; on Linux it is
  a 0600 file (Secret Service/TPM sealing is a planned hardening).
* Phone: ECDSA P-256 key generated **inside Android Keystore** (non-exportable) with a
  Keystore-issued self-signed certificate.
* Device ID = `hex(SHA-256(SubjectPublicKeyInfo DER))`. Short form `XXXX-XXXX-XXXX-XXXX`.

## First pairing

```mermaid
sequenceDiagram
  participant P as Phone
  participant A as Agent
  participant O as Desktop owner (settings page)
  P->>A: TLS 1.3 handshake (both certificates; QR pins agent fp)
  P->>A: hello(device_id, peer_device_id)
  A->>P: hello_ack(paired=false)
  P->>A: pair_commit(H("ccontrol-commit-v1"||Nc), requested_scopes)
  A->>P: pair_nonce(Ns)
  P->>A: pair_reveal(Nc)
  Note over P,A: both compute SAS = H("ccontrol-sas-v1"||fpA||fpP||Nc||Ns) mod 10^6
  A->>O: pending request: name, IP, fp, SAS, scopes
  A->>P: pair_awaiting_approval(120)
  O->>A: Accept (selected scopes ⊆ requested) / Reject
  A->>P: pair_result(accepted, granted_scopes)
  A->>P: hello_ack(paired=true, manifest)
```

Why the commitment: a MITM must present its own certificates, so the two SAS values differ
unless it can choose nonces to make them collide. The phone commits before seeing `Ns`,
and the agent fixes `Ns` before seeing `Nc`, so an attacker gets one 10⁻⁶ guess per attempt.
Attempts are limited to 6 per source IP per 10 minutes, at most 4 pending requests, each
requiring the owner to click Accept.

**QR path:** the agent's settings page shows `ccontrol://pair?h=<ip>&p=<port>&fp=<agent fp>&n=<name>`.
The phone pins `fp` for the handshake, so a different machine at that address fails before
any pairing message. The SAS and owner approval still apply.

High-risk scopes (`input.control`, `files.write`, `process.terminate`, `power.control`,
`commands.run`, `admin.action`) are never pre-ticked on the approval form.

## Subsequent connections

* The phone stores the agent fingerprint, last host and port. It reconnects with the
  fingerprint pinned, trying the mDNS-discovered address first, then the last known one.
  IP changes are therefore harmless; a different machine at the old IP is reported as
  **Identity changed** and never trusted.
* The agent looks the phone up by its certificate fingerprint. No QR or SAS is needed again.

## Revocation and reset

| Action | Effect |
|---|---|
| Revoke on the desktop (settings page or `ccontrol-agent revoke <id-prefix>`) | Trust row deleted; live sessions receive `event: revoked` and close; later connects get `hello_ack(paired=false)` and the phone shows **Permission required**. |
| Forget on the phone | Inventory entry removed, no reconnects. The agent keeps its trust row until revoked there (the UI says so). |
| Reset agent identity (delete `identity.*` in the data dir) | Every phone sees **Identity changed** and must pair again. |
| Scope change on the desktop | Applies to the next request immediately. |
