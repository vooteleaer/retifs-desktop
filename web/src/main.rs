//! retifs-web: a node's web UI (retifs ADR-0037), for a NAS or a container
//! without a screen. One page and a JSON API over the same `core::api`
//! calls the Android and Haiku front ends make.
//!
//!     retifs-web [--listen ADDR:PORT] [--allow CIDR,…] [--host NAME,…] [--pairing] [--store DIR]
//!     retifs-web --pair-link            (prints the pairing link again)
//!
//! Who may use it is the installer's choice:
//! - `--listen` (`RETIFS_WEB_LISTEN`, default `127.0.0.1:8741`) and `--allow`
//!   (`RETIFS_WEB_ALLOW`, default loopback only): anyone on the allowed
//!   networks may use it.
//! - `--pairing` (`RETIFS_WEB_PAIRING=1`), for networks others share too: a
//!   browser is let in once by a pairing link printed in the log
//!   (`/pair/<token>`), and then holds a session cookie.
//! - It's opened by an IP address or `localhost`; a name (`truenas.local`)
//!   only if given with `--host` (`RETIFS_WEB_HOSTS`): another site's page
//!   can't reach it through a name it points at this machine (DNS rebinding).
//!
//! Changes are POSTs with an `X-Retifs: 1` header.

use std::io::Read;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use retifs_desktop_core::api;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Request, Response, Server};

const PAGE: &str = include_str!("index.html");
const SCRIPT: &str = include_str!("app.js");
const COOKIE: &str = "retifs_web";
/// A request body at most (settings, a report's text).
const BODY_MAX: u64 = 256 << 10;

/// An allowed network: an address and its prefix length.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Net(IpAddr, u8);

impl Net {
    fn parse(s: &str) -> Result<Net, String> {
        let s = s.trim();
        let (a, len) = match s.split_once('/') {
            Some((a, l)) => (a, Some(l.parse::<u8>().map_err(|_| format!("bad prefix in {s}"))?)),
            None => (s, None),
        };
        let ip: IpAddr = a.parse().map_err(|_| format!("bad address in {s}"))?;
        let max = if ip.is_ipv4() { 32 } else { 128 };
        let len = len.unwrap_or(max);
        if len > max {
            return Err(format!("prefix too long in {s}"));
        }
        Ok(Net(ip, len))
    }

    fn contains(&self, ip: IpAddr) -> bool {
        // An IPv4 client may come as ::ffff:a.b.c.d on a dual-stack socket.
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
            v4 => v4,
        };
        match (self.0, ip) {
            (IpAddr::V4(n), IpAddr::V4(a)) => {
                let mask = if self.1 == 0 { 0 } else { u32::MAX << (32 - self.1) };
                u32::from(n) & mask == u32::from(a) & mask
            }
            (IpAddr::V6(n), IpAddr::V6(a)) => {
                let mask = if self.1 == 0 { 0 } else { u128::MAX << (128 - self.1) };
                u128::from(n) & mask == u128::from(a) & mask
            }
            _ => false,
        }
    }
}

fn parse_allow(s: &str) -> Result<Vec<Net>, String> {
    s.split(',').filter(|p| !p.trim().is_empty()).map(Net::parse).collect()
}

struct State {
    dir: PathBuf,
    allow: Vec<Net>,
    listen: String,
    /// Browsers must be let in by a pairing link (else the allowed networks are trusted).
    pairing: bool,
    /// Names (besides IP addresses and `localhost`) it may be opened by.
    hosts: Vec<String>,
    /// Hashes of the sessions let in.
    sessions: Mutex<Vec<String>>,
}

fn random_token() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("OS entropy");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|x| format!("{x:02x}")).collect()
}

/// Writes a file only its owner can read.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let part = path.with_extension("part");
    std::fs::write(&part, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&part, path)
}

impl State {
    /// The pairing token now (made if there's none).
    fn pair_token(&self) -> String {
        let p = self.dir.join("pair");
        match std::fs::read_to_string(&p) {
            Ok(t) if t.trim().len() == 32 => t.trim().to_string(),
            _ => {
                let t = random_token();
                let _ = write_private(&p, &t);
                t
            }
        }
    }

    fn pair_link(&self) -> String {
        format!("http://{}/pair/{}", self.listen, self.pair_token())
    }

