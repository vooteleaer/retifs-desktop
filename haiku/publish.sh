#!/bin/sh
# Publishes Haiku packages in a retifs library as a pkgman repository (in
# the library, or a folder of it): the packages in packages/, the
# repository file (repo) and its checksum (repo.sha256) next to them. Every
# Haiku device has its libraries mounted in ~/retifs (retifsd mounts them);
# one adds the repository once with
#
#     pkgman add-repo file:///boot/home/retifs/<library>[/<folder>]
#
# and from then on gets new versions with pkgman update (or HaikuDepot),
# fetched over Reticulum like any file in the library. A package can also
# be installed by opening it in Tracker.
#
#     haiku/publish.sh LIBRARY[/FOLDER] PACKAGE.hpkg...
#
# A package replaces the older versions of itself there.
set -e
WHERE=$1
shift || true
[ -n "$WHERE" ] && [ $# -gt 0 ] || { echo "usage: $0 LIBRARY[/FOLDER] PACKAGE.hpkg..." >&2; exit 2; }
LIB=${WHERE%%/*}
TOP=$HOME/retifs/$LIB
DIR=$HOME/retifs/$WHERE
# A volume of its own, not a folder on this disk: mounted, or made (a new
# library begins with its first record) and mounted.
mounted() {
	[ -d "$TOP" ] && [ "$(stat -c %d "$TOP")" != "$(stat -c %d "$(dirname "$TOP")")" ]
}
if ! mounted; then
	if ! retifs query "$LIB" > /dev/null 2>&1; then
		R=$(mktemp)
		printf '%s\n' "retifs installers: Haiku packages (a pkgman repository) and" \
			"other systems' installers. On Haiku:" \
			"    pkgman add-repo file://$TOP/haiku" > "$R"
		retifs rec add "$LIB" path=/README.txt type=text/plain --file "$R" > /dev/null
		rm -f "$R"
	fi
	retifs mount "$LIB" > /dev/null
	mounted || { echo "$LIB isn't mounted at $TOP" >&2; exit 1; }
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
name $(echo "$WHERE" | tr '/' '-')
vendor "retifs"
summary "Haiku packages in the retifs library $WHERE"
priority 2
baseurl file://$DIR
identifier "tag:retifs,2026:repositories/$WHERE/x86_64"
architecture x86_64
EOF
package_repo create -q "$T/repo.info" "$DIR"/packages/*.hpkg
sha256sum "$T/repo" | cut -d' ' -f1 > "$T/repo.sha256"
for f in repo.info repo repo.sha256; do
	cp "$T/$f" "$DIR/$f"
done
rm -rf "$T"
echo "== $WHERE: $(ls "$DIR/packages" | wc -l) package(s); on another device:"
echo "   pkgman add-repo file://$DIR"
