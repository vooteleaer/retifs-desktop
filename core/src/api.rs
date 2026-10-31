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

/// The settings the utility offers, with their values (null: the default).
pub fn settings() -> Value {
    let text = std::fs::read_to_string(device::settings_file(&crate::default_store())).unwrap_or_default();
    let list: Vec<Value> = device::SETTINGS
        .iter()
        .map(|(section, key, what)| {
            json!({
                "section": section,
                "key": key,
                "what": what,
                "switch": *key == "no_storage",
                "value": device::get_setting(&text, section, key).map(|v| device::shown_value(&v)),
            })
        })
        .collect();
    json!(list)
}

/// Saves settings: a JSON object `{"section.key": "typed value" | null}`;
/// only those that changed are written. The daemon is restarted by the caller.
pub fn save_settings(values: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(values).map_err(|e| e.to_string())?;
    let path = device::settings_file(&crate::default_store());
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    for (section, key, _) in device::SETTINGS {
        let Some(typed) = v.get(format!("{section}.{key}")) else { continue };
        let now = typed.as_str().map(str::trim).filter(|t| !t.is_empty()).map(device::toml_value);
        if device::get_setting(&text, section, key) != now {
            text = device::set_setting(&text, section, key, now.as_deref());
        }
    }
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The services retifs runs as here, with their state.
pub fn services() -> Value {
    let list: Vec<Value> = device::SERVICES
        .iter()
        .map(|(name, what)| json!({ "name": name, "what": what, "state": device::service_state(name) }))
        .collect();
    json!(list)
}
