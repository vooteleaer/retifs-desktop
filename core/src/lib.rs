//! What every retifs-desktop front end shares (GTK on Linux, Windows and
//! Haiku later): talking to the local `retifsd` through `libretifs`.
//!
//! The utility manages retifs (this node, storage, namespaces as units:
//! members, sharing, what's kept where, devices); it doesn't browse or
//! catalogue content, which is what apps such as a photo album are for.
//!
//! Calls are blocking: a front end runs them on a worker thread and shows
//! the result on its own UI thread. The core keeps no state of its own
//! beyond the connection's runtime.

pub mod device;

use std::{collections::HashMap, path::PathBuf};

pub use libretifs::{saved::SavedQuery, AttrValue, NsView, RecordView, StoreUsage};
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
