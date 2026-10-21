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

use std::path::PathBuf;

pub use libretifs::{NsView, StoreUsage};
use libretifs::{Client, Request, Response};

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
