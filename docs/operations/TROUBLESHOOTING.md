# Troubleshooting

## Phone can't find the computer
* Both devices must be on the same LAN/VPN. Guest Wi-Fi and "AP/client isolation" block it.
* Android 17+: grant **Local network access** (the app asks; Settings → Apps → ccontrol → Permissions).
* mDNS is often blocked on corporate/mesh networks. Scan the QR code or enter the IP shown on
  the settings page instead.
* Windows Firewall: allow `ccontrol-agent.exe` on **Private** networks (the installer can add the
  rule when run as admin). Make sure the Wi-Fi profile is Private, not Public.
* Linux firewalls: `sudo ufw allow from 192.168.0.0/16 to any port 47800 proto tcp` (adapt the range).
  firewalld: `sudo firewall-cmd --zone=home --add-port=47800/tcp --permanent && sudo firewall-cmd --reload`.

## "Identity changed"
A different machine (or a reinstalled agent with a new identity) answers at the remembered
address. If you reset or reinstalled the agent on purpose, choose *Forget* on the phone and pair again.

## "Permission required"
The computer revoked this phone or was reset. Pair again; the owner must approve.

## Pairing request never appears
* Open the settings page (`ccontrol-agent ui`). Requests expire after 120 s.
* More than 6 attempts in 10 minutes from one IP are rate-limited. Wait, or restart the agent.

## Volume shows "Unavailable"
* Windows: no default playback device, or the agent runs outside your desktop session (e.g. as a
  service). Run it as your user.
* Linux: install `wireplumber` (`wpctl`) or `pulseaudio-utils` (`pactl`). Headless sessions have no
  user audio server.

## Media keys do nothing
* Windows: `SendInput` cannot reach elevated windows or the secure desktop (UAC, lock screen).
* Linux: needs a running MPRIS player (Spotify, Firefox, VLC…) on the session bus.

## Wayland
Remote input and screen capture are not implemented. They will require the portal prompts
(RemoteDesktop/ScreenCast) and will never bypass them.

## SELinux (Fedora)
The agent is a user process that binds an unprivileged port, so the default targeted policy
allows it. If AVC denials appear, check `ausearch -m avc -c ccontrol-agent` and file an issue.

## Logs
`<data dir>/logs/agent.*.jsonl` (7 days kept). The data dir is `%APPDATA%\ccontrol\ccontrol\data\agent`
on Windows and `~/.local/share/ccontrol/agent` on Linux. Logs contain no keys, tokens or
notification contents. Set `CCONTROL_LOG=debug` for more detail.
