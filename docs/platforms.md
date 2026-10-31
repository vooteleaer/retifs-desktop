# retifs on every platform

One retifs (the same Rust daemon, `retifsd`, and the same transport, Prns) and one shared core (`core/`), with a native front end per platform (the user's choice, 2026-10-21: no webview). This is where each platform stands and what's next (2026-10-30).

| | Daemon | Reticulum | Utility | Files | Packaging |
|---|---|---|---|---|---|
| **Linux** | runs (amd64, arm64) | bundled prnsd, or the machine's | GTK 4 / libadwaita, tray (StatusNotifierItem) | FUSE mount | .deb (retifs amd64+arm64, utility amd64) |
| **Windows** | runs | bundled prnsd, or the user's | Win32, notification area | Cloud Files | retifs-setup.exe (per user) |
| **Android** | runs (emulator, x86_64; aarch64 builds) | bundled prnsd, TCP shared instance | Kotlin + Compose, foreground service | — (DocumentsProvider planned) | debug APK (`android/build-apk.ps1`) |
| **Haiku** | runs (x86_64, native build) | prnsd runs (the prnsd-haiku fork) | Haiku API (BeAPI) window; Deskbar replicant planned | userlandfs module (retifs-haikufs): attributes, queries | app folder (`haiku/build.sh`); hpkg planned |

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
**Runs (2026-10-30)**, in an Android 14 emulator (x86_64) with the app from `android/`: the foreground service starts the bundled prnsd and retifsd; a TCP connection added in the app connects to the testbed; the testbed's public libraries are heard, and Join adds one, fetched as needed. `retifsd`, `retifs` and `prnsd` (feature `tokio-cloud-host`: TCP, no serial or Bluetooth) build unchanged with NDK 27 for `aarch64-linux-android` and `x86_64-linux-android` (API 26). Not yet on a real phone.

What it took:
- **The shared instance on TCP:** the app writes prnsd's first config with `shared_instance_type = tcp` (port 37428). On Android prnsd listens on the abstract socket by default while Prns clients look for TCP (to report to Prns).
- **prnsd run by the app** (`prnsd run`) can't apply interface changes live (`--apply` needs a managed daemon): the core restarts it instead (`RETIFS_PRNSD_APPLY=restart`), and retifsd with it, as a Prns client doesn't attach to a new shared instance by itself (to report to Prns).
- **Phone defaults**, set once and changeable in Settings: nothing held for others, 1 GB kept free (the desktop default, max(10 %, 5 GB), left a phone with less free space no room at all), the device named after the model.
- **On the other end:** a prnsd interface a phone connects to needs `--ingress-control false`, or the phone's announces are held back.

Next: an arm64 build on a real phone; Find and libraries' contents; the DocumentsProvider; battery (below).

The plan it follows:

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
- **retifsd runs (2026-10-30)** on Haiku nightly hrev60299 (x86_64, a QEMU VM): built natively with HaikuPorts' Rust 1.94.1 and gcc 13 (`--ignore-rust-version`: Prns asks for 1.96 but builds), with prns-haiku's keepalive patch and no change to retifs. With a shared instance reached over a tunnel it attached, heard the testbed's public libraries, joined one lazily, and held 1.2 GB that other nodes placed on it within 20 minutes: the store (redb, slabs, file locking, mmap), tokio and mio, and Reticulum links in both directions work. Not yet: files in Tracker (userlandfs, the Haiku API) and prnsd itself.
- **prnsd runs on Haiku (2026-10-30)** from a local fork of Prns, `prnsd-haiku`: branch `haiku` on the Prns release, a few small commits each behind `cfg(target_os = "haiku")`, meant for upstream (`HAIKU.md` there). mDNS LAN discovery left out (no `IP_PKTINFO` on Haiku); if-addrs and mio patched (mio assumed an accepted socket inherits non-blocking mode on Haiku, it does not, and the runtime stopped at the first local client); lifecycle locks through `flock`. A Haiku node now runs on its own: prnsd connected to the testbed, retifsd on it, a library heard, joined and searched. Next: RNode over USB serial. retifsd on Haiku needs the same mio fix (its local API), until mio upstream has it.
- **Files in Tracker (2026-10-31):** retifs-haikufs (in retifs) mounts a namespace as a volume through userlandfs: records as files, their attributes as Haiku attributes (Tracker columns by MIME type), queries in Haiku's language including live ones (Tracker's Find). Read-only so far.
- **The utility (2026-10-31)**, `haiku/`: a Haiku API window over the shared core through a C library (`haiku/capi`, JSON in and out, the same calls as Android's JNI layer, now `core::api`). This device, Network and Settings work against retifsd and prnsd running as launch_daemon jobs (`x-vnd.retifs-daemon`, `x-vnd.retifs-prnsd`, started and stopped with `launch_roster`). The store is `~/config/var/retifs`. Next: the Deskbar replicant (the tray), an hpkg.
- **Files: userlandfs**, natively (ADR-0002): records with their attributes as Haiku attributes, indexes and live queries in Tracker. Haiku is where retifs's model fits best: BeFS already has records with attributes and queries.
- **Utility:** a BeAPI app (C++) over the core through a C interface (cbindgen), or Rust bindings to the Haiku API; a Deskbar replicant for the state.
- **Test machine:** a Haiku nightly VM in QEMU (KVM, in WSL here), driven headless; real hardware later.
