#!/bin/sh
# Builds retifs for Haiku (the status and settings utility) on Haiku:
# the core as a library (haiku/capi), the BeAPI app over it, put together
# as an app folder (haiku/build/retifs: the app, lib/libretifs_desktop.so).
#
#     haiku/build.sh
#
# Needs: pkgman install rust_bin gcc haiku_devel; retifs next to retifs-desktop.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
OUT=$HERE/build/retifs
cd "$ROOT"
cargo build --release --ignore-rust-version -p retifs-desktop-capi
mkdir -p "$OUT/lib"
# A new file, not the old one overwritten: a running app has it mapped.
rm -f "${OUT:?}/lib/libretifs_desktop.so"
cp target/release/libretifs_desktop.so "$OUT/lib/"
# $ORIGIN/lib: the core is found next to the app also when Deskbar, not the
# app, loads it (the replicant); -rdynamic: Deskbar finds the replicant's
# Instantiate() among the app's exported symbols.
g++ -O2 -std=c++17 -Wall -o "$OUT/retifs" "$HERE/app/RetifsApp.cpp" \
	-I "$HERE/capi" -I /boot/system/develop/headers/private/shared \
	-L "$OUT/lib" -lretifs_desktop -lbe -lshared \
	-Wl,-rpath,'$ORIGIN/lib' -rdynamic
rc -o "$HERE/build/retifs.rsrc" "$HERE/app/retifs.rdef"
xres -o "$OUT/retifs" "$HERE/build/retifs.rsrc"
mimeset -f "$OUT/retifs"
# The programs it manages, in its bin/ folder as a package has them (where they
# are built: retifs next to retifs-desktop). Reticulum is Prns's own package
# (prnsd-haiku's haiku/).
rm -f "${OUT:?}/bin/prnsd"
for p in "$ROOT/../retifs/target/release/retifsd" "$ROOT/../retifs/target/release/retifs"; do
	if [ -f "$p" ]; then
		mkdir -p "$OUT/bin"
		rm -f "${OUT:?}/bin/$(basename "$p")"	# a running one keeps its old file
		cp "$p" "$OUT/bin/"
	fi
done
echo "== $OUT"
ls -la "$OUT" "$OUT/lib" "$OUT/bin"
