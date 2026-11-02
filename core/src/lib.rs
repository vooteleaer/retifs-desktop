//! What every retifs-desktop front end shares (GTK on Linux, Windows, Android,
//! Haiku): talking to the local `retifsd` through `libretifs`.
//!
//! The utility manages retifs (this node, storage, namespaces as units:
//! members, sharing, what's kept where, devices); it doesn't browse or
//! catalogue content, which is what apps such as a photo album are for.
//!
//! Calls are blocking: a front end runs them on a worker thread and shows
//! the result on its own UI thread. The core keeps no state of its own
//! beyond the connection's runtime.

pub mod api;
pub mod device;
pub mod status_icon;

use std::{collections::HashMap, path::PathBuf};

pub use libretifs::{saved::SavedQuery, AttrValue, HeardLibrary, InviteInfo, NsView, RecordView, StoreUsage};
use libretifs::{saved, Client, Request, Response};

/// One record a search found, and where.
#[derive(Debug, Clone)]
pub struct Hit {
    pub ns: String,
    pub record: RecordView,
}

impl Hit {
    fn text(&self, key: &str) -> Option<String> {
        self.record.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| match v {
            AttrValue::Str(s) => s.clone(),
            other => format_value(other),
        })
    }

    /// What to call it: its name, title or path, else its ID.
    pub fn title(&self) -> String {
        self.title_key().and_then(|k| self.text(k)).unwrap_or_else(|| self.record.id.clone())
    }

    fn title_key(&self) -> Option<&'static str> {
        ["name", "title", "path"].into_iter().find(|k| self.text(k).is_some())
    }

    /// Its other attributes (not the title), briefly.
    pub fn summary(&self) -> String {
        let title = self.title_key();
        self.record
            .attrs
            .iter()
            .filter(|(k, _)| Some(k.as_str()) != title)
            .map(|(k, v)| format!("{k} {}", format_value(v)))
            .collect::<Vec<_>>()
            .join(" · ")
    }

    pub fn path(&self) -> Option<String> {
        self.text("path")
    }
}

/// An attribute value as text.
pub fn format_value(v: &AttrValue) -> String {
    match v {
        AttrValue::Null => "—".into(),
        AttrValue::Bool(b) => b.to_string(),
        AttrValue::Int(i) => i.to_string(),
        AttrValue::Float(f) => f.to_string(),
        AttrValue::Str(s) => s.clone(),
        AttrValue::Bytes(b) => format!("{} bytes", b.len()),
        AttrValue::List(items) => items.iter().map(format_value).collect::<Vec<_>>().join(", "),
    }
}

/// The daemon as the window and tray show it.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub up: bool,
    pub store: String,
    pub node: Option<String>,
    pub chunks: u64,
    pub usage: Option<StoreUsage>,
    pub error: Option<String>,
}

/// The store the local daemon owns (`$RETIFS_STORE`, else the platform default).
pub fn default_store() -> PathBuf {
    libretifs::default_store_dir()
}

/// Human-readable sizes (binary units).
pub fn human(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", units[i]) }
}

/// The connection to `retifsd`: a small runtime for libretifs' async client.
pub struct Daemon {
    rt: tokio::runtime::Runtime,
    store: PathBuf,
}

impl Daemon {
    /// The daemon of the default store (`$RETIFS_STORE`, else the platform default).
    pub fn new() -> Self {
        Self::at(libretifs::default_store_dir())
    }

    pub fn at(store: PathBuf) -> Self {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime");
        Daemon { rt, store }
    }

    /// The store this daemon owns (its settings file is there).
    pub fn store(&self) -> &std::path::Path {
        &self.store
    }

    fn client(&self) -> Result<Client, String> {
        self.rt.block_on(Client::connect(&self.store)).map_err(|e| e.to_string())
    }

