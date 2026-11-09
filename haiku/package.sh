#!/bin/sh
# Makes the retifs package for Haiku (an hpkg) from the app folder
# build.sh made: the app with its library, retifsd and the retifs command
# (on the PATH), retifsd as a launch_daemon job at login, the userlandfs
# module that makes each library a volume (retifsd mounts every library in
# ~/retifs once the desktop is up), and an entry in the Deskbar's
# Applications menu. Not mounted by a login job: a userlandfs mount from one
# blocked the user's launch_daemon, and the desktop never came up. Reticulum is the Prns package's (prnsd
# with its own app and Deskbar icon), which this one requires.
#
#     haiku/build.sh && haiku/package.sh
#     pkgman install haiku/build/retifs-<version>-x86_64.hpkg
#
# A system package: the launch jobs name the programs at their place under
# /boot/system. On Haiku, with retifsd built in retifs.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
APP=$HERE/build/retifs
STAGE=$HERE/build/package
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)-19
ARCH=x86_64

for f in retifs lib/libretifs_desktop.so bin/retifsd bin/retifs add-ons/retifs-haikufs; do
	[ -f "$APP/$f" ] || { echo "missing $APP/$f: build it first" >&2; exit 1; }
done

rm -rf "${STAGE:?}"
mkdir -p "$STAGE/apps" "$STAGE/bin" "$STAGE/data/user_launch" \
	"$STAGE/data/deskbar/menu/Applications" "$STAGE/add-ons/userlandfs"
cp -a "$APP" "$STAGE/apps/retifs"
# The module where userlandfs looks for it, by the name mount -t userlandfs takes.
mv "$STAGE/apps/retifs/add-ons/retifs-haikufs" "$STAGE/add-ons/userlandfs/retifs"
rmdir "$STAGE/apps/retifs/add-ons"
# A prnsd from an older build folder: Prns's package has it now.
rm -f "$STAGE/apps/retifs/bin/prnsd"
ln -s ../apps/retifs/bin/retifs "$STAGE/bin/retifs"
ln -s ../../../../apps/retifs/retifs "$STAGE/data/deskbar/menu/Applications/retifs"

# The jobs, with the package's paths (launch/retifs is the template).
{
	echo "# retifs (package): the retifs daemon, started for every user at login"
	echo "# after Reticulum (the Prns package's prnsd)."
	sed 's|APP|/boot/system/apps/retifs|g' "$HERE/launch/retifs" | grep -v '^#'
} > "$STAGE/data/user_launch/retifs"
cat >> "$STAGE/data/user_launch/retifs" <<'EOF'

# The status icon in the Deskbar's tray (added once; the Deskbar keeps it):
# with the desktop, once the Deskbar runs. At top level it started before
# app_server, and a login sometimes never got its desktop (2026-11-09).
target desktop {
	job x-vnd.retifs-deskbar {
		launch /boot/system/apps/retifs/retifs --deskbar
		requires x-vnd.Be-TSKB
	}
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

Includes the retifs daemon and command, and the status and settings utility \
with its Deskbar icon. retifs starts at login as a launch_daemon job, on the \
Reticulum of the Prns package (prnsd); the store is ~/config/var/retifs."
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
	prns
	userland_fs
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
