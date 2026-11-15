#!/usr/bin/env bash
# Build ccontrol-amd64.deb from a release agent binary. Usage: build-deb.sh <agent-binary> <version> <out-dir>
set -euo pipefail
BIN=$1; VERSION=$2; OUT=$3
ROOT=$(mktemp -d)
here=$(cd "$(dirname "$0")" && pwd)
install -Dm755 "$BIN" "$ROOT/usr/bin/ccontrol-agent"
install -Dm644 "$here/../linux/ccontrol-agent.service" "$ROOT/usr/lib/systemd/user/ccontrol-agent.service"
install -Dm644 "$here/../linux/ccontrol-settings.desktop" "$ROOT/usr/share/applications/ccontrol-settings.desktop"
install -Dm644 "$here/../linux/ccontrol.svg" "$ROOT/usr/share/icons/hicolor/scalable/apps/ccontrol.svg"
install -Dm644 "$here/copyright" "$ROOT/usr/share/doc/ccontrol/copyright"
mkdir -p "$ROOT/DEBIAN"
SIZE=$(du -sk "$ROOT/usr" | cut -f1)
sed -e "s/@VERSION@/$VERSION/" -e "s/@SIZE@/$SIZE/" "$here/control.in" > "$ROOT/DEBIAN/control"
install -m755 "$here/postinst" "$ROOT/DEBIAN/postinst"
install -m755 "$here/prerm" "$ROOT/DEBIAN/prerm"
mkdir -p "$OUT"
dpkg-deb --root-owner-group --build "$ROOT" "$OUT/ccontrol-amd64.deb"
rm -rf "$ROOT"
