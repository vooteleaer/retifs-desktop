# retifs-desktop

A desktop management utility for [retifs](https://github.com/vooteleaer/retifs), in the spirit of the Nextcloud desktop client: a tray icon and a window to see and manage what retifs holds, where, and who shares it. Native widgets on each platform over a shared Rust core on `libretifs`: GTK 4 / libadwaita on Linux first, Windows and Haiku later.

It manages retifs (this node, storage, namespaces, sharing, devices); browsing content is left to apps.

Status: milestones 1 and 2 on Linux (overview; each namespace's members, sharing address, what this device keeps). Plan: [docs/plan.md](docs/plan.md).

## Build and run (Linux)
Needs GTK 4 and libadwaita development files (`libgtk-4-dev libadwaita-1-dev` on Debian) and a running `retifsd`.

    cargo run -p retifs-desktop-gtk

It talks to the daemon of the default store (`$RETIFS_STORE`, else the platform default). `core/` builds and tests everywhere (`cargo test`).
