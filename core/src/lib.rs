//! What every retifs-desktop front end shares (GTK on Linux, Windows and
//! Haiku later): talking to the local `retifsd` through `libretifs`.
//!
//! Calls are blocking: a front end runs them on a worker thread and shows
//! the result on its own UI thread. The core keeps no state of its own
//! beyond the connection's runtime.

use std::path::PathBuf;

pub use libretifs::{NsView, StoreUsage};
use libretifs::{Client, RecordView, Request, Response};

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

/// A record with its attribute values as text.
#[derive(Debug, Clone)]
pub struct Record {
    pub id: String,
    pub attrs: Vec<(String, String)>,
    pub blocks: Vec<String>,
}

impl Record {
    /// What to call it in a list: its title, name or path, else its ID.
    pub fn label(&self) -> String {
        for key in ["title", "name", "path"] {
            if let Some((_, v)) = self.attrs.iter().find(|(k, _)| k == key) {
                return v.trim_matches('"').to_string();
            }
        }
        format!("{}…", &self.id[..12.min(self.id.len())])
    }
}

impl From<RecordView> for Record {
    fn from(r: RecordView) -> Self {
        Record { id: r.id, attrs: r.attrs.into_iter().map(|(k, v)| (k, v.to_string())).collect(), blocks: r.blocks }
    }
}

/// One version of a record (None: it didn't exist then, or was deleted).
#[derive(Debug, Clone)]
pub struct Version {
    pub version: u64,
    pub time_ms: u64,
    pub record: Option<Record>,
}

/// What changed from `older` to `newer`, one line per change.
pub fn changes(newer: Option<&Record>, older: Option<&Record>) -> Vec<String> {
    let (Some(newer), Some(older)) = (newer, older) else {
        return vec![if newer.is_none() { "Deleted" } else { "Created" }.to_string()];
    };
    let mut out = Vec::new();
    for (k, v) in &newer.attrs {
        match older.attrs.iter().find(|(ok, _)| ok == k) {
            Some((_, ov)) if ov == v => {}
            Some(_) => out.push(format!("{k} → {v}")),
            None => out.push(format!("Added {k} = {v}")),
        }
    }
    for (k, _) in &older.attrs {
        if !newer.attrs.iter().any(|(nk, _)| nk == k) {
            out.push(format!("Removed {k}"));
        }
    }
    if newer.blocks != older.blocks {
        out.push("Content changed".into());
    }
    if out.is_empty() {
        out.push("No change".into());
    }
    out
}

/// Splits a search line into query terms: whitespace separates, "double
/// quotes" keep spaces in a value (`album="Prangli 2026"`).
pub fn terms(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in line.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                cur.push(ch);
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
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

    fn client(&self) -> Result<Client, String> {
        self.rt.block_on(Client::connect(&self.store)).map_err(|e| e.to_string())
    }

    fn call(&self, req: Request) -> Result<Response, String> {
        let mut c = self.client()?;
        self.rt.block_on(c.call(req)).map_err(|e| e.to_string())
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

    /// Records of `ns` matching a search line (all of them if it's empty).
    pub fn query(&self, ns: &str, line: &str) -> Result<Vec<Record>, String> {
        match self.call(Request::Query { ns: ns.into(), terms: terms(line), at: None })? {
            Response::Records { records } => Ok(records.into_iter().map(Record::from).collect()),
            _ => Err("unexpected answer from retifsd".into()),
        }
    }

    /// Every version of a record, newest first.
    pub fn history(&self, ns: &str, id: &str) -> Result<Vec<Version>, String> {
        match self.call(Request::RecordHistory { ns: ns.into(), id: id.into() })? {
            Response::History { entries } => Ok(entries
                .into_iter()
                .map(|e| Version { version: e.version, time_ms: e.time_ms, record: e.record.map(Record::from) })
                .collect()),
            _ => Err("unexpected answer from retifsd".into()),
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
    use super::*;

    #[test]
    fn search_terms_keep_quoted_spaces() {
        assert_eq!(terms(r#"album="Prangli 2026"  type=image"#), vec![r#"album="Prangli 2026""#, "type=image"]);
        assert!(terms("   ").is_empty());
    }

    #[test]
    fn changes_between_versions() {
        let r = |attrs: &[(&str, &str)]| Record { id: "x".into(), attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), blocks: vec![] };
        let old = r(&[("title", "\"a\""), ("n", "1")]);
        let new = r(&[("title", "\"b\""), ("tag", "\"x\"")]);
        assert_eq!(changes(Some(&new), Some(&old)), vec!["title → \"b\"", "Added tag = \"x\"", "Removed n"]);
        assert_eq!(changes(None, Some(&old)), vec!["Deleted"]);
        assert_eq!(changes(Some(&new), None), vec!["Created"]);
    }
}