    pub fn status(&self) -> Status {
        let store = self.store.display().to_string();
        let mut c = match self.client() {
            Ok(c) => c,
            Err(e) => return Status { store, error: Some(e), ..Default::default() },
        };
        match self.rt.block_on(c.ping()) {
            Ok((node, chunks, usage)) => Status { up: true, store, node: Some(node), chunks, usage, error: None },
            Err(e) => Status { store, error: Some(e.to_string()), ..Default::default() },
        }
    }

    fn request(&self, c: &mut Client, req: Request) -> Result<Response, String> {
        match self.rt.block_on(c.call(req)).map_err(|e| e.to_string())? {
            Response::Error { message } => Err(message),
            r => Ok(r),
        }
    }

    /// Public libraries heard announced on the network (retifs ADR-0029).
    pub fn heard(&self) -> Result<Vec<libretifs::HeardLibrary>, String> {
        let mut c = self.client()?;
        match self.request(&mut c, Request::HeardLibraries)? {
            Response::Heard { libraries } => Ok(libraries),
            _ => Err("unexpected answer".into()),
        }
    }

    /// Joins a heard library lazily (only what's looked at is fetched), from
    /// the node that announced it, under a name not taken here. Its name.
    pub fn join_heard(&self, l: &libretifs::HeardLibrary) -> Result<String, String> {
        let base = libretifs::link::suggest_name(if l.name.trim().is_empty() { "library" } else { &l.name });
        let mut c = self.client()?;
        let taken = match self.request(&mut c, Request::Namespaces)? {
            Response::Names { names } => names,
            _ => Vec::new(),
        };
        let name = std::iter::once(base.clone()).chain((2..).map(|i| format!("{base}-{i}"))).find(|n| !taken.contains(n)).expect("a free name");
        // A map (ADR-0028): its index held whole (its tiles' records, small),
        // so a tile is found here at once; its tiles fetched as looked at.
        // Anything else lazily.
        let map = l.has.iter().any(|(t, _)| t == "application/x-vnd.retifs.tileset" || t == "application/vnd.mapbox-vector-tile");
        self.request(&mut c, Request::NsJoin { ns: name.clone(), address: l.address.clone(), from: vec![l.node.clone()], lazy: !map })?;
        if map {
            self.request(&mut c, Request::NsKeep { ns: name.clone(), mode: "rules".into() })?;
        }
        Ok(name)
    }

    /// Leaves a library on this device (others keep their copies).
    pub fn leave(&self, ns: &str) -> Result<(), String> {
        let mut c = self.client()?;
        self.request(&mut c, Request::NsLeave { ns: ns.into(), mine: false }).map(|_| ())
    }

    /// The namespaces a search may look in (all but `me`).
    pub fn namespace_names(&self) -> Result<Vec<String>, String> {
        let mut c = self.client()?;
        self.rt.block_on(saved::namespaces(&mut c))
    }

    /// Records matching `terms` (as `retifs query` takes them) in `scope`
    /// (namespace names; empty: every one but `me`).
    pub fn find(&self, scope: &[String], terms: &[String]) -> Result<Vec<Hit>, String> {
        let mut c = self.client()?;
        let names = if scope.is_empty() { self.rt.block_on(saved::namespaces(&mut c))? } else { scope.to_vec() };
        let mut out = Vec::new();
        for ns in names {
            let req = Request::Query { ns: ns.clone(), terms: terms.to_vec(), at: None };
            match self.rt.block_on(c.call(req)).map_err(|e| e.to_string())? {
                Response::Records { records } => out.extend(records.into_iter().map(|record| Hit { ns: ns.clone(), record })),
                Response::Error { message } => return Err(format!("{ns}: {message}")),
                _ => {}
            }
        }
        out.sort_by_cached_key(|h| (h.title().to_lowercase(), h.ns.clone()));
        Ok(out)
    }

