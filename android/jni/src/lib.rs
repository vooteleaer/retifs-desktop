//! retifs-desktop for Android: the shared core to Kotlin through JNI
//! (`org.retifs.desktop.Core`), every answer as JSON from `core::api` (Android
//! parses it with its own org.json). The same calls as the GTK and Win32
//! front ends; no second implementation of the daemon's protocol in Kotlin.
//!
//! Errors come back as a string ("" when fine), or inside the JSON.

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jint, jstring, JNI_TRUE};
use jni::JNIEnv;
use retifs_desktop_core::{api, device, files};

fn arg(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(Into::into).unwrap_or_default()
}

fn out(env: &mut JNIEnv, s: impl Into<String>) -> jstring {
    env.new_string(s.into()).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn result(env: &mut JNIEnv, r: Result<(), String>) -> jstring {
    out(env, r.err().unwrap_or_default())
}

/// Where everything is: the app's files folder as HOME, and the store.
/// Reticulum is Prns's app, not run by this one.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_init<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, home: JString<'l>, store: JString<'l>) {
    let (home, store) = (arg(&mut env, &home), arg(&mut env, &store));
    std::env::set_var("HOME", home);
    std::env::set_var("RETIFS_STORE", store);
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_status<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
    out(&mut env, api::status().to_string())
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_up<'l>(_: JNIEnv<'l>, _: JClass<'l>) -> jni::sys::jboolean {
    api::up() as jni::sys::jboolean
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

// Libraries as files (the DocumentsProvider). Paths are a library's own
// (`/a/b.txt`); listings are JSON, errors a string ("" when fine).

fn entry_json(e: &files::Entry) -> serde_json::Value {
    serde_json::json!({ "name": e.name, "path": e.path, "dir": e.dir, "size": e.size, "mtime": e.mtime_ms })
}

/// The libraries the Files app shows (all but `me`): `["name", …]`, or `{"error"}`.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_docLibraries<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
    let r = match retifs_desktop_core::Daemon::new().namespace_names() {
        Ok(names) => serde_json::json!(names),
        Err(e) => serde_json::json!({ "error": e }),
    };
    out(&mut env, r.to_string())
}

/// A folder's entries: `[{name, path, dir, size, mtime}]`, or `{"error"}`.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_docList<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, dir: JString<'l>) -> jstring {
    let (ns, dir) = (arg(&mut env, &ns), arg(&mut env, &dir));
    let r = match retifs_desktop_core::Daemon::new().list(&ns, &dir) {
        Ok(entries) => serde_json::Value::Array(entries.iter().map(entry_json).collect()),
        Err(e) => serde_json::json!({ "error": e }),
    };
    out(&mut env, r.to_string())
}

/// One entry, `null` if there's none, or `{"error"}`.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_docStat<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, path: JString<'l>) -> jstring {
    let (ns, path) = (arg(&mut env, &ns), arg(&mut env, &path));
    let r = match retifs_desktop_core::Daemon::new().stat(&ns, &path) {
        Ok(Some(e)) => entry_json(&e),
        Ok(None) => serde_json::Value::Null,
        Err(e) => serde_json::json!({ "error": e }),
    };
    out(&mut env, r.to_string())
}

/// A file's content into the file `dest`.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_docRead<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, path: JString<'l>, dest: JString<'l>) -> jstring {
    let (ns, path, dest) = (arg(&mut env, &ns), arg(&mut env, &path), arg(&mut env, &dest));
    let r = files::read_to(&ns, &path, std::path::Path::new(&dest)).map(|_| ());
    result(&mut env, r)
}