    /// Lets the browser holding `token` in: a session, and a new pairing token.
    fn pair(&self, token: &str) -> Option<String> {
        if token.len() != 32 || token != self.pair_token() {
            return None;
        }
        let session = random_token();
        let mut s = self.sessions.lock().unwrap();
        s.push(hash(&session));
        let _ = write_private(&self.dir.join("sessions"), &(s.join("\n") + "\n"));
        let _ = std::fs::remove_file(self.dir.join("pair"));
        println!("a browser was let in; the next pairing link: {}", self.pair_link());
        Some(session)
    }

    fn signed_in(&self, req: &Request) -> bool {
        if !self.pairing {
            return true;
        }
        let Some(session) = cookie(req, COOKIE) else { return false };
        let h = hash(&session);
        // Read again: deleting the file signs every browser out.
        let now: Vec<String> = std::fs::read_to_string(self.dir.join("sessions")).unwrap_or_default().lines().map(str::to_string).collect();
        let ok = now.iter().any(|x| *x == h);
        *self.sessions.lock().unwrap() = now;
        ok
    }
}

/// Whether the request names this machine by an address, `localhost` or an
/// allowed name: a name another site points here (DNS rebinding) isn't one.
fn host_ok(req: &Request, names: &[String]) -> bool {
    let Some(h) = req.headers().iter().find(|h| h.field.equiv("Host")) else { return false };
    let host = h.value.as_str().trim();
    // [v6]:port, v4:port, name:port, or without the port.
    let bare = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else if host.matches(':').count() == 1 {
        host.split(':').next().unwrap_or("")
    } else {
        host
    };
    let bare = bare.to_ascii_lowercase();
    bare.parse::<IpAddr>().is_ok() || bare == "localhost" || names.iter().any(|n| n.eq_ignore_ascii_case(&bare))
}

fn cookie(req: &Request, name: &str) -> Option<String> {
    req.headers()
        .iter()
        .filter(|h| h.field.equiv("Cookie"))
        .flat_map(|h| h.value.as_str().split(';').map(str::trim).map(String::from).collect::<Vec<_>>())
        .find_map(|c| c.strip_prefix(&format!("{name}=")).map(String::from))
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("ASCII header")
}

/// Headers every answer gets: not cached, not framed, nothing from elsewhere.
fn guarded<R: Read>(r: Response<R>) -> Response<R> {
    r.with_header(header("Cache-Control", "no-store"))
        .with_header(header("X-Frame-Options", "DENY"))
        .with_header(header("X-Content-Type-Options", "nosniff"))
        .with_header(header("Referrer-Policy", "no-referrer"))
        .with_header(header(
            "Content-Security-Policy",
            "default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; form-action 'none'; frame-ancestors 'none'; base-uri 'none'",
        ))
}

fn json_answer(v: &Value, status: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    guarded(Response::from_data(v.to_string().into_bytes()).with_status_code(status).with_header(header("Content-Type", "application/json")))
}

fn ok_or(r: Result<Value, String>) -> (Value, u16) {
    match r {
        Ok(v) => (v, 200),
        Err(e) => (json!({ "error": e }), 400),
    }
}

fn s<'a>(body: &'a Value, key: &str) -> &'a str {
    body.get(key).and_then(Value::as_str).unwrap_or("")
}

