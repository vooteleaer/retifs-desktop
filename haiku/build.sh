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
g++ -O2 -std=c++17 -Wall -o "$OUT/retifs" "$HERE/app/RetifsApp.cpp" \
	-I "$HERE/capi" -I /boot/system/develop/headers/private/shared \
	-L "$OUT/lib" -lretifs_desktop -lbe -lshared
# The programs it manages, in its bin/ folder as a package has them (where they
# are built: retifs next to retifs-desktop, Prns for Haiku next to it).
for p in "$ROOT/../prnsd-haiku/prnsd/target/release/prnsd" \
		"$ROOT/../retifs/target/release/retifsd" "$ROOT/../retifs/target/release/retifs"; do
	if [ -f "$p" ]; then
		mkdir -p "$OUT/bin"
		rm -f "${OUT:?}/bin/$(basename "$p")"	# a running one keeps its old file
		cp "$p" "$OUT/bin/"
	fi
done
echo "== $OUT"
ls -la "$OUT" "$OUT/lib" "$OUT/bin"
