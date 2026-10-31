# retifs for Haiku

The status and settings utility with the Haiku API: this device (retifs,
its storage, libraries, libraries heard on the network), the network
(Reticulum's connections) and settings, as on Linux, Windows and Android.

- `capi/`: the shared core (`../core`) as a C library, JSON in and out
  (`retifs_desktop.h`).
- `app/`: the BeAPI app over it; `BJson` reads the answers into BMessages,
  and the core's blocking calls run in a looper of their own.
- `launch/retifs`: retifs and Reticulum as launch_daemon jobs
  (`x-vnd.retifs-daemon`, `x-vnd.retifs-prnsd`); the utility starts, stops and
  restarts them with `launch_roster`.
- The Deskbar replicant (in the app): the shared status icon in the tray, its
  state from a thread of its own, a menu with the state, Open, Restart and
  Remove. The app adds it when it starts (`retifs --deskbar` only adds it);
  Deskbar keeps it across restarts. The app is linked with `-rdynamic` and
  `$ORIGIN/lib`, so Deskbar can load it and find the core next to it.

## Building, on Haiku

    pkgman install rust_bin gcc haiku_devel
    haiku/build.sh

with retifs next to retifs-desktop, and the Haiku branch of Prns
(`prnsd-haiku`) for prnsd. The result is an app folder,
`haiku/build/retifs`: `retifs` (the app), `lib/libretifs_desktop.so`, and
`bin/` with prnsd, retifsd and the `retifs` command when they are built.

retifsd and prnsd need mio with Haiku's accept fix (tokio-rs/mio#2036) until
mio has it in a release: prnsd-haiku carries it; retifsd is built with
`--config 'patch.crates-io.mio.path="…/prnsd-haiku/third_party/mio"'`.

## The package

    haiku/build.sh && haiku/package.sh
    pkgman install haiku/build/retifs-<version>-x86_64.hpkg

A system package: the app in `/boot/system/apps/retifs` (with `lib/` and `bin/`:
prnsd, retifsd, retifs), the `retifs` command on the PATH, the jobs in
`data/user_launch/retifs` (at every login, and one that adds the Deskbar
icon), and the app in the Deskbar's Applications menu.

The store is `~/config/var/retifs` (libretifs' default on Haiku).