    /// Each namespace's change count (it grows when its records change):
    /// a front end runs a search again when these differ.
    pub fn change_counts(&self, scope: &[String]) -> HashMap<String, u64> {
        let Ok(mut c) = self.client() else { return HashMap::new() };
        let names = if scope.is_empty() { self.rt.block_on(saved::namespaces(&mut c)).unwrap_or_default() } else { scope.to_vec() };
        names
            .into_iter()
            .filter_map(|ns| match self.rt.block_on(c.call(Request::Watch { ns: ns.clone(), since: 0, timeout_ms: 0 })) {
                Ok(Response::Changed { count }) => Some((ns, count)),
                _ => None,
            })
            .collect()
    }

    /// The saved queries (in `me`, so every device of the user's has them),
    /// each with the names its namespaces have here and how many aren't here.
    pub fn saved(&self) -> Vec<(SavedQuery, Vec<String>, usize)> {
        let Ok(mut c) = self.client() else { return Vec::new() };
        let list = self.rt.block_on(saved::list(&mut c));
        list.into_iter()
            .map(|q| {
                let (here, missing) = self.rt.block_on(saved::resolve(&mut c, &q)).unwrap_or_default();
                (q, here, missing.len())
            })
            .collect()
    }

    pub fn save(&self, name: &str, scope: &[String], terms: &[String]) -> Result<(), String> {
        let mut c = self.client()?;
        self.rt.block_on(saved::save(&mut c, name, scope, terms))
    }

    pub fn forget_saved(&self, name: &str) -> Result<(), String> {
        let mut c = self.client()?;
        self.rt.block_on(saved::remove(&mut c, name))
    }

    /// Every namespace with its view (writers, peers, keep mode, checkpoint).
    /// Followed channels' namespaces, which have no member view, are left out.
    pub fn namespaces(&self) -> Result<Vec<NsView>, String> {
        let mut c = self.client()?;
        let names = match self.rt.block_on(c.call(Request::Namespaces)).map_err(|e| e.to_string())? {
            Response::Names { names } => names,
            _ => Vec::new(),
        };
        Ok(names
            .into_iter()
            .filter_map(|ns| match self.rt.block_on(c.call(Request::NsInfo { ns })) {
                Ok(Response::Namespace(v)) => Some(v),
                _ => None,
            })
            .collect())
    }

    /// The libraries for a list of them, each its view (`NsInfo`) and what a
    /// person wants to know at a glance: `access` ("read only", "read and
    /// write", "read, write and invite"), `own` (made by this user), `public`
    /// (announced, here or by whoever made it), `favorite`. Favorites first,
    /// then by name; `me` (this user's own) last.
    pub fn libraries(&self) -> Result<serde_json::Value, String> {
        let views = self.namespaces()?;
        let mut c = self.client()?;
        let favorites: Vec<String> = self.rt.block_on(saved::favorites(&mut c)).into_iter().map(|(w, _)| w).collect();
        let heard_here: Vec<String> = self.heard().unwrap_or_default().into_iter().filter_map(|l| l.here).collect();
        let mut out: Vec<(bool, bool, String, serde_json::Value)> = views
            .into_iter()
            .map(|v| {
                let creator = v.address.split(':').nth(2).unwrap_or("").to_string();
                let rights = v.writers.iter().find(|(w, _)| *w == v.own_writer).map(|(_, r)| r.clone());
                let access = match &rights {
                    Some(r) if r.iter().any(|x| x == "grant") => "read, write and invite",
                    Some(_) => "read and write",
                    None => "read only",
                };
                let favorite = favorites.contains(&creator);
                let public = v.announced || heard_here.contains(&v.name);
                let mut j = serde_json::to_value(&v).unwrap_or_default();
                j["access"] = access.into();
                j["own"] = (creator == v.own_writer).into();
                j["public"] = public.into();
                j["favorite"] = favorite.into();
                (v.name == saved::ME, !favorite, v.name.to_lowercase(), j)
            })
            .collect();
        out.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
        Ok(serde_json::Value::Array(out.into_iter().map(|(_, _, _, j)| j).collect()))
    }

