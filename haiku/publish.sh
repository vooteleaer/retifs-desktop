#!/bin/sh
# Publishes Haiku packages in a retifs library that is a pkgman repository:
# the packages in packages/, the repository file (repo) and its checksum
# (repo.sha256) next to them. Every device that has the library mounted (in
# ~/retifs, as the retifs package does at login) adds it once with
#
#     pkgman add-repo file:///boot/home/retifs/<library>
#
# and from then on gets new versions with pkgman update (or HaikuDepot),
# fetched over Reticulum like any file in the library.
#
#     haiku/publish.sh LIBRARY PACKAGE.hpkg...
#
# A package replaces the older versions of itself in the library.
set -e
LIB=$1
shift || true
[ -n "$LIB" ] && [ $# -gt 0 ] || { echo "usage: $0 LIBRARY PACKAGE.hpkg..." >&2; exit 2; }
DIR=$HOME/retifs/$LIB
# A volume of its own, not a folder on this disk: mounted, or made (a new
# library begins with its first record) and mounted.
mounted() {
	[ -d "$DIR" ] && [ "$(stat -c %d "$DIR")" != "$(stat -c %d "$(dirname "$DIR")")" ]
}
if ! mounted; then
	retifs query "$LIB" > /dev/null 2>&1 \
		|| retifs rec add "$LIB" type=inode/directory path=/packages > /dev/null
	retifs mount "$LIB" > /dev/null
	mounted || { echo "$LIB isn't mounted at $DIR" >&2; exit 1; }
fi
mkdir -p "$DIR/packages"

for p in "$@"; do
	f=$(basename "$p")
	name=$(package list -i "$p" | sed -n 's/^\tname:[[:space:]]*//p' | head -1)
	[ -n "$name" ] || { echo "$p: not a package" >&2; exit 1; }
	# Older versions of it go (name-version-revision-arch.hpkg).
	for old in "$DIR/packages/$name"-[0-9]*.hpkg; do
		[ -e "$old" ] && [ "$(basename "$old")" != "$f" ] && rm -f "$old"
	done
	cp "$p" "$DIR/packages/$f"
	echo "  $f"
done

# Made in a folder of its own (package_repo writes repo next to repo.info)
# and copied in: one new version of each file in the library.
T=$(mktemp -d)
cat > "$T/repo.info" <<EOF
name $LIB
vendor "retifs"
summary "Haiku packages in the retifs library $LIB"
priority 2
baseurl file://$DIR
identifier "tag:retifs,2026:repositories/$LIB/x86_64"
architecture x86_64
EOF
package_repo create -q "$T/repo.info" "$DIR"/packages/*.hpkg
sha256sum "$T/repo" | cut -d' ' -f1 > "$T/repo.sha256"
for f in repo.info repo repo.sha256; do
	cp "$T/$f" "$DIR/$f"
done
rm -rf "$T"
echo "== $LIB: $(ls "$DIR/packages" | wc -l) package(s); on another device:"
echo "   pkgman add-repo file://$DIR"