/// The file `src` as the file at `path` (a new version, or a new record).
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_docWrite<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, path: JString<'l>, src: JString<'l>) -> jstring {
    let (ns, path, src) = (arg(&mut env, &ns), arg(&mut env, &path), arg(&mut env, &src));
    let r = files::write_from(&ns, &path, std::path::Path::new(&src));
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_docMkdir<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, path: JString<'l>) -> jstring {
    let (ns, path) = (arg(&mut env, &ns), arg(&mut env, &path));
    let r = retifs_desktop_core::Daemon::new().make_dir(&ns, &path);
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_docRemove<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, path: JString<'l>) -> jstring {
    let (ns, path) = (arg(&mut env, &ns), arg(&mut env, &path));
    let r = retifs_desktop_core::Daemon::new().remove(&ns, &path);
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_docRename<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, from: JString<'l>, to: JString<'l>) -> jstring {
    let (ns, from, to) = (arg(&mut env, &ns), arg(&mut env, &from), arg(&mut env, &to));
    let r = retifs_desktop_core::Daemon::new().rename(&ns, &from, &to);
    result(&mut env, r)
}

// Contacts for the sync adapter (Android's own Contacts, the dialer and every
// app that reads them): retifs-contacts' records, as JSON.

fn contact_json(c: &retifs_contacts_core::Contact) -> serde_json::Value {
    serde_json::json!({ "ns": c.ns, "id": c.id, "values": c.values })
}

fn contact_of(v: &serde_json::Value) -> retifs_contacts_core::Contact {
    let mut c = retifs_contacts_core::Contact::new(v["ns"].as_str().unwrap_or(retifs_contacts_core::ME));
    c.id = v["id"].as_str().map(String::from);
    if let Some(values) = v["values"].as_object() {
        c.values = values.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect();
    }
    c
}

/// The libraries holding contacts, `me` first, each with whether it may be
/// written here: `[{"ns", "writable"}]`, or `{"error"}`.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_contactLibraries<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
    let r = match retifs_contacts_core::Book::new().libraries() {
        Ok(mut names) => {
            let access = retifs_desktop_core::Daemon::new().libraries().unwrap_or_default();
            let libs: Vec<&serde_json::Value> = access.as_array().into_iter().flatten().collect();
            // Made for contacts (ADR-0032), even with none in it yet.
            for l in &libs {
                let name = l["name"].as_str().unwrap_or_default();
                if l["for"].as_array().is_some_and(|f| f.iter().any(|w| w == "contacts")) && !names.iter().any(|n| n == name) {
                    names.push(name.to_string());
                }
            }
            let writable = |ns: &str| ns == retifs_contacts_core::ME || libs.iter().any(|l| l["name"] == ns && l["access"].as_str().is_some_and(|a| a.contains("write")));
            serde_json::Value::Array(names.iter().map(|n| serde_json::json!({ "ns": n, "writable": writable(n) })).collect())
        }
        Err(e) => serde_json::json!({ "error": e }),
    };
    out(&mut env, r.to_string())
}

/// The contacts in `libraries` (a JSON array of names): `[{ns, id, values}]`, or `{"error"}`.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_contacts<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, libraries: JString<'l>) -> jstring {
    let libraries: Vec<String> = serde_json::from_str(&arg(&mut env, &libraries)).unwrap_or_default();
    let r = match retifs_contacts_core::Book::new().contacts(&libraries) {
        Ok(list) => serde_json::Value::Array(list.iter().map(contact_json).collect()),
        Err(e) => serde_json::json!({ "error": e }),
    };
    out(&mut env, r.to_string())
}

/// Saves `contact` (`{ns, id, values}`; `before` as it was, or "null" for a
/// new one): the contact as saved, or `{"error"}`.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_contactSave<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, contact: JString<'l>, before: JString<'l>) -> jstring {
    let contact: serde_json::Value = serde_json::from_str(&arg(&mut env, &contact)).unwrap_or_default();
    let before: serde_json::Value = serde_json::from_str(&arg(&mut env, &before)).unwrap_or_default();
    let before = (!before.is_null()).then(|| contact_of(&before));
    let r = match retifs_contacts_core::Book::new().save(&contact_of(&contact), before.as_ref()) {
        Ok(saved) => contact_json(&saved),
        Err(e) => serde_json::json!({ "error": e }),
    };
    out(&mut env, r.to_string())
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_contactDelete<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, id: JString<'l>) -> jstring {
    let (ns, id) = (arg(&mut env, &ns), arg(&mut env, &id));
    let mut c = retifs_contacts_core::Contact::new(&ns);
    c.id = Some(id);
    let r = retifs_contacts_core::Book::new().delete(&c);
    result(&mut env, r)
}

