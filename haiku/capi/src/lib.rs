//! retifs-desktop for Haiku: the shared core to C (and so to the C++ BeAPI
//! front end), every answer a JSON string from `core::api`. The same calls
//! as the other front ends; no second implementation of the daemon's
//! protocol in C++.
//!
//! Every returned string is the caller's to free with `rd_free`. Actions
//! answer "" when they went well, else what went wrong. Calls block: the
//! front end makes them off its window threads.

use std::ffi::{CStr, CString, c_char, c_int};

use retifs_desktop_core::{api, device};

fn out(s: impl Into<String>) -> *mut c_char {
    let s = s.into().replace('\0', " ");
    CString::new(s).expect("no NUL left").into_raw()
}

fn result(r: Result<(), String>) -> *mut c_char {
    out(r.err().unwrap_or_default())
}

/// # Safety
/// `s` is null or a NUL-terminated string.
unsafe fn arg(s: *const c_char) -> String {
    if s.is_null() { String::new() } else { unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned() }
}

/// Where the app is installed (its folder, with `bin/retifsd`), for when the
/// program running isn't the app: the Deskbar replicant. Call it first.
///
/// # Safety
/// `dir` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rd_set_app_dir(dir: *const c_char) {
    device::set_install_dir(unsafe { arg(dir) }.into());
}

/// The daemon: up or not, the node, storage, libraries (JSON).
#[unsafe(no_mangle)]
pub extern "C" fn rd_status() -> *mut c_char {
    out(api::status().to_string())
}

/// Public libraries heard on the network (JSON array).
#[unsafe(no_mangle)]
pub extern "C" fn rd_heard() -> *mut c_char {
    out(api::heard().to_string())
}

/// Reticulum and its connections (JSON).
#[unsafe(no_mangle)]
pub extern "C" fn rd_reticulum() -> *mut c_char {
    out(api::reticulum().to_string())
}

/// The settings offered, with their values (JSON array).
#[unsafe(no_mangle)]
pub extern "C" fn rd_settings() -> *mut c_char {
    out(api::settings().to_string())
}

/// The services retifs runs as, with their states (JSON array).
#[unsafe(no_mangle)]
pub extern "C" fn rd_services() -> *mut c_char {
    out(api::services().to_string())
}

/// # Safety
/// `address` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rd_join(address: *const c_char) -> *mut c_char {
    result(api::join(&unsafe { arg(address) }).map(|_| ()))
}

/// # Safety
/// `link` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rd_open_link(link: *const c_char) -> *mut c_char {
    result(api::open_link(&unsafe { arg(link) }).map(|_| ()))
}

/// # Safety
/// `ns` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rd_leave(ns: *const c_char) -> *mut c_char {
    result(api::leave(&unsafe { arg(ns) }))
}

/// # Safety
/// `name` and `host` are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rd_add_tcp(name: *const c_char, host: *const c_char, port: c_int) -> *mut c_char {
    let (name, host) = unsafe { (arg(name), arg(host)) };
    result(u16::try_from(port).map_err(|_| "a port is 1 to 65535".to_string()).and_then(|p| device::add_tcp(&name, &host, p)))
}

/// # Safety
/// `name` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rd_set_interface(name: *const c_char, enabled: c_int) -> *mut c_char {
    result(device::set_interface(&unsafe { arg(name) }, enabled != 0))
}

/// # Safety
/// `name` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rd_remove_interface(name: *const c_char) -> *mut c_char {
    result(device::remove_interface(&unsafe { arg(name) }))
}

/// Saves settings: `{"section.key": "typed value" | null}`.
///
/// # Safety
/// `values` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rd_save_settings(values: *const c_char) -> *mut c_char {
    result(api::save_settings(&unsafe { arg(values) }))
}

/// `start`, `stop` or `restart` a service by name.
///
/// # Safety
/// `name` and `action` are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rd_service(name: *const c_char, action: *const c_char) -> *mut c_char {
    let (name, action) = unsafe { (arg(name), arg(action)) };
    result(device::service(&name, &action))
}

/// What the tray (Deskbar) icon shows: `{"health": "good"|"partly"|"down", "lines": [...]}`.
#[unsafe(no_mangle)]
pub extern "C" fn rd_tray() -> *mut c_char {
    out(api::tray().to_string())
}

/// Draws the status icon, `size` × `size` ARGB pixels (alpha in the top
/// byte), into `pixels`; `health` is "good", "partly" or "down".
///
/// # Safety
/// `health` is a NUL-terminated string; `pixels` has room for size × size u32.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rd_icon(size: c_int, health: *const c_char, pixels: *mut u32) {
    use retifs_desktop_core::status_icon::{Health, draw};
    let Ok(size) = u32::try_from(size) else { return };
    let h = match unsafe { arg(health) }.as_str() {
        "good" => Health::Good,
        "partly" => Health::Partly,
        _ => Health::Down,
    };
    let icon = draw(size, h);
    unsafe { std::ptr::copy_nonoverlapping(icon.as_ptr(), pixels, icon.len()) };
}

/// Frees a string this library returned.
///
/// # Safety
/// `s` came from this library and isn't used afterwards (or is null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rd_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}
