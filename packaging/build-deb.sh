#!/bin/bash
# Builds retifs-desktop_<version>_amd64.deb: the GTK 4 / libadwaita utility.
# GTK 4.12 and libadwaita 1.4 or newer: Debian 13, Ubuntu 24.04. Built on
# such a machine (it links the system's GTK); needs the retifs package.
#
#     packaging/build-deb.sh [OUT_DIR]
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
OUT=${1:-$ROOT/target/deb}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
COMMIT=$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)
DATE=$(git -C "$ROOT" log -1 --format=%cd --date=format:%Y%m%d 2>/dev/null || date +%Y%m%d)
DEBVER="$VERSION~git$DATE.$COMMIT"
ARCH=$(dpkg --print-architecture)
mkdir -p "$OUT"

cargo build --release --locked --manifest-path "$ROOT/Cargo.toml" -p retifs-desktop-gtk
BIN=$CARGO_TARGET_DIR/release/retifs-desktop
# The newest glibc symbol it uses: its libc6 dependency.
GLIBC=$(objdump -T "$BIN" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -V | tail -1)

STAGE=$(mktemp -d)
chmod 0755 "$STAGE"
trap 'rm -rf "$STAGE"' EXIT
install -d "$STAGE/DEBIAN" "$STAGE/etc/xdg/autostart" "$STAGE/usr/bin" "$STAGE/usr/share/applications" "$STAGE/usr/share/icons/hicolor/scalable/apps" "$STAGE/usr/share/doc/retifs-desktop"
install -m 0755 "$BIN" "$STAGE/usr/bin/retifs-desktop"
install -m 0644 "$HERE/org.retifs.Desktop.desktop" "$STAGE/usr/share/applications/"
# In the tray from login (a desktop without a tray: nothing shows, nothing runs on).
install -m 0644 "$HERE/org.retifs.Desktop-autostart.desktop" "$STAGE/etc/xdg/autostart/"
install -m 0644 "$HERE/org.retifs.Desktop.svg" "$STAGE/usr/share/icons/hicolor/scalable/apps/"
install -m 0644 "$ROOT/README.md" "$STAGE/usr/share/doc/retifs-desktop/"
SIZE=$(du -sk "$STAGE/usr" | cut -f1)
cat > "$STAGE/DEBIAN/control" <<CTRL
Package: retifs-desktop
Version: $DEBVER
Architecture: $ARCH
Maintainer: retifs <https://github.com/vooteleaer/retifs>
Installed-Size: $SIZE
Depends: retifs, libc6 (>= $GLIBC), libgtk-4-1 (>= 4.12), libadwaita-1-0 (>= 1.4)
Section: net
Priority: optional
Description: retifs utility: this device, its libraries and how they're kept
 The retifs node at a glance, in the tray and in a window: Reticulum and
 its connections, the services, settings, storage, libraries and their
 members, and Find. A prototype.
CTRL
DEB="$OUT/retifs-desktop_${DEBVER}_${ARCH}.deb"
dpkg-deb --root-owner-group -Zxz --build "$STAGE" "$DEB" >/dev/null
echo "== $DEB ($(du -h "$DEB" | cut -f1))"