/// Waits until one of the libraries in `known` (`{"name": change count or
/// null}`) changes, for at most `timeout_ms`: `{"changed", "known"}` (see
/// `api::wait_change`).
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_waitChange<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, known: JString<'l>, timeout_ms: jint) -> jstring {
    let known: serde_json::Value = serde_json::from_str(&arg(&mut env, &known)).unwrap_or_default();
    out(&mut env, api::wait_change(&known, timeout_ms.max(0) as u64).to_string())
}

// Events for the calendar sync adapter (Android's own Calendar): retifs-calendar's
// records, as JSON.

fn event_json(e: &retifs_calendar_core::Event) -> serde_json::Value {
    serde_json::json!({ "ns": e.ns, "id": e.id, "values": e.values })
}

fn event_of(v: &serde_json::Value) -> retifs_calendar_core::Event {
    let mut e = retifs_calendar_core::Event::new(v["ns"].as_str().unwrap_or(retifs_calendar_core::ME));
    e.id = v["id"].as_str().map(String::from);
    if let Some(values) = v["values"].as_object() {
        e.values = values.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect();
    }
    e
}

/// The libraries that are calendars (holding events, a `retifs.calendar`
/// record, or made for it, ADR-0032), `me` first: `[{"ns", "writable",
/// "color"}]`, or `{"error"}`.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_calendarLibraries<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
    let cal = retifs_calendar_core::Calendar::new();
    let r = match cal.libraries() {
        Ok(mut names) => {
            let access = retifs_desktop_core::Daemon::new().libraries().unwrap_or_default();
            let libs: Vec<&serde_json::Value> = access.as_array().into_iter().flatten().collect();
            for l in &libs {
                let name = l["name"].as_str().unwrap_or_default();
                if l["for"].as_array().is_some_and(|f| f.iter().any(|w| w == "calendar")) && !names.iter().any(|n| n == name) {
                    names.push(name.to_string());
                }
            }
            let writable = |ns: &str| ns == retifs_calendar_core::ME || libs.iter().any(|l| l["name"] == ns && l["access"].as_str().is_some_and(|a| a.contains("write")));
            serde_json::Value::Array(names.iter().map(|n| serde_json::json!({ "ns": n, "writable": writable(n), "color": cal.color(n) })).collect())
        }
        Err(e) => serde_json::json!({ "error": e }),
    };
    out(&mut env, r.to_string())
}

/// Library `ns`'s events as they are (a repeating one once): `[{ns, id, values}]`, or `{"error"}`.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_calendarEvents<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>) -> jstring {
    let ns = arg(&mut env, &ns);
    let r = match retifs_calendar_core::Calendar::new().events(&ns) {
        Ok(list) => serde_json::Value::Array(list.iter().map(event_json).collect()),
        // A library not here yet (`me` before anything was saved): none.
        Err(e) if e.contains("no library") || e.contains("no namespace") => serde_json::json!([]),
        Err(e) => serde_json::json!({ "error": e }),
    };
    out(&mut env, r.to_string())
}

/// Saves `event` (`{ns, id, values}`; `before` as it was, or "null"): as saved, or `{"error"}`.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_calendarSave<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, event: JString<'l>, before: JString<'l>) -> jstring {
    let event: serde_json::Value = serde_json::from_str(&arg(&mut env, &event)).unwrap_or_default();
    let before: serde_json::Value = serde_json::from_str(&arg(&mut env, &before)).unwrap_or_default();
    let before = (!before.is_null()).then(|| event_of(&before));
    let r = match retifs_calendar_core::Calendar::new().save(&event_of(&event), before.as_ref()) {
        Ok(saved) => event_json(&saved),
        Err(e) => serde_json::json!({ "error": e }),
    };
    out(&mut env, r.to_string())
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_calendarDelete<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, id: JString<'l>) -> jstring {
    let (ns, id) = (arg(&mut env, &ns), arg(&mut env, &id));
    let mut e = retifs_calendar_core::Event::new(&ns);
    e.id = Some(id);
    let r = retifs_calendar_core::Calendar::new().delete(&e);
    result(&mut env, r)
}

