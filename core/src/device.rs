//! This device: Reticulum (the shared instance retifs runs on), the
//! services, and the node's settings. Reticulum is asked through `prnsd`'s
//! own command line (`status --json`, `interfaces …`); the services are
//! systemd user units on Linux (the .deb's) and, on Windows, processes the
//! utility itself starts at login (the installer's); the settings are the
//! daemon's `retifs.toml`, edited in place so its comments stay.

use std::path::{Path, PathBuf};
use std::process::Command;

/// One Reticulum interface, as `prnsd status --json` shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Interface {
    pub name: String,
    /// "TcpClient", "LocalServer", "AutoInterface", …
    pub kind: String,
    pub up: bool,
    pub rx: u64,
    pub tx: u64,
}

impl Interface {
    /// The machine's own shared-instance socket: not a way out.
    pub fn is_local(&self) -> bool {
        self.kind == "LocalServer" || self.kind == "LocalClient" || self.name.starts_with("Shared Instance")
    }
}

/// Reticulum on this machine.
#[derive(Debug, Clone, Default)]
pub struct Reticulum {
    /// The prnsd found to ask (the bundled one first).
    pub prnsd: Option<PathBuf>,
    /// A shared instance answered.
    pub running: bool,
    pub transport_id: Option<String>,
    pub uptime_s: Option<f64>,
    pub interfaces: Vec<Interface>,
    pub error: Option<String>,
}

/// The prnsd to use: the one the retifs package bundles, one on the PATH,
/// or (Windows) the managed one.
pub fn prnsd() -> Option<PathBuf> {
    // Where an app ships it itself (Android: among its native libraries).
    if let Some(p) = std::env::var_os("RETIFS_PRNSD").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let bundled = if cfg!(windows) {
        install_dir().join("prnsd.exe")
    } else if cfg!(target_os = "haiku") {
        install_dir().join("bin").join("prnsd") // in the app's folder (bin/), as on Windows
    } else {
        PathBuf::from("/usr/lib/retifs/prnsd")
    };
    if bundled.is_file() {
        return Some(bundled);
    }
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        for name in ["prnsd\\prnsd-managed.exe"] {
            let p = PathBuf::from(&base).join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let exe = if cfg!(windows) { "prnsd.exe" } else { "prnsd" };
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(exe)).find(|p| p.is_file()))
}

/// Parses `prnsd status --json`.
pub fn parse_status(json: &str) -> Result<Reticulum, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("prnsd status: {e}"))?;
    let interfaces = v["interfaces"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|i| Interface {
                    name: i["name"].as_str().unwrap_or("?").to_string(),
                    kind: i["type"].as_str().unwrap_or("").to_string(),
                    up: i["status"].as_bool().unwrap_or(false),
                    rx: i["rxb"].as_u64().unwrap_or(0),
                    tx: i["txb"].as_u64().unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Reticulum {
        prnsd: None,
        running: true,
        transport_id: v["transport_id"].as_str().map(str::to_string),
        uptime_s: v["transport_uptime"].as_f64(),
        interfaces,
        error: None,
    })
}

/// Asks the shared instance how it is.
pub fn reticulum() -> Reticulum {
    let Some(bin) = prnsd() else {
        return Reticulum { error: Some("prnsd not found (install the retifs package, or prnsd)".into()), ..Default::default() };
    };
    let out = Command::new(&bin).args(["status", "--json"]).output();
    let mut r = match out {
        Ok(o) if o.status.success() => parse_status(&String::from_utf8_lossy(&o.stdout)).unwrap_or_else(|e| Reticulum { error: Some(e), ..Default::default() }),
        Ok(o) => Reticulum { error: Some(first_line(&o.stderr, "no Reticulum shared instance is running")), ..Default::default() },
        Err(e) => Reticulum { error: Some(e.to_string()), ..Default::default() },
    };
    r.prnsd = Some(bin);
    r
}

