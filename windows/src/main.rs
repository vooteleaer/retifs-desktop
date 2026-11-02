//! retifs-desktop for Windows: native Win32 controls over the shared core,
//! and an icon in the notification area. It's also what starts retifs at
//! login (`--autostart`): Reticulum where none runs yet, then retifsd.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod libraries;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("retifs-desktop-win is the Windows front end; on Linux use retifs-desktop-gtk");
}
