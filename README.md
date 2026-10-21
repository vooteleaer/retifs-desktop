# retifs-desktop

A desktop management utility for [retifs](https://github.com/vooteleaer/retifs), in the spirit of the Nextcloud desktop client: a tray icon and a window to see and manage what retifs holds, where, and who shares it. Built with Tauri on `libretifs`; Linux first, Windows too.

Status: planning. See [docs/plan.md](docs/plan.md).

## Run (development)
Needs a running `retifsd` (see the retifs README) and Node with npm.

    npm install
    npm run dev

It talks to the daemon of the default store (`$RETIFS_STORE`, else the platform default). Closing the window keeps it in the tray; quit from the tray menu.