fn first_line(bytes: &[u8], or: &str) -> String {
    String::from_utf8_lossy(bytes).lines().find(|l| !l.trim().is_empty()).map(str::to_string).unwrap_or_else(|| or.to_string())
}

/// A configured interface (`prnsd interfaces list`): name, type, enabled.
#[derive(Debug, Clone, PartialEq)]
pub struct Configured {
    pub name: String,
    pub kind: String,
    pub enabled: bool,
}

/// Parses `prnsd interfaces list` ("1. RTR-3: TCPClientInterface (enabled)").
pub fn parse_list(text: &str) -> Vec<Configured> {
    text.lines()
        .filter_map(|l| {
            let (_, rest) = l.trim().split_once(". ")?;
            let (name, rest) = rest.rsplit_once(": ")?;
            let (kind, state) = rest.rsplit_once(" (")?;
            Some(Configured { name: name.to_string(), kind: kind.trim().to_string(), enabled: state.trim_end_matches(')') == "enabled" })
        })
        .collect()
}

/// The interfaces in the Reticulum configuration, enabled or not.
pub fn configured() -> Result<Vec<Configured>, String> {
    let bin = prnsd().ok_or("prnsd not found")?;
    let o = Command::new(bin).args(["interfaces", "list"]).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(parse_list(&String::from_utf8_lossy(&o.stdout))) } else { Err(first_line(&o.stderr, "prnsd interfaces list failed")) }
}

fn run_prnsd(args: &[&str]) -> Result<(), String> {
    let bin = prnsd().ok_or("prnsd not found")?;
    let o = Command::new(bin).args(args).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(first_line(&o.stderr, "prnsd failed")) }
}

/// Changes the Reticulum configuration and puts it to use: a managed prnsd
/// (`prnsd start`, Windows) applies it live; the package's service runs
/// `prnsd run`, which can't, so it's restarted.
fn reconfigure(args: &[&str]) -> Result<(), String> {
    // An app that runs prnsd itself (Android) restarts it after the change.
    if std::env::var("RETIFS_PRNSD_APPLY").as_deref() == Ok("restart") {
        return run_prnsd(args);
    }
    if service_state(BUNDLED).as_deref() == Some("active") {
        run_prnsd(args)?;
        service(BUNDLED, "restart")?;
        // retifsd doesn't attach to a new shared instance by itself (on
        // Linux the unit is PartOf prnsd's): restarted after it.
        #[cfg(target_os = "haiku")]
        {
            std::thread::sleep(std::time::Duration::from_secs(2));
            service(RETIFSD, "restart")?;
        }
        Ok(())
    } else {
        let mut with = args.to_vec();
        with.push("--apply");
        run_prnsd(&with)
    }
}

/// The package's Reticulum service.
#[cfg(not(any(windows, target_os = "haiku")))]
const BUNDLED: &str = "retifs-prnsd.service";
#[cfg(target_os = "haiku")]
const BUNDLED: &str = "x-vnd.retifs-prnsd";
#[cfg(windows)]
const BUNDLED: &str = "prnsd";

/// Adds a TCP connection to another Reticulum node.
pub fn add_tcp(name: &str, host: &str, port: u16) -> Result<(), String> {
    let port = port.to_string();
    reconfigure(&["interfaces", "add", "TCPClientInterface", "--name", name, "--target-host", host, "--target-port", &port])
}

pub fn set_interface(name: &str, enabled: bool) -> Result<(), String> {
    reconfigure(&["interfaces", if enabled { "enable" } else { "disable" }, name])
}

pub fn remove_interface(name: &str) -> Result<(), String> {
    reconfigure(&["interfaces", "remove", name, "--yes"])
}

