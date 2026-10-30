# retifs on every platform

One retifs (the same Rust daemon, `retifsd`, and the same transport, Prns) and one shared core (`core/`), with a native front end per platform (the user's choice, 2026-10-21: no webview). This is where each platform stands and what's next (2026-10-30).

| | Daemon | Reticulum | Utility | Files | Packaging |
|---|---|---|---|---|---|
| **Linux** | runs (amd64, arm64) | bundled prnsd, or the machine's | GTK 4 / libadwaita, tray (StatusNotifierItem) | FUSE mount | .deb (retifs amd64+arm64, utility amd64) |
| **Windows** | runs | bundled prnsd, or the user's | Win32, notification area | Cloud Files | retifs-setup.exe (per user) |
| **Android** | cross-compiles (aarch64) | prnsd cross-compiles (TCP) | — (Kotlin + Compose planned) | — (DocumentsProvider planned) | — (APK planned) |
| **Haiku** | not tried | prns-haiku: cross-checks | — (BeAPI planned) | — (userlandfs planned) | — (hpkg planned) |

## Linux
Done: the `retifs` .deb for amd64 and arm64 (glibc 2.36+: Debian 12, Ubuntu 22.04, Raspberry Pi OS bookworm), prnsd bundled and started only where no Reticulum runs; the utility's .deb for amd64 (GTK 4.12+, libadwaita 1.4+: Debian 13, Ubuntu 24.04), in the tray from login.

Next:
- The utility for arm64 (a Raspberry Pi desktop): the same .deb, cross-built against arm64 GTK.
- Libraries and Find exist only here; Windows gets them next, then Android.
- Other distributions: an .rpm from the same script (Fedora), later an AUR recipe. A Flatpak for the utility only (it needs `--filesystem=xdg-data/retifs` for the daemon's socket, and systemctl through the portal), after the native packages.
- GNOME shows no tray icon without the AppIndicator extension: the utility then works as a window, which it does now.

## Windows
Done: the utility (Win32, Common Controls 6, per-monitor DPI) with Overview, Network and Settings, the notification-area icon, and starting retifs at login; retifs-setup.exe per user, no administrator.

Next: Libraries and Find (as on Linux); a signed installer (SmartScreen warns about unsigned ones); the service state should name the store a running retifsd serves, not just that one runs.

## Android
**Spike (2026-10-30):** `retifsd`, the `retifs` CLI and `prnsd` (feature `tokio-cloud-host`: TCP, no serial or Bluetooth) cross-compile unchanged for `aarch64-linux-android` (API 26) with NDK 27. Not run yet: no device or emulator here.

Plan, one app, "retifs":
- **Kotlin and Jetpack Compose** (Material 3), the platform's own toolkit.
- **A foreground service** runs the daemons as executables shipped as native libraries (`libretifsd.so`, `libprnsd.so` in `jniLibs`, run from `nativeLibraryDir`: since Android 10 an app may not execute files it wrote, but may execute its native libraries). Its notification is Android's tray: the same states, Open and Stop.
- **Reticulum:** the bundled prnsd where no shared instance answers on localhost TCP 37428; if another app (Sideband) runs one, retifs uses it. Prns's Android work (the `wip/android` branch) brings the radios (RNode over USB and Bluetooth) later.
- **The UI talks to the core through JNI:** `core/` built as a library for Android with a thin JNI layer returning JSON (status, Reticulum, settings, libraries, find). The same calls as the GTK and Win32 front ends, no second protocol implementation in Kotlin.
- **A phone is a small node:** storage-less or a small store limit by default (ADR-0031), holding nothing for others unless the user says so; keep rules decide what stays offline.
- **Files:** a `DocumentsProvider` (Storage Access Framework) shows libraries in the Files app and every app's file picker, Android's counterpart of FUSE and Cloud Files. After the utility.
- **Open questions:** battery (Doze ends background networking: sync in windows, or when charging), the foreground service type Android 14+ accepts for this (`dataSync` is time-limited from Android 15; `connectedDevice` or `specialUse`), a multicast lock for Reticulum's AutoInterface on Wi-Fi.
- **Later:** the long-term Android ROM with retifs for everything (separate project).

## Haiku
Haiku's own way, not Linux's: settings in `~/config/settings/retifs`, launch_daemon, hpkg, the Haiku API for UI, attributes and live queries as Haiku does them.

- **Reticulum:** the prns-haiku project (prnsd port, Network preferences add-on, Deskbar replicant). It cross-checks for `x86_64-unknown-haiku` with one patch; not linked or run yet.
- **retifsd:** to check what blocks it: redb (file locking, mmap), tokio and mio on Haiku, zstd's C code with Haiku's cross compiler, passing file descriptors over Unix sockets (SCM_RIGHTS).
- **Files: userlandfs**, natively (ADR-0002): records with their attributes as Haiku attributes, indexes and live queries in Tracker. Haiku is where retifs's model fits best: BeFS already has records with attributes and queries.
- **Utility:** a BeAPI app (C++) over the core through a C interface (cbindgen), or Rust bindings to the Haiku API; a Deskbar replicant for the state.
- **Needs a Haiku machine:** a VM (QEMU or VirtualBox) with a nightly Haiku, or real hardware.