/// Joins the library an invitation or `retifs:` link names, as `name` (or,
/// empty, the name the link suggests).
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_openLink<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, link: JString<'l>, name: JString<'l>) -> jstring {
    let (link, name) = (arg(&mut env, &link), arg(&mut env, &name));
    let r = if name.trim().is_empty() { api::open_link(link.trim()).map(|_| ()) } else { retifs_desktop_core::Daemon::new().open_link(link.trim(), name.trim()) };
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_leave<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>) -> jstring {
    let ns = arg(&mut env, &ns);
    let r = api::leave(&ns);
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_deleteLibrary<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>) -> jstring {
    let ns = arg(&mut env, &ns);
    let r = api::delete_library(&ns);
    result(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_sharedInstanceUp<'l>(_: JNIEnv<'l>, _: JClass<'l>) -> jboolean {
    device::shared_instance_up() as jboolean
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
pub extern "system" fn Java_org_retifs_desktop_Core_create<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, name: JString<'l>, public: jboolean, purposes: JString<'l>) -> jstring {
    let (name, purposes) = (arg(&mut env, &name), arg(&mut env, &purposes));
    let r = api::create_library(&name, &api::purpose_words(&purposes)).and_then(|()| if public == JNI_TRUE { api::announce(&name, true) } else { Ok(()) });
    result(&mut env, r)
}

/// Says what a library is for: "contacts,calendar" (ADR-0032; "": generic).
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_setPurpose<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, ns: JString<'l>, purposes: JString<'l>) -> jstring {
    let (ns, purposes) = (arg(&mut env, &ns), arg(&mut env, &purposes));
    let r = api::set_purpose(&ns, &api::purpose_words(&purposes));
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

// Other apps' access (the access service): an app the user let in gets a
// token of its own, kept by this app, and connections made with it.

/// Lets app `name` use `namespaces` (a JSON array), writing there if `write`:
/// its token, or "error: …". The same name again replaces it.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_appAdd<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, name: JString<'l>, namespaces: JString<'l>, write: jboolean) -> jstring {
    let name = arg(&mut env, &name);
    let namespaces: Vec<String> = serde_json::from_str(&arg(&mut env, &namespaces)).unwrap_or_default();
    let r = retifs_desktop_core::Daemon::new().app_add(&name, namespaces, write == JNI_TRUE).unwrap_or_else(|e| format!("error: {e}"));
    out(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_appRemove<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, name: JString<'l>) -> jstring {
    let name = arg(&mut env, &name);
    let r = retifs_desktop_core::Daemon::new().app_remove(&name);
    result(&mut env, r)
}

/// A connection to retifsd as the app with `token` (past its Hello): a
/// descriptor the caller owns, or -1.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_appConnect<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, token: JString<'l>) -> jint {
    use std::os::fd::IntoRawFd;
    let token = arg(&mut env, &token);
    match retifs_desktop_core::Daemon::new().app_connect(&token) {
        Ok(s) => s.into_raw_fd(),
        Err(e) => {
            eprintln!("access: {e}");
            -1
        }
    }
}

/// The recovery kit (retifs ADR-0033): `{words, covered, not_covered}` or `{"error"}`.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_recoveryCreate<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
    let v = api::recovery_create().unwrap_or_else(|e| serde_json::json!({ "error": e }));
    out(&mut env, v.to_string())
}

/// Typed recovery words checked: `{ok, say}`.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_recoveryCheck<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, text: JString<'l>) -> jstring {
    let text = arg(&mut env, &text);
    out(&mut env, api::recovery_check(&text).to_string())
}

/// Recovers from the words (minutes): `{libraries}` or `{"error"}`.
#[no_mangle]
pub extern "system" fn Java_org_retifs_desktop_Core_recoveryRestore<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, text: JString<'l>) -> jstring {
    let text = arg(&mut env, &text);
    let v = api::recovery_restore(&text).unwrap_or_else(|e| serde_json::json!({ "error": e }));
    out(&mut env, v.to_string())
}