    pub fn keep(&self, ns: &str, all: bool) -> Result<(), String> {
        let mut c = self.client()?;
        let mode = if all { "all" } else { "rules" };
        self.request(&mut c, Request::NsKeep { ns: ns.into(), mode: mode.into() }).map(|_| ())
    }

    /// A read link to a library, naming this node to fetch from.
    pub fn link(&self, ns: &str) -> Result<String, String> {
        let mut c = self.client()?;
        let address = match self.request(&mut c, Request::NsInfo { ns: ns.into() })? {
            Response::Namespace(v) => v.address,
            _ => return Err("unexpected answer".into()),
        };
        let node = self.status().node.unwrap_or_default();
        let from: Vec<String> = if node.is_empty() { Vec::new() } else { vec![node] };
        Ok(libretifs::link::Link::namespace(&address, &from, Some(ns)).to_string())
    }

    pub fn announce(&self, ns: &str, on: bool) -> Result<(), String> {
        let mut c = self.client()?;
        self.request(&mut c, Request::NsAnnounce { ns: ns.into(), on, name: None, about: None }).map(|_| ())
    }

    pub fn favorite(&self, ns: &str, on: bool) -> Result<(), String> {
        let mut c = self.client()?;
        self.rt.block_on(saved::set_favorite(&mut c, ns, on))
    }

    /// A library's members, for its owner to see and manage: each writer key
    /// with `name` (this device, or the contact whose node it is), `words`
    /// (its fingerprint, three words), `role` ("read", "read and write",
    /// "read, write and invite"), `limits` (keys, until), `you`, `creator`.
    /// The creator first, then by name.
    pub fn members(&self, ns: &str) -> Result<serde_json::Value, String> {
        let mut c = self.client()?;
        let v = match self.request(&mut c, Request::NsInfo { ns: ns.into() })? {
            Response::Namespace(v) => v,
            _ => return Err("unexpected answer".into()),
        };
        let creator = v.address.split(':').nth(2).unwrap_or("").to_string();
        // Contacts by node: a member's device named as the user named it.
        let mut contacts: HashMap<String, String> = HashMap::new();
        for term in [format!("type={}", libretifs::types::PERSON), "type=contact".to_string()] {
            if let Ok(Response::Records { records }) = self.request(&mut c, Request::Query { ns: saved::ME.into(), terms: vec![term], at: None }) {
                for r in records {
                    let get = |k: &str| r.attrs.iter().find_map(|(key, val)| match val {
                        AttrValue::Str(s) if key == k => Some(s.clone()),
                        _ => None,
                    });
                    if let (Some(name), Some(node)) = (get("name"), get("retifs:node")) {
                        contacts.insert(node.to_lowercase(), name);
                    }
                }
            }
        }
        let nodes: HashMap<&str, &str> = v.nodes.iter().map(|(w, n)| (w.as_str(), n.as_str())).collect();
        let mut out: Vec<(bool, String, serde_json::Value)> = v
            .writers
            .iter()
            .map(|(w, rights)| {
                let (plain, limits): (Vec<&String>, Vec<&String>) = rights.iter().partition(|r| *r == "write" || *r == "grant");
                let role = if plain.iter().any(|r| *r == "grant") {
                    "read, write and invite"
                } else if plain.iter().any(|r| *r == "write") {
                    "read and write"
                } else {
                    "read"
                };
                let you = *w == v.own_writer;
                let node = nodes.get(w.as_str()).map(|n| n.to_string());
                let name = if you {
                    "This device".to_string()
                } else {
                    node.as_ref().and_then(|n| contacts.get(&n.to_lowercase()).cloned()).unwrap_or_default()
                };
                let key: Vec<u8> = (0..w.len() / 2).filter_map(|i| u8::from_str_radix(&w[2 * i..2 * i + 2], 16).ok()).collect();
                let words = retifs_core::words::fingerprint(&key, 3).join(" ");
                let j = serde_json::json!({
                    "writer": w, "name": name, "words": words, "role": role, "limits": limits,
                    "node": node, "you": you, "creator": *w == creator,
                });
                (*w != creator, if name.is_empty() { format!("~{words}") } else { name.to_lowercase() }, j)
            })
            .collect();
        out.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        Ok(serde_json::Value::Array(out.into_iter().map(|(_, _, j)| j).collect()))
    }

