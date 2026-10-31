#!/bin/sh
# Makes the retifs package for Haiku (an hpkg) from the app folder
# build.sh made: the app with its library and the programs it manages, the
# retifs command on the PATH, retifs and Reticulum as launch_daemon jobs at
# login, and an entry in the Deskbar's Applications menu.
#
#     haiku/build.sh && haiku/package.sh
#     pkgman install haiku/build/retifs-<version>-x86_64.hpkg
#
# A system package: the launch jobs name the programs at their place under
# /boot/system. On Haiku, with prnsd built in prnsd-haiku and retifsd in retifs.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
APP=$HERE/build/retifs
STAGE=$HERE/build/package
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)-1
ARCH=x86_64

for f in retifs lib/libretifs_desktop.so bin/prnsd bin/retifsd bin/retifs; do
	[ -f "$APP/$f" ] || { echo "missing $APP/$f: build it first" >&2; exit 1; }
done

rm -rf "${STAGE:?}"
mkdir -p "$STAGE/apps" "$STAGE/bin" "$STAGE/data/user_launch" \
	"$STAGE/data/deskbar/menu/Applications"
cp -a "$APP" "$STAGE/apps/retifs"
ln -s ../apps/retifs/bin/retifs "$STAGE/bin/retifs"
ln -s ../../../../apps/retifs/retifs "$STAGE/data/deskbar/menu/Applications/retifs"

# The jobs, with the package's paths (launch/retifs is the template).
{
	echo "# retifs (package): Reticulum (the bundled prnsd) and the retifs daemon,"
	echo "# started for every user at login."
	sed 's|APP|/boot/system/apps/retifs|g' "$HERE/launch/retifs" | grep -v '^#'
} > "$STAGE/data/user_launch/retifs"
cat >> "$STAGE/data/user_launch/retifs" <<'EOF'

# The status icon in the Deskbar's tray (added once; the Deskbar keeps it).
job x-vnd.retifs-deskbar {
	launch /boot/system/apps/retifs/retifs --deskbar
}
EOF

cat > "$STAGE/.PackageInfo" <<EOF
name			retifs
version			$VERSION
architecture	$ARCH
summary			"Distributed, always-encrypted data over Reticulum"
description		"retifs keeps records and libraries instead of files, synced between your \
devices and others' over Reticulum, encrypted end to end. A working prototype: \
the formats still change, keep no data in it you can't lose.

Includes the retifs daemon and command, the status and settings utility with \
its Deskbar icon, and prnsd (Prns, MIT or Apache-2.0) for Reticulum. They \
start at login as launch_daemon jobs; the store is ~/config/var/retifs."
packager		"retifs <https://github.com/vooteleaer/retifs>"
vendor			"retifs"
licenses {
	"MIT"
}
copyrights {
	"2026 retifs contributors"
}
provides {
	retifs = $VERSION
	app:retifs = $VERSION
	cmd:retifs
}
requires {
	haiku
	lib:libgcc_s
	lib:libstdc++
}
urls {
	"https://github.com/vooteleaer/retifs"
}
EOF

PKG=$HERE/build/retifs-$VERSION-$ARCH.hpkg
rm -f "$PKG"
cd "$STAGE"
package create -C "$STAGE" "$PKG"
echo "== $PKG"
package list "$PKG" | head -30
