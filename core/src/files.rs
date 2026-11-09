//! Libraries as files, for a front end that shows them in the system's own
//! file places (Android's DocumentsProvider: the Files app and every app's
//! file picker). The same tree as the FUSE mount and Cloud Files
//! (`retifs-tree`): a record's `path` is where it is, records without one
//! are in Unfiled. Read and written through retifsd; nothing is kept here
//! but each library's tree, rebuilt when the library changes.

use std::{
    collections::HashMap,
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex},
};

use libretifs::{types, AttrValue, Request, Response, TxStep};
use retifs_tree::{self as tree, Node, Tree};

use crate::Daemon;

/// Content moves in pieces this large.
const PIECE: u64 = 4 << 20;
/// How long reading waits for content that isn't on this device yet.
const WAIT_MS: u64 = 60_000;

/// One file or folder.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    /// Its path in the library (`/a/b.txt`; `/` the library itself).
    pub path: String,
    pub dir: bool,
    pub size: u64,
    pub mtime_ms: Option<i64>,
}

/// Each library's tree, with the change count it was built at.
static TREES: Mutex<Option<HashMap<String, (u64, Arc<Tree>)>>> = Mutex::new(None);

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn entry(path: &str, node: &Node) -> Entry {
    let name = if path == "/" { String::new() } else { tree::name(path).to_string() };
    match node {
        Node::Dir { .. } => Entry { name, path: path.to_string(), dir: true, size: 0, mtime_ms: None },
        Node::File { size, mtime_ms, .. } => Entry { name, path: path.to_string(), dir: false, size: *size, mtime_ms: *mtime_ms },
    }
}

/// A path as the tree has it, or an error for one it can't (`..`, empty).
fn norm(path: &str) -> Result<String, String> {
    tree::normalize(path).ok_or_else(|| format!("not a path: {path}"))
}

impl Daemon {
    /// Library `ns` as a tree, as of now.
    fn tree(&self, ns: &str) -> Result<Arc<Tree>, String> {
        let mut c = self.client()?;
        let count = match self.request(&mut c, Request::Watch { ns: ns.into(), since: 0, timeout_ms: 0 })? {
            Response::Changed { count } => count,
            _ => return Err("unexpected answer".into()),
        };
        if let Some((at, t)) = TREES.lock().unwrap().get_or_insert_with(HashMap::new).get(ns) {
            if *at == count {
                return Ok(t.clone());
            }
        }
        let records = match self.request(&mut c, Request::Query { ns: ns.into(), terms: Vec::new(), at: None })? {
            Response::Records { records } => records,
            _ => return Err("unexpected answer".into()),
        };
        let t = Arc::new(Tree::build(&records));
        TREES.lock().unwrap().get_or_insert_with(HashMap::new).insert(ns.to_string(), (count, t.clone()));
        Ok(t)
    }

    fn forget_tree(ns: &str) {
        if let Some(t) = TREES.lock().unwrap().as_mut() {
            t.remove(ns);
        }
    }

    /// What's directly in folder `dir` of library `ns` (saved queries' folder left out).
    pub fn list(&self, ns: &str, dir: &str) -> Result<Vec<Entry>, String> {
        let dir = norm(dir)?;
        let t = self.tree(ns)?;
        match t.get(&dir) {
            Some(Node::Dir { .. }) => {}
            Some(Node::File { .. }) => return Err(format!("{dir} is a file")),
            None => return Err(format!("no {dir} in {ns}")),
        }
        Ok(t.children(&dir).into_iter().filter(|(_, p)| p != tree::QUERY_DIR).filter_map(|(_, p)| t.get(&p).map(|n| entry(&p, n))).collect())
    }

    /// The file or folder at `path`, if there's one.
    pub fn stat(&self, ns: &str, path: &str) -> Result<Option<Entry>, String> {
        let path = norm(path)?;
        Ok(self.tree(ns)?.get(&path).map(|n| entry(&path, n)))
    }

    /// The file at `path` into `out` (fetched from other nodes if it isn't here).
    pub fn read_file(&self, ns: &str, path: &str, out: &mut dyn Write) -> Result<u64, String> {
        let path = norm(path)?;
        let (cap, size) = match self.tree(ns)?.get(&path) {
            Some(Node::File { cap: Some(cap), size, .. }) => (cap.clone(), *size),
            Some(Node::File { cap: None, .. }) => return Ok(0), // empty
            Some(Node::Dir { .. }) => return Err(format!("{path} is a folder")),
            None => return Err(format!("no {path} in {ns}")),
        };
        let mut c = self.client()?;
        let mut offset = 0;
        while offset < size {
            let len = PIECE.min(size - offset);
            let req = Request::ReadRange { cap: cap.clone(), ns: Some(ns.into()), offset, len, wait_ms: WAIT_MS };
            match self.request(&mut c, req)? {
                Response::Range { data } if !data.is_empty() => {
                    out.write_all(&data).map_err(|e| e.to_string())?;
                    offset += data.len() as u64;
                }
                _ => return Err(format!("{path}: short read at {offset}")),
            }
        }
        Ok(offset)
    }