/// The user services the retifs package installs (Linux, systemd).
#[cfg(not(any(windows, target_os = "haiku")))]
pub const SERVICES: &[(&str, &str)] = &[("retifsd.service", "retifs daemon"), ("retifs-prnsd.service", "Reticulum (bundled prnsd)"), ("retifs-gateway.service", "Libraries as local websites")];
/// On Windows: the programs next to the utility, which it starts itself.
#[cfg(windows)]
pub const SERVICES: &[(&str, &str)] = &[("retifsd", "retifs daemon"), ("prnsd", "Reticulum (bundled prnsd)")];
/// On Haiku: launch_daemon jobs of the user's (`~/config/settings/launch/retifs`,
/// or a package's `data/user_launch`), started at login.
#[cfg(target_os = "haiku")]
pub const SERVICES: &[(&str, &str)] = &[("x-vnd.retifs-daemon", "retifs daemon"), ("x-vnd.retifs-prnsd", "Reticulum (bundled prnsd)")];

/// The service retifs itself runs as.
pub const RETIFSD: &str = SERVICES[0].0;

/// A user service's state ("active", "inactive", "failed", …), None where
/// there's no systemd.
#[cfg(not(any(windows, target_os = "haiku")))]
pub fn service_state(unit: &str) -> Option<String> {
    let o = Command::new("systemctl").args(["--user", "is-active", unit]).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// `start`, `stop` or `restart` a user service.
#[cfg(not(any(windows, target_os = "haiku")))]
pub fn service(unit: &str, action: &str) -> Result<(), String> {
    let o = Command::new("systemctl").args(["--user", action, unit]).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(first_line(&o.stderr, "systemctl failed")) }
}

/// A launch_daemon job's state: "active" while it runs, "inactive" when it
/// is known but not running, None when there's no such job.
#[cfg(target_os = "haiku")]
pub fn service_state(job: &str) -> Option<String> {
    let o = Command::new("launch_roster").args(["info", job]).output().ok()?;
    if !o.status.success() {
        return None;
    }
    let info = String::from_utf8_lossy(&o.stdout);
    Some(if job_running(&info) { "active" } else { "inactive" }.to_string())
}

/// Whether `launch_roster info` says the job runs (`running = bool(true)`).
pub fn job_running(info: &str) -> bool {
    info.lines().any(|l| {
        let l = l.trim();
        l.starts_with("running") && l.contains("bool(true)")
    })
}

/// `start`, `stop` or `restart` a launch_daemon job.
#[cfg(target_os = "haiku")]
pub fn service(job: &str, action: &str) -> Result<(), String> {
    let o = Command::new("launch_roster").args([action, job]).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(first_line(&o.stderr, "launch_roster failed")) }
}

/// Where the utility is installed, with the programs it starts (Windows, Haiku).
pub fn install_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."))
}

/// Whether a Reticulum shared instance listens on this machine (its TCP
/// port, as on Windows).
pub fn shared_instance_up() -> bool {
    std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], 37428).into(), std::time::Duration::from_millis(400)).is_ok()
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
fn running(exe: &str) -> bool {
    use std::os::windows::process::CommandExt;
    Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {exe}"), "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains(&exe.to_ascii_lowercase()))
        .unwrap_or(false)
}

/// A program's state: "active" while it runs ("prnsd": this install's own,
/// not another prnsd the user runs).
#[cfg(windows)]
pub fn service_state(name: &str) -> Option<String> {
    Some(if running(&format!("{name}.exe")) { "active" } else { "inactive" }.to_string())
}