/// A GET's query value.
fn q(url: &str, key: &str) -> String {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| unescape(v))
        .unwrap_or_default()
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                if let Ok(x) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(x);
                    i += 2;
                } else {
                    out.push(b'%');
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The JSON API: what each path does with the core.
fn api_call(method: &Method, url: &str, body: &Value) -> (Value, u16) {
    let path = url.split('?').next().unwrap_or("");
    let done = |r: Result<(), String>| ok_or(r.map(|_| json!({ "ok": true })));
    let text = |r: Result<String, String>| ok_or(r.map(|t| json!({ "text": t })));
    match (method, path) {
        (Method::Get, "/api/status") => (api::status(), 200),
        (Method::Get, "/api/heard") => (api::heard(), 200),
        (Method::Get, "/api/reticulum") => (api::reticulum(), 200),
        (Method::Get, "/api/settings") => (api::settings(), 200),
        (Method::Get, "/api/members") => (api::members(&q(url, "ns")), 200),
        (Method::Get, "/api/invites") => (api::invites(&q(url, "ns")), 200),
        (Method::Get, "/api/paths") => (api::paths(&q(url, "ns")), 200),
        (Method::Get, "/api/link") => text(api::link(&q(url, "ns"))),
        (Method::Get, "/api/reports") => (api::crash_reports(), 200),
        (Method::Get, "/api/report") => text(api::crash_text(&q(url, "path"))),
        (Method::Post, "/api/open") => text(api::open_link(s(body, "link"))),
        (Method::Post, "/api/join") => text(api::join(s(body, "address"))),
        (Method::Post, "/api/create") => done(api::create_library(s(body, "name"), &api::purpose_words(s(body, "for")))),
        (Method::Post, "/api/keep") => done(api::keep(s(body, "ns"), body["all"].as_bool().unwrap_or(false))),
        (Method::Post, "/api/favorite") => done(api::favorite(s(body, "ns"), body["on"].as_bool().unwrap_or(false))),
        (Method::Post, "/api/announce") => done(api::announce(s(body, "ns"), body["on"].as_bool().unwrap_or(false))),
        (Method::Post, "/api/invite") => text(api::invite(s(body, "ns"), s(body, "role"), body["days"].as_u64().unwrap_or(7) as u32)),
        (Method::Post, "/api/cancel-invite") => done(api::cancel_invite(s(body, "ns"), s(body, "id"))),
        (Method::Post, "/api/remove-member") => done(api::remove_member(s(body, "ns"), s(body, "writer"))),
        (Method::Post, "/api/leave") => done(api::leave(s(body, "ns"))),
        (Method::Post, "/api/delete") => done(api::delete_library(s(body, "ns"))),
        (Method::Post, "/api/settings") => done(api::save_settings(&body["values"].to_string())),
        (Method::Post, "/api/report") => done(api::crash_report("user", "web", s(body, "text"))),
        _ => (json!({ "error": "no such call" }), 404),
    }
}

fn handle(state: &State, mut req: Request) {
    let peer = req.remote_addr().map(|a| a.ip());
    if !peer.is_some_and(|ip| state.allow.iter().any(|n| n.contains(ip))) {
        let _ = req.respond(guarded(Response::from_string("Not from here.\n").with_status_code(403)));
        return;
    }
    if !host_ok(&req, &state.hosts) {
        let _ = req.respond(guarded(Response::from_string("Open it by this machine's address (a name only if retifs-web was given it with --host).\n").with_status_code(403)));
        return;
    }
    let url = req.url().to_string();
    let method = req.method().clone();

    if !state.pairing && url.starts_with("/pair/") {
        let _ = req.respond(guarded(Response::from_string("").with_status_code(303).with_header(header("Location", "/"))));
        return;
    }
    if let Some(token) = url.strip_prefix("/pair/") {
        let answer = match state.pair(token) {
            Some(session) => Response::from_string("")
                .with_status_code(303)
                .with_header(header("Location", "/"))
                .with_header(header("Set-Cookie", &format!("{COOKIE}={session}; HttpOnly; SameSite=Strict; Path=/; Max-Age=31536000"))),
            None => Response::from_string("This pairing link was used already, or isn't this node's. retifs-web prints the current one in its log (or: retifs-web --pair-link).\n").with_status_code(403),
        };
        let _ = req.respond(guarded(answer));
        return;
    }

    let signed_in = state.signed_in(&req);
    if url == "/" || url.starts_with("/?") {
        let page = if signed_in { PAGE.to_string() } else { NOT_PAIRED.to_string() };
        let _ = req.respond(guarded(Response::from_string(page).with_header(header("Content-Type", "text/html; charset=utf-8"))));
        return;
    }
    if url == "/app.js" {
        let _ = req.respond(guarded(Response::from_string(SCRIPT).with_header(header("Content-Type", "text/javascript; charset=utf-8"))));
        return;
    }
    if !url.starts_with("/api/") {
        let _ = req.respond(guarded(Response::from_string("").with_status_code(404)));
        return;
    }
    if !signed_in {
        let _ = req.respond(json_answer(&json!({ "error": "not paired: open the pairing link from retifs-web's log" }), 401));
        return;
    }
    let body: Value = if method == Method::Post {
        // A cross-site form can't set this header (and no CORS is allowed).
        let ours = req.headers().iter().any(|h| h.field.equiv("X-Retifs") && h.value.as_str() == "1");
        if !ours {
            let _ = req.respond(json_answer(&json!({ "error": "missing X-Retifs header" }), 403));
            return;
        }
        let mut text = String::new();
        let _ = req.as_reader().take(BODY_MAX).read_to_string(&mut text);
        serde_json::from_str(&text).unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let (v, status) = api_call(&method, &url, &body);
    let _ = req.respond(json_answer(&v, status));
}

const NOT_PAIRED: &str = r#"<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>retifs</title>
<style>body{font:16px system-ui,sans-serif;max-width:36em;margin:4em auto;padding:0 1em;color:#222;background:#fafafa}@media(prefers-color-scheme:dark){body{color:#ddd;background:#1d1d20}}code{font-size:90%}</style>
<h1>retifs</h1><p>This browser isn't let in to this node yet. Open the pairing link that <code>retifs-web</code> printed in its log (on a TrueNAS app: <em>View Logs</em>), or print it again on the node with <code>retifs-web --pair-link</code>.</p>"#;

fn main() {
    let mut listen = std::env::var("RETIFS_WEB_LISTEN").unwrap_or_else(|_| "127.0.0.1:8741".into());
    let mut allow = std::env::var("RETIFS_WEB_ALLOW").unwrap_or_else(|_| "127.0.0.1/8,::1/128".into());
    let mut print_link = false;
    let mut pairing = std::env::var("RETIFS_WEB_PAIRING").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true"));
    let mut hosts: Vec<String> = std::env::var("RETIFS_WEB_HOSTS").unwrap_or_default().split(',').map(|h| h.trim().to_string()).filter(|h| !h.is_empty()).collect();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => listen = args.next().expect("--listen ADDR:PORT"),
            "--allow" => allow = args.next().expect("--allow CIDR,…"),
            "--store" => std::env::set_var("RETIFS_STORE", args.next().expect("--store DIR")),
            "--pair-link" => print_link = true,
            "--pairing" => pairing = true,
            "--host" => hosts.extend(args.next().expect("--host NAME,…").split(',').map(|h| h.trim().to_string()).filter(|h| !h.is_empty())),
            "-h" | "--help" => {
                println!("retifs-web [--listen ADDR:PORT] [--allow CIDR,…] [--host NAME,…] [--pairing] [--store DIR] | --pair-link");
                return;
            }
            other => {
                eprintln!("retifs-web: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let allow = parse_allow(&allow).unwrap_or_else(|e| {
        eprintln!("retifs-web: --allow: {e}");
        std::process::exit(2);
    });
    let dir = retifs_desktop_core::default_store().join("web");
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let sessions = std::fs::read_to_string(dir.join("sessions")).unwrap_or_default().lines().map(str::to_string).collect();
    let state = Arc::new(State { dir, allow, listen: listen.clone(), pairing, hosts, sessions: Mutex::new(sessions) });
    if print_link {
        println!("{}", state.pair_link());
        return;
    }
    let server = Server::http(&listen).unwrap_or_else(|e| {
        eprintln!("retifs-web: can't listen on {listen}: {e}");
        std::process::exit(1);
    });
    let nets: Vec<String> = state.allow.iter().map(|n| format!("{}/{}", n.0, n.1)).collect();
    println!("retifs-web on http://{listen}/, for {}", nets.join(", "));
    if state.pairing {
        println!("pairing link (open it once in the browser to let in; a new one follows each use): {}", state.pair_link());
    } else {
        println!("no pairing: anyone on the allowed networks may use it (--pairing asks for a pairing link)");
    }
    if listen.starts_with("0.0.0.0") || listen.starts_with("[::]") {
        println!("  (use this machine's address in place of {})", listen.rsplit_once(':').map(|(h, _)| h).unwrap_or(&listen));
    }
    for req in server.incoming_requests() {
        let state = state.clone();
        // Core calls block (some for seconds): each request its own thread.
        std::thread::spawn(move || handle(&state, req));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nets() {
        let lan = Net::parse("192.168.0.0/24").unwrap();
        assert!(lan.contains("192.168.0.18".parse().unwrap()));
        assert!(!lan.contains("192.168.1.18".parse().unwrap()));
        assert!(lan.contains("::ffff:192.168.0.7".parse().unwrap()));
        let tail = Net::parse("100.64.0.0/10").unwrap();
        assert!(tail.contains("100.75.58.112".parse().unwrap()));
        assert!(!tail.contains("100.128.0.1".parse().unwrap()));
        assert!(Net::parse("::1").unwrap().contains("::1".parse().unwrap()));
        assert!(Net::parse("0.0.0.0/0").unwrap().contains("8.8.8.8".parse().unwrap()));
        assert!(Net::parse("10.0.0.0/33").is_err());
        assert_eq!(parse_allow("127.0.0.1/8, ::1/128").unwrap().len(), 2);
    }

    #[test]
    fn queries() {
        assert_eq!(q("/api/members?ns=my%20music&x=1", "ns"), "my music");
        assert_eq!(q("/api/report?path=/rtr-2/a+b.txt", "path"), "/rtr-2/a b.txt");
        assert_eq!(q("/api/status", "ns"), "");
    }
}
