#!/usr/bin/env bash
# Build ccontrol-x86_64.AppImage. Usage: build-appimage.sh <agent-binary> <out-dir>
# Requires appimagetool on PATH (CI downloads a pinned release).
set -euo pipefail
BIN=$1; OUT=$2
here=$(cd "$(dirname "$0")" && pwd)
APPDIR=$(mktemp -d)/ccontrol.AppDir
install -Dm755 "$BIN" "$APPDIR/usr/bin/ccontrol-agent"
install -Dm644 "$here/../linux/ccontrol.svg" "$APPDIR/ccontrol.svg"
sed 's/^Exec=.*/Exec=ccontrol-agent ui/' "$here/../linux/ccontrol-settings.desktop" > "$APPDIR/ccontrol.desktop"
cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
HERE=$(dirname "$(readlink -f "$0")")
# No arguments: start the agent (it keeps running after the settings page is closed).
[ $# -eq 0 ] && set -- run
exec "$HERE/usr/bin/ccontrol-agent" "$@"
RUN
chmod +x "$APPDIR/AppRun"
mkdir -p "$OUT"
ARCH=x86_64 appimagetool "$APPDIR" "$OUT/ccontrol-x86_64.AppImage"
