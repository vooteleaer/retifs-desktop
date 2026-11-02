//! retifs-desktop for Android: the shared core to Kotlin through JNI
//! (`org.retifs.desktop.Core`), every answer as JSON from `core::api` (Android
//! parses it with its own org.json). The same calls as the GTK and Win32
//! front ends; no second implementation of the daemon's protocol in Kotlin.
//!
//! Errors come back as a string ("" when fine), or inside the JSON.

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jint, jstring, JNI_TRUE};
use jni::JNIEnv;
use retifs_desktop_core::{api, device};

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

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_status<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
    out(&mut env, api::status().to_string())
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_heard<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
    out(&mut env, api::heard().to_string())
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_join<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, address: JString<'l>) -> jstring {
    let address = arg(&mut env, &address);
    let r = api::join(&address).map(|_| ());
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_leave<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>) -> jstring {
    let ns = arg(&mut env, &ns);
    let r = api::leave(&ns);
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_reticulum<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
    out(&mut env, api::reticulum().to_string())
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

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_settings<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
    out(&mut env, api::settings().to_string())
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_saveSettings<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, values: JString<'l>) -> jstring {
    let values = arg(&mut env, &values);
    let r = api::save_settings(&values);
    result(&mut env, r)
}

// Libraries: what can be done with one, members and invitations (as the
// Haiku, GTK and Windows utilities).

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_create<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, name: JString<'l>, public: jboolean) -> jstring {
    let name = arg(&mut env, &name);
    let r = api::create(&name).and_then(|()| if public == JNI_TRUE { api::announce(&name, true) } else { Ok(()) });
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_keep<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, all: jboolean) -> jstring {
    let ns = arg(&mut env, &ns);
    let r = api::keep(&ns, all == JNI_TRUE);
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_favorite<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, on: jboolean) -> jstring {
    let ns = arg(&mut env, &ns);
    let r = api::favorite(&ns, on == JNI_TRUE);
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_announce<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, on: jboolean) -> jstring {
    let ns = arg(&mut env, &ns);
    let r = api::announce(&ns, on == JNI_TRUE);
    result(&mut env, r)
}

/// A library's link, or "error: …".
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_link<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>) -> jstring {
    let ns = arg(&mut env, &ns);
    let r = api::link(&ns).unwrap_or_else(|e| format!("error: {e}"));
    out(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_members<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>) -> jstring {
    let ns = arg(&mut env, &ns);
    out(&mut env, api::members(&ns).to_string())
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_invites<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>) -> jstring {
    let ns = arg(&mut env, &ns);
    out(&mut env, api::invites(&ns).to_string())
}

/// An invitation's link (`role`: read, write, invite), or "error: …".
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_invite<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, role: JString<'l>, days: jint) -> jstring {
    let (ns, role) = (arg(&mut env, &ns), arg(&mut env, &role));
    let r = api::invite(&ns, &role, days.max(1) as u32).unwrap_or_else(|e| format!("error: {e}"));
    out(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_cancelInvite<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, id: JString<'l>) -> jstring {
    let (ns, id) = (arg(&mut env, &ns), arg(&mut env, &id));
    let r = api::cancel_invite(&ns, &id);
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_removeMember<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, writer: JString<'l>) -> jstring {
    let (ns, writer) = (arg(&mut env, &ns), arg(&mut env, &writer));
    let r = api::remove_member(&ns, &writer);
    result(&mut env, r)
}