/// `start`, `stop` or `restart` one of the programs next to the utility,
/// hidden, its output in `%LOCALAPPDATA%\retifs\logs\<name>.log`.
#[cfg(windows)]
pub fn service(name: &str, action: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let exe = format!("{name}.exe");
    if action == "stop" || action == "restart" {
        let _ = Command::new("taskkill").args(["/IM", &exe, "/F"]).creation_flags(CREATE_NO_WINDOW).output();
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    if action == "start" || action == "restart" {
        if name == "prnsd" && shared_instance_up() {
            return Err("another Reticulum runs on this machine already".into());
        }
        let logs = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("retifs").join("logs");
        std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
        let log = std::fs::OpenOptions::new().create(true).append(true).open(logs.join(format!("{name}.log"))).map_err(|e| e.to_string())?;
        let args: &[&str] = if name == "prnsd" { &["run"] } else { &[] };
        Command::new(install_dir().join(&exe))
            .args(args)
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            // No console, and a process group of its own.
            .creation_flags(CREATE_NO_WINDOW | 0x0000_0200)
            .spawn()
            .map_err(|e| format!("{exe}: {e}"))?;
    }
    Ok(())
}

/// At login (Windows): Reticulum where none runs yet, then retifs. Says what
/// it did.
#[cfg(windows)]
pub fn supervise() -> Vec<String> {
    let mut did = Vec::new();
    if !shared_instance_up() && install_dir().join("prnsd.exe").is_file() {
        match service("prnsd", "start") {
            Ok(()) => did.push("started Reticulum (prnsd)".to_string()),
            Err(e) => did.push(format!("Reticulum: {e}")),
        }
        for _ in 0..30 {
            if shared_instance_up() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
    if !running("retifsd.exe") {
        match service("retifsd", "start") {
            Ok(()) => did.push("started retifsd".to_string()),
            Err(e) => did.push(format!("retifsd: {e}")),
        }
    }
    did
}

/// The settings the utility offers, of `retifs.toml`'s: (section, key, what it is).
pub const SETTINGS: [(&str, &str, &str); 7] = [
    ("node", "device", "This device's name"),
    ("node", "site", "Where it is (one copy of its files is kept at another site)"),
    ("store", "max_store", "Store limit"),
    ("store", "min_free", "Always leave this much free on the disk"),
    ("store", "no_storage", "Keep nothing of my own here (only a keyring)"),
    ("placement", "hold_for_others", "Hold others' encrypted copies up to (0: none)"),
    ("placement", "min_copies", "Copies of each of my files, this one included"),
];

pub fn settings_file(store: &Path) -> PathBuf {
    store.join("retifs.toml")
}

/// A setting's value in the file (as written, e.g. `"50G"`, `3`, `true`),
/// None while it's commented out (the default).
pub fn get_setting(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current = "";
    for line in text.lines() {
        let t = line.trim();
        if let Some(s) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            current = s.trim();
        } else if current == section {
            if let Some((k, v)) = t.split_once('=') {
                if k.trim() == key {
                    return Some(strip_comment(v).trim().to_string());
                }
            }
        }
    }
    None
}

fn strip_comment(v: &str) -> &str {
    // A value never holds '#' outside quotes in these settings.
    let mut quoted = false;
    for (i, c) in v.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &v[..i],
            _ => {}
        }
    }
    v
}

