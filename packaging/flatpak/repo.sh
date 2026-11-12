#!/bin/bash
# Puts the retifs Flatpaks into one Flatpak repository (OSTree, archive mode),
# a folder that can live in a retifs library (retifs-installers/flatpak).
# Every Linux device has its libraries in ~/retifs (retifsd mounts them); one
# adds the repository once with
#
#     flatpak remote-add --user --no-gpg-verify retifs file://$HOME/retifs/retifs-installers/flatpak
#
# and installs from it (flatpak install retifs org.retifs.Photos), and from
# then on gets new versions with flatpak update (or the desktop's software
# app), fetched over Reticulum like any file in the library. No GPG key: who
# may write the library decides what's in it. The runtime (org.gnome.Platform)
# comes from Flathub.
#
#     packaging/flatpak/repo.sh REPO [APP_ID...]
#
# Takes each app's newest build (and its translations, if any) from
# build.sh's own repository ($WORK/<app id>/repo, default
# ~/.cache/retifs-flatpak), every app id built there when none is named; the
# branch is the builds' own (master). A build already in REPO isn't added
# again.
set -euo pipefail
REPO=${1:?usage: $0 REPO [APP_ID...]}
shift
WORK=${WORK:-$HOME/.cache/retifs-flatpak}
IDS=${*:-$(ls "$WORK")}
ARCH=${ARCH:-$(flatpak --default-arch)}

[ -d "$REPO/objects" ] || ostree init --mode=archive-z2 --repo="$REPO"
for ref in $(for id in $IDS; do ostree refs --repo="$WORK/$id/repo" | sed "s|^|$id |"; done | grep -E " (app/[^/]*|runtime/[^/]*\.Locale)/$ARCH/" | tr ' ' '='); do
    id=${ref%%=*}
    REF=${ref#*=}
    SRC=$WORK/$id/repo
    NEW=$(ostree rev-parse --repo="$SRC" "$REF")
    OLD=$(ostree rev-parse --repo="$REPO" "$REF" 2>/dev/null || true)
    # The same content gets the same tree checksum; a commit differs anyway
    # (its time), so compare what's in it.
    if [ -n "$OLD" ] && [ "$(ostree show --repo="$SRC" --print-metadata-key=xa.metadata "$NEW" 2>/dev/null; ostree ls --repo="$SRC" -C -R "$NEW" | md5sum)" = \
                         "$(ostree show --repo="$REPO" --print-metadata-key=xa.metadata "$OLD" 2>/dev/null; ostree ls --repo="$REPO" -C -R "$OLD" | md5sum)" ]; then
        echo "$REF: as before"
        continue
    fi
    flatpak build-commit-from --src-repo="$SRC" "$REPO" "$REF"
    echo "$REF: $(ostree rev-parse --repo="$REPO" "$REF" | cut -c1-12)"
done
# The summary and the appstream data the software apps list.
flatpak build-update-repo --title=retifs --prune --prune-depth=2 "$REPO"
