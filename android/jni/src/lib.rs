//! retifs-desktop for Android: the shared core to Kotlin through JNI
//! (`org.retifs.desktop.Core`), every answer as JSON (Android parses it with
//! its own org.json). The same calls as the GTK and Win32 front ends; no
//! second implementation of the daemon's protocol in Kotlin.
//!
//! Errors come back as a string ("" when fine), or inside the JSON.

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jint, jstring, JNI_TRUE};
use jni::JNIEnv;
use retifs_desktop_core::device;
use retifs_desktop_core::Daemon;
use serde_json::{json, Value};

fn arg(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(Into::into).unwrap_or_default()
}

fn out(env: &mut JNIEnv, s: impl Into<String>) -> jstring {
    env.new_string(s.into()).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn result(env: &mut JNIEnv, r: Result<(), String>) -> jstring {
    out(env, r.err().unwrap_or_default())
}

/// Where everything is: the app's files folder as HOME (prnsd's
/// `~/.reticulum`), the store, and the prnsd among the native libraries.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_init<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, home: JString<'l>, store: JString<'l>, prnsd: JString<'l>) {
    let (home, store, prnsd) = (arg(&mut env, &home), arg(&mut env, &store), arg(&mut env, &prnsd));
    std::env::set_var("HOME", home);
    std::env::set_var("RETIFS_STORE", store);
    std::env::set_var("RETIFS_PRNSD", prnsd);
    // The app runs prnsd itself (`prnsd run`, which can't apply changes
    // live): it restarts it after a change.
    std::env::set_var("RETIFS_PRNSD_APPLY", "restart");
}

/// The daemon: up or not, the node, storage, libraries.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_status<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
    let d = Daemon::new();
    let s = d.status();
    let libraries: Value = if s.up { d.namespaces().map(|l| json!(l)).unwrap_or(json!([])) } else { json!([]) };
    out(
        &mut env,
        json!({
            "up": s.up,
            "store": s.store,
            "node": s.node,
            "chunks": s.chunks,
            "usage": s.usage,
            "error": s.error,
            "libraries": libraries,
        })
        .to_string(),
    )
}

/// Public libraries heard on the network, as JSON.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_heard<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
    let r = Daemon::new().heard().map(|l| json!(l)).unwrap_or_else(|e| json!({ "error": e }));
    out(&mut env, r.to_string())
}

/// Joins the heard library at `address` (from its announce); "" or an error.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_join<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, address: JString<'l>) -> jstring {
    let address = arg(&mut env, &address);
    let d = Daemon::new();
    let r = d.heard().and_then(|all| {
        let l = all.into_iter().find(|l| l.address == address).ok_or("no longer heard")?;
        d.join_heard(&l).map(|_| ())
    });
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_leave<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>) -> jstring {
    let ns = arg(&mut env, &ns);
    let r = Daemon::new().leave(&ns);
    result(&mut env, r)
}

/// Reticulum and its connections (configured, and as they are now).
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_reticulum<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
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
    out(
        &mut env,
        json!({
            "running": r.running,
            "transport": r.transport_id,
            "uptime": r.uptime_s,
            "error": r.error,
            "connections": connections,
        })
        .to_string(),
    )
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_sharedInstanceUp<'l>(_: JNIEnv<'l>, _: JClass<'l>) -> jboolean {
    device::shared_instance_up() as jboolean
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_addTcp<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, name: JString<'l>, host: JString<'l>, port: jint) -> jstring {
    let (name, host) = (arg(&mut env, &name), arg(&mut env, &host));
    let r = u16::try_from(port).map_err(|_| "a port is 1 to 65535".to_string()).and_then(|p| device::add_tcp(&name, &host, p));
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_setInterface<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, name: JString<'l>, enabled: jboolean) -> jstring {
    let name = arg(&mut env, &name);
    let r = device::set_interface(&name, enabled == JNI_TRUE);
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_removeInterface<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, name: JString<'l>) -> jstring {
    let name = arg(&mut env, &name);
    let r = device::remove_interface(&name);
    result(&mut env, r)
}

/// The settings the utility offers, with their values (null: the default).
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_settings<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
    let text = std::fs::read_to_string(device::settings_file(&retifs_desktop_core::default_store())).unwrap_or_default();
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
    out(&mut env, json!(list).to_string())
}

/// Saves settings: a JSON object `{"section.key": "typed value" | null}`;
/// only those that changed are written. The daemon is restarted by the app.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_saveSettings<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, values: JString<'l>) -> jstring {
    let values = arg(&mut env, &values);
    let r = (|| -> Result<(), String> {
        let v: Value = serde_json::from_str(&values).map_err(|e| e.to_string())?;
        let path = device::settings_file(&retifs_desktop_core::default_store());
        let mut text = std::fs::read_to_string(&path).unwrap_or_default();
        for (section, key, _) in device::SETTINGS {
            let Some(typed) = v.get(format!("{section}.{key}")) else { continue };
            let now = typed.as_str().map(str::trim).filter(|t| !t.is_empty()).map(device::toml_value);
            if device::get_setting(&text, section, key) != now {
                text = device::set_setting(&text, section, key, now.as_deref());
            }
        }
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    })();
    result(&mut env, r)
}
