# Privacy

* **Local-first.** No cloud account, telemetry upload, analytics or third-party servers. Traffic
  stays between your phone and your computers on your LAN/VPN.
* **What the agent stores** (`agent.db`): paired phone IDs (key fingerprints), names, platform,
  granted scopes, pairing/last-seen times, and an audit log of action names and outcomes
  (≤ 5000 rows). It stores no request payloads, notification contents or audio.
* **What the phone stores** (`devices.json`, app-private, excluded from backup): computer IDs,
  names, last address, capability manifest, scopes. The private key stays in Android Keystore.
* **Logs**: short device IDs, action names, errors. Never keys, tokens, notification text or media.
* **Permissions on Android**: Internet, network state and (Android 17+) local network access.
  Phase 1 requests no notification, microphone, camera or screen-capture permission. QR
  scanning uses Google's code scanner UI, which needs no camera permission for this app.
* **Notifications (future)**: opt-in per app, hidden-content mode, short configurable retention,
  encrypted at rest when retained.
* **Visibility**: anything that streams audio/screen or injects input will show a persistent
  indicator on both devices. Nothing runs covertly.
