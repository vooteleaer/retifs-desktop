//! The core as JSON, for front ends in other languages (Kotlin on Android,
//! C++ on Haiku): every call blocks and answers a JSON value or an error
//! string. The same calls as the GTK and Win32 front ends make directly.

use serde_json::{Value, json};

use crate::{Daemon, device};

/// The daemon: up or not, the node, storage, libraries.
pub fn status() -> Value {
    let d = Daemon::new();
    let s = d.status();
    let libraries = if s.up { d.namespaces().map(|l| json!(l)).unwrap_or(json!([])) } else { json!([]) };
    json!({
        "up": s.up,
        "store": s.store,
        "node": s.node,
        "chunks": s.chunks,
        "usage": s.usage,
        "error": s.error,
        "libraries": libraries,
    })
}

/// Public libraries heard on the network.
pub fn heard() -> Value {
    Daemon::new().heard().map(|l| json!(l)).unwrap_or_else(|e| json!({ "error": e }))
}

/// Joins the heard library at `address` (from its announce): its local name.
pub fn join(address: &str) -> Result<String, String> {
    let d = Daemon::new();
    let all = d.heard()?;
    let l = all.into_iter().find(|l| l.address == address).ok_or("no longer heard")?;
    d.join_heard(&l)
}

pub fn leave(ns: &str) -> Result<(), String> {
    Daemon::new().leave(ns)
}

/// Reticulum and its connections (configured, and as they are now).
pub fn reticulum() -> Value {
    let r = device::reticulum();
    let configured = device::configured().unwrap_or_default();
    let connections: Vec<Value> = configured
        .iter()
        .map(|c| {
            let live = r.interfaces.iter().find(|i| i.name == c.name);
            json!({
                "name": c.name,
                "kind": c.kind.trim_end_matches("Interface"),
                "enabled": c.enabled,
                "up": live.is_some_and(|i| i.up),
                "rx": live.map_or(0, |i| i.rx),
                "tx": live.map_or(0, |i| i.tx),
            })
        })
        .collect();
    json!({
        "running": r.running,
        "transport": r.transport_id,
        "uptime": r.uptime_s,
        "error": r.error,
        "connections": connections,
    })
}

/// The settings the utility offers: `value` what to show in the field (the
/// file's, else the default this machine gets, so a field is never empty
/// where there is one), `default` that default, `set` whether the file sets
/// it, `unit` a hint for sizes. Sizes are shown with their unit (`250 GB`).
pub fn settings() -> Value {
    let list: Vec<Value> = device::settings_view(&crate::default_store())
        .into_iter()
        .map(|s| {
            json!({
                "section": s.section,
                "key": s.key,
                "what": s.what,
                "switch": s.key == "no_storage",
                "value": s.value,
                "default": s.default,
                "set": s.set,
                "unit": s.unit,
            })
        })
        .collect();
    json!(list)
}

/// Saves settings: a JSON object `{"section.key": "typed value" | null}`;
/// only those that changed are written, and one typed as its default (as the
/// field was filled) stays the default. Sizes are taken with units (`250 GB`)
/// and written as retifsd reads them (`250G`). The daemon is restarted by the
/// caller.
pub fn save_settings(values: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(values).map_err(|e| e.to_string())?;
    let typed: Vec<(&str, &str, String)> = device::SETTINGS
        .iter()
        .filter_map(|&(section, key, _)| v.get(format!("{section}.{key}")).map(|t| (section, key, t.as_str().unwrap_or("").to_string())))
        .collect();
    device::save_typed(&crate::default_store(), &typed)
}

/// What a tray icon shows: `health` ("good", "partly", "down") and the
/// lines of text for its tooltip or menu.
pub fn tray() -> Value {
    use crate::status_icon::{Health, TrayState};
    let t = TrayState::of(&Daemon::new().status(), &device::reticulum());
    let health = match t.health() {
        Health::Good => "good",
        Health::Partly => "partly",
        Health::Down => "down",
    };
    json!({ "health": health, "lines": t.lines() })
}

/// The services retifs runs as here, with their state.
pub fn services() -> Value {
    let list: Vec<Value> = device::SERVICES
        .iter()
        .map(|(name, what)| json!({ "name": name, "what": what, "state": device::service_state(name) }))
        .collect();
    json!(list)
}