    /// An invitation to library `ns`: `role` "read", "write" or "invite"
    /// (write and invite others), good for `expires_ms`. Its link.
    pub fn invite(&self, ns: &str, role: &str, expires_ms: u64) -> Result<String, String> {
        let mut c = self.client()?;
        let req = Request::InviteCreate {
            ns: ns.into(),
            read: role == "read",
            grant: role == "invite",
            keys: Vec::new(),
            until: None,
            expires_ms: Some(expires_ms),
            q: None,
            label: Some(ns.into()),
        };
        match self.request(&mut c, req)? {
            Response::Link { link } => Ok(link),
            _ => Err("unexpected answer".into()),
        }
    }

    /// The invitations made to library `ns` (pending, used, expired).
    pub fn invites(&self, ns: &str) -> Result<Vec<libretifs::InviteInfo>, String> {
        let mut c = self.client()?;
        match self.request(&mut c, Request::InviteList { ns: ns.into() })? {
            Response::Invites { invites } => Ok(invites),
            _ => Err("unexpected answer".into()),
        }
    }

    pub fn cancel_invite(&self, ns: &str, id: &str) -> Result<(), String> {
        let mut c = self.client()?;
        self.request(&mut c, Request::InviteCancel { ns: ns.into(), id: id.into() }).map(|_| ())
    }

    /// Removes a member: its writes stop counting, and the read key changes,
    /// so it can't read what's written from now on (what it read, it keeps).
    pub fn remove_member(&self, ns: &str, writer: &str) -> Result<(), String> {
        let mut c = self.client()?;
        self.request(&mut c, Request::NsRevoke { ns: ns.into(), writer: writer.into() }).map(|_| ())
    }

    /// A new namespace of this user's, `name` (its keys are made here).
    pub fn create_namespace(&self, name: &str) -> Result<(), String> {
        let mut c = self.client()?;
        if self.rt.block_on(saved::namespaces(&mut c))?.iter().any(|n| n == name) {
            return Err(format!("there is a namespace {name} already"));
        }
        match self.rt.block_on(c.call(Request::NsAddress { ns: name.into() })).map_err(|e| e.to_string())? {
            Response::Error { message } => Err(message),
            _ => Ok(()),
        }
    }

    /// The local name a namespace link or invitation suggests.
    pub fn link_name(link: &str) -> Result<String, String> {
        let l = libretifs::link::Link::parse(link.trim()).map_err(|e| e.to_string())?;
        Ok(libretifs::link::suggest_name(l.param("name").unwrap_or("shared")))
    }

    /// Joins the namespace a `retifs:` link names, as `retifs open` does: a
    /// namespace link to read it, an invitation to write too.
    pub fn open_link(&self, link: &str, name: &str) -> Result<(), String> {
        let text = link.trim();
        let l = libretifs::link::Link::parse(text).map_err(|e| e.to_string())?;
        let req = match l.kind.as_str() {
            "invite" => Request::InviteAccept { ns: name.into(), link: text.into() },
            _ => match l.ns_address() {
                Some(address) => Request::NsJoin { ns: name.into(), address, from: l.params_named("from").map(String::from).collect(), lazy: false },
                None => return Err(format!("a retifs:{} link, not a namespace", l.kind)),
            },
        };
        let mut c = self.client()?;
        match self.rt.block_on(c.call(req)).map_err(|e| e.to_string())? {
            Response::Error { message } => Err(message),
            _ => Ok(()),
        }
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes() {
        assert_eq!(super::human(512), "512 B");
        assert_eq!(super::human(1536), "1.5 KB");
        assert_eq!(super::human(5 << 30), "5.0 GB");
    }
}