    /// `input` as the file at `path`: a new version of the record there, or
    /// a new record.
    pub fn write_file(&self, ns: &str, path: &str, input: &mut dyn Read) -> Result<(), String> {
        let path = norm(path)?;
        if tree::is_virtual(&path) || path == "/" {
            return Err(format!("{path} can't be written"));
        }
        let existing = match self.tree(ns)?.get(&path) {
            Some(Node::File { id, .. }) => Some(id.clone()),
            Some(Node::Dir { .. }) => return Err(format!("{path} is a folder")),
            None => None,
        };
        let mut c = self.client()?;
        let id = match existing {
            Some(id) => id,
            None => {
                let attrs = vec![("path".into(), AttrValue::Str(path.clone())), ("name".into(), AttrValue::Str(tree::name(&path).to_string()))];
                match self.request(&mut c, Request::RecordCreate { ns: ns.into(), attrs, data: None })? {
                    Response::Records { records } if !records.is_empty() => records[0].id.clone(),
                    _ => return Err("unexpected answer".into()),
                }
            }
        };
        let session = match self.request(&mut c, Request::WriteBegin { ns: ns.into(), id, truncate: true })? {
            Response::Session { session, .. } => session,
            _ => return Err("unexpected answer".into()),
        };
        let mut piece = vec![0u8; PIECE as usize];
        let mut offset = 0;
        loop {
            let n = input.read(&mut piece).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            if let Err(e) = self.request(&mut c, Request::WriteAt { session, offset, data: piece[..n].to_vec() }) {
                let _ = self.request(&mut c, Request::WriteAbort { session });
                return Err(e);
            }
            offset += n as u64;
        }
        self.request(&mut c, Request::WriteCommit { session, attrs: vec![("mtime".into(), AttrValue::Int(now_ms()))] })?;
        Self::forget_tree(ns);
        Ok(())
    }

    /// A new, empty folder at `path`.
    pub fn make_dir(&self, ns: &str, path: &str) -> Result<(), String> {
        let path = norm(path)?;
        if tree::is_virtual(&path) || path == "/" {
            return Err(format!("{path} can't be made"));
        }
        if self.tree(ns)?.get(&path).is_some() {
            return Err(format!("{path} is there already"));
        }
        let attrs = vec![
            ("path".into(), AttrValue::Str(path.clone())),
            ("type".into(), AttrValue::Str(types::DIRECTORY.into())),
            ("name".into(), AttrValue::Str(tree::name(&path).to_string())),
        ];
        let mut c = self.client()?;
        self.request(&mut c, Request::RecordCreate { ns: ns.into(), attrs, data: None })?;
        Self::forget_tree(ns);
        Ok(())
    }

    /// The file or folder at `path` gone, a folder with all in it, in one commit.
    pub fn remove(&self, ns: &str, path: &str) -> Result<(), String> {
        let path = norm(path)?;
        if path == "/" {
            return Err("a library is left, not deleted".into());
        }
        let t = self.tree(ns)?;
        let steps: Vec<TxStep> = match t.get(&path) {
            Some(Node::File { id, .. }) => vec![TxStep::Delete { id: id.clone() }],
            Some(Node::Dir { .. }) => t.records_under(&path).into_iter().map(|(_, id)| TxStep::Delete { id }).collect(),
            None => return Err(format!("no {path} in {ns}")),
        };
        if !steps.is_empty() {
            let mut c = self.client()?;
            self.request(&mut c, Request::Transaction { ns: ns.into(), steps })?;
        }
        Self::forget_tree(ns);
        Ok(())
    }

    /// Moves `from` to `to` (a new name, or another folder of the same
    /// library), everything below a folder with it, in one commit.
    pub fn rename(&self, ns: &str, from: &str, to: &str) -> Result<(), String> {
        let (from, to) = (norm(from)?, norm(to)?);
        if tree::is_virtual(&from) || tree::is_virtual(&to) || from == "/" || to == "/" {
            return Err("only files and folders with a place can be moved".into());
        }
        let t = self.tree(ns)?;
        if t.get(&to).is_some() {
            return Err(format!("{to} is there already"));
        }
        let new_name = tree::name(&to).to_string();
        let steps: Vec<TxStep> = match t.get(&from) {
            Some(Node::File { id, .. }) => vec![TxStep::Set {
                id: id.clone(),
                attrs: vec![("path".into(), AttrValue::Str(to.clone())), ("name".into(), AttrValue::Str(new_name))],
                unset: Vec::new(),
                data: None,
                blocks: None,
            }],
            Some(Node::Dir { .. }) => t
                .records_under(&from)
                .into_iter()
                .map(|(p, id)| {
                    let mut attrs = vec![("path".into(), AttrValue::Str(format!("{to}{}", &p[from.len()..])))];
                    if p == from {
                        attrs.push(("name".into(), AttrValue::Str(new_name.clone())));
                    }
                    TxStep::Set { id, attrs, unset: Vec::new(), data: None, blocks: None }
                })
                .collect(),
            None => return Err(format!("no {from} in {ns}")),
        };
        if !steps.is_empty() {
            let mut c = self.client()?;
            self.request(&mut c, Request::Transaction { ns: ns.into(), steps })?;
        }
        Self::forget_tree(ns);
        Ok(())
    }
}

/// Reads a file into `dest` (a cache file a front end hands out).
pub fn read_to(ns: &str, path: &str, dest: &Path) -> Result<u64, String> {
    let mut f = std::fs::File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    Daemon::new().read_file(ns, path, &mut f)
}

/// Writes `src` (a file a front end had written) as the file at `path`.
pub fn write_from(ns: &str, path: &str, src: &Path) -> Result<(), String> {
    let mut f = std::fs::File::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    Daemon::new().write_file(ns, path, &mut f)
}
