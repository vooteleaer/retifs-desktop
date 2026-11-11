//! The core as JSON, for front ends in other languages (Kotlin on Android,
//! C++ on Haiku): every call blocks and answers a JSON value or an error
//! string. The same calls as the GTK and Win32 front ends make directly.

use serde_json::{Value, json};

use crate::{Daemon, device};

/// The daemon: up or not, the node, storage, libraries.
pub fn status() -> Value {
    let d = Daemon::new();
    let s = d.status();
    let libraries = if s.up { d.libraries().unwrap_or(json!([])) } else { json!([]) };
    json!({
        "up": s.up,
        "store": s.store,
        "node": s.node,
        "chunks": s.chunks,
        "usage": s.usage,
        "error": s.error,
        // While it starts: the step, its progress (0 to 1) if known, since when.
        "starting": s.starting.as_ref().map(|st| json!({ "step": st.step, "progress": st.progress, "since_ms": st.since_ms, "text": s.starting_text() })),
        "libraries": libraries,
    })
}

/// Whether the daemon answers (a ping only: for a status line asked often).
pub fn up() -> bool {
    Daemon::new().status().up
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

/// Joins the library a `retifs:` link (or invitation) names, under the name
/// it suggests: its local name.
pub fn open_link(link: &str) -> Result<String, String> {
    let d = Daemon::new();
    let ns = Daemon::link_name(link)?;
    d.open_link(link, &ns)?;
    Ok(ns)
}

/// Makes the recovery kit: `{ "words": [...], "covered": [...], "not_covered": [...] }`.
pub fn recovery_create() -> Result<Value, String> {
    let k = Daemon::new().recovery_create()?;
    Ok(json!({ "words": k.words, "covered": k.covered, "not_covered": k.not_covered }))
}

/// Checks typed recovery words: `{ "ok": bool, "say": "…" }`.
pub fn recovery_check(text: &str) -> Value {
    match crate::check_recovery_words(text) {
        Ok(_) => json!({ "ok": true, "say": "The words check out." }),
        Err(e) => json!({ "ok": false, "say": e }),
    }
}

/// Recovers from the words (blocks for minutes): `{ "libraries": [...] }`.
pub fn recovery_restore(text: &str) -> Result<Value, String> {
    let words = crate::check_recovery_words(text)?;
    let libraries = Daemon::new().recovery_restore(&words)?;
    Ok(json!({ "libraries": libraries }))
}

pub fn leave(ns: &str) -> Result<(), String> {
    Daemon::new().leave(ns)
}

/// Deletes library `ns` on every member's device.
pub fn delete_library(ns: &str) -> Result<(), String> {
    Daemon::new().delete_library(ns)
}

/// Makes a new library of this user's (its keys made here), named `name`.
pub fn create(name: &str) -> Result<(), String> {
    Daemon::new().create_namespace(name)
}

/// A new library, said to be for `purposes` (ADR-0032; none: generic).
pub fn create_library(name: &str, purposes: &[String]) -> Result<(), String> {
    let d = Daemon::new();
    d.create_namespace(name)?;
    if purposes.is_empty() { Ok(()) } else { d.set_purpose(name, purposes) }
}

/// Says what a library is for (ADR-0032; none: generic).
pub fn set_purpose(ns: &str, purposes: &[String]) -> Result<(), String> {
    Daemon::new().set_purpose(ns, purposes)
}

/// "contacts,calendar" as words.
pub fn purpose_words(csv: &str) -> Vec<String> {
    csv.split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect()
}

/// Keeps all of a library here (records and content, readable offline), or
/// only what's used.
pub fn keep(ns: &str, all: bool) -> Result<(), String> {
    Daemon::new().keep(ns, all)
}

/// A library's members (see `Daemon::members`).
pub fn members(ns: &str) -> Value {
    Daemon::new().members(ns).unwrap_or_else(|e| json!({ "error": e }))
}

/// An invitation's link: `role` "read", "write" or "invite", for `days`.
pub fn invite(ns: &str, role: &str, days: u32) -> Result<String, String> {
    Daemon::new().invite(ns, role, u64::from(days.max(1)) * 86_400_000)
}

/// The invitations made to a library: `[{"id", "role", "expires", "spent"}]`.
pub fn invites(ns: &str) -> Value {
    match Daemon::new().invites(ns) {
        Ok(list) => json!(list
            .iter()
            .map(|i| json!({
                "id": i.id,
                "role": if !i.write { "read" } else if i.grant { "read, write and invite" } else { "read and write" },
                "expires": i.expires,
                "spent": i.spent,
            }))
            .collect::<Vec<_>>()),
        Err(e) => json!({ "error": e }),
    }
}

pub fn cancel_invite(ns: &str, id: &str) -> Result<(), String> {
    Daemon::new().cancel_invite(ns, id)
}

pub fn remove_member(ns: &str, writer: &str) -> Result<(), String> {
    Daemon::new().remove_member(ns, writer)
}

/// A library's link (`retifs:ns/…`): whoever has it can read the library.
pub fn link(ns: &str) -> Result<String, String> {
    Daemon::new().link(ns)
}

/// Announces a library on the network as public, or stops.
pub fn announce(ns: &str, on: bool) -> Result<(), String> {
    Daemon::new().announce(ns, on)
}

/// Makes a library a favorite (on every device of the user's), or not.
pub fn favorite(ns: &str, on: bool) -> Result<(), String> {
    Daemon::new().favorite(ns, on)
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

/// Waits until one of the libraries in `known` (`{"name": change count}`)
/// changes, for at most `timeout_ms`: `{"changed", "known"}`, every count
/// brought up to date. One whose count isn't known yet (`null`) is watched
/// from its count now; one retifsd has no count for is left out.
pub fn wait_change(known: &Value, timeout_ms: u64) -> Value {
    let d = Daemon::new();
    let given = known.as_object().cloned().unwrap_or_default();
    let mut counts: std::collections::HashMap<String, u64> = given.iter().filter_map(|(ns, n)| n.as_u64().map(|n| (ns.clone(), n))).collect();
    let fresh: Vec<String> = given.keys().filter(|ns| !counts.contains_key(*ns)).cloned().collect();
    if !fresh.is_empty() {
        counts.extend(d.change_counts(&fresh));
    }
    let changed = d.wait_change(&mut counts, std::time::Duration::from_millis(timeout_ms));
    json!({ "changed": changed, "known": counts })
}

/// The services retifs runs as here, with their state.
pub fn services() -> Value {
    let list: Vec<Value> = device::SERVICES
        .iter()
        .map(|(name, what)| json!({ "name": name, "what": what, "state": device::service_state(name) }))
        .collect();
    json!(list)
}