/// Sets a setting (a TOML value: `"50G"`, `3`, `true`), or with None puts
/// it back to its default (commented out). The line's own comment stays; a
/// key missing from the file is added to its section (made if missing).
pub fn set_setting(text: &str, section: &str, key: &str, value: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut done = false;
    let mut section_end: Option<usize> = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(s) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if current == section && section_end.is_none() {
                // After its last line that isn't blank.
                let mut at = out.len();
                while at > 0 && out[at - 1].trim().is_empty() {
                    at -= 1;
                }
                section_end = Some(at);
            }
            current = s.trim().to_string();
            out.push(line.to_string());
            continue;
        }
        if current == section && !done {
            let body = t.strip_prefix('#').map(str::trim_start).unwrap_or(t);
            if let Some((k, rest)) = body.split_once('=') {
                if k.trim() == key {
                    let old = strip_comment(rest);
                    let note = rest[old.len()..].trim();
                    let pad = |s: String| if note.is_empty() { s } else { format!("{s:<28} {note}") };
                    out.push(match value {
                        Some(v) => pad(format!("{key} = {v}")),
                        None if t.starts_with('#') => line.to_string(),
                        None => pad(format!("# {key} = {}", old.trim())),
                    });
                    done = true;
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }
    if current == section && section_end.is_none() {
        section_end = Some(out.len());
    }
    if !done {
        if let Some(v) = value {
            match section_end {
                Some(at) => out.insert(at, format!("{key} = {v}")),
                None => {
                    out.push(String::new());
                    out.push(format!("[{section}]"));
                    out.push(format!("{key} = {v}"));
                }
            }
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// A value typed by the user, as TOML: numbers and true/false as they are,
/// anything else quoted.
pub fn toml_value(typed: &str) -> String {
    let t = typed.trim();
    if t == "true" || t == "false" || t.parse::<i64>().is_ok() || t.parse::<f64>().is_ok() {
        t.to_string()
    } else {
        format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// A value from the file, as the user reads it (quotes off).
pub fn shown_value(v: &str) -> String {
    v.trim().trim_matches('"').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# retifs node settings.\n\n[store]\n# max_store = \"...\"          # store limit\n# no_storage = false         # keep nothing\n\n[node]\n# device = \"...\"             # this device's name\n";

    #[test]
    fn settings_are_set_and_put_back_with_their_comments() {
        let s = set_setting(FILE, "store", "max_store", Some("\"50G\""));
        assert!(s.contains("max_store = \"50G\"            # store limit"), "{s}");
        assert_eq!(get_setting(&s, "store", "max_store").as_deref(), Some("\"50G\""));
        assert_eq!(get_setting(&s, "node", "max_store"), None, "per section");
        let back = set_setting(&s, "store", "max_store", None);
        assert!(back.contains("# max_store = \"50G\""), "{back}");
        assert_eq!(get_setting(&back, "store", "max_store"), None);
        assert_eq!(set_setting(FILE, "node", "device", None), FILE, "a default stays as it was");
    }

    #[test]
    fn a_missing_key_or_section_is_added() {
        let s = set_setting(FILE, "store", "cache_max", Some("\"1G\""));
        assert!(s.contains("cache_max = \"1G\"\n\n[node]"), "{s}");
        let s = set_setting(FILE, "placement", "min_copies", Some("2"));
        assert!(s.ends_with("[placement]\nmin_copies = 2\n"), "{s}");
        assert_eq!(get_setting(&s, "placement", "min_copies").as_deref(), Some("2"));
    }

    #[test]
    fn typed_values_become_toml() {
        assert_eq!(toml_value("3"), "3");
        assert_eq!(toml_value("true"), "true");
        assert_eq!(toml_value("50G"), "\"50G\"");
        assert_eq!(toml_value("Prangli \"home\""), "\"Prangli \\\"home\\\"\"");
        assert_eq!(shown_value("\"50G\""), "50G");
    }

    #[test]
    fn prnsd_interfaces_list() {
        let l = parse_list("1. RTR-3: TCPClientInterface (enabled)
2. Hopspot USB: PrnsUsbAuto (disabled)
");
        assert_eq!(l, vec![
            Configured { name: "RTR-3".into(), kind: "TCPClientInterface".into(), enabled: true },
            Configured { name: "Hopspot USB".into(), kind: "PrnsUsbAuto".into(), enabled: false },
        ]);
    }

    #[test]
    fn launch_roster_info() {
        let running = "Details: BMessage(0x0) {\n        name = string(\"x-vnd.retifs-daemon\", 20 bytes)\n        running = bool(true)\n        launched = bool(true)\n";
        assert!(job_running(running));
        assert!(!job_running(&running.replace("running = bool(true)", "running = bool(false)")));
    }

    #[test]
    fn prnsd_status_json() {
        let json = r#"{"interfaces":[{"name":"Shared Instance[09699a59]","type":"LocalServer","status":true,"rxb":10,"txb":20},{"name":"Backbone","type":"TcpClient","status":false,"rxb":0,"txb":5}],"transport_id":"832c","transport_uptime":626.5}"#;
        let r = parse_status(json).unwrap();
        assert_eq!((r.running, r.transport_id.as_deref(), r.uptime_s), (true, Some("832c"), Some(626.5)));
        assert_eq!(r.interfaces.len(), 2);
        assert!(r.interfaces[0].is_local() && !r.interfaces[1].is_local());
        assert_eq!((r.interfaces[1].name.as_str(), r.interfaces[1].up, r.interfaces[1].tx), ("Backbone", false, 5));
    }
}
