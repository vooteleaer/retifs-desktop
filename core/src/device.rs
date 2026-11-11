//! This device: Reticulum (the shared instance retifs runs on), the
//! services, and the node's settings. Any Reticulum implementation that
//! offers a shared instance will do (Python's `rnsd`, Prns's `prnsd`, …):
//! whether it runs is asked of the shared instance itself, and its
//! connections are read and changed in Reticulum's own configuration file,
//! which every implementation reads. Where `prnsd`'s command line is there,
//! it's used besides: per-interface traffic, and changes applied live. The
//! services are
//! systemd user units on Linux (the .deb's; from the Flatpak, the host's
//! through `flatpak-spawn --host`) and, on Windows, processes the
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
    /// A `prnsd` found to ask for details (Prns's), if there is one.
    pub prnsd: Option<PathBuf>,
    /// A shared instance answered.
    pub running: bool,
    pub transport_id: Option<String>,
    pub uptime_s: Option<f64>,
    pub interfaces: Vec<Interface>,
    pub error: Option<String>,
}

/// The prnsd to ask: Prns's own, installed apart from retifs (retifs never
/// brings one). Haiku: the Prns package's; Windows: the managed one; else
/// the PATH's, or cargo's.
pub fn prnsd() -> Option<PathBuf> {
    // The Flatpak: the host's, found there (its PATH, or cargo's).
    if in_flatpak() {
        let o = host("sh").args(["-c", r#"command -v prnsd || { [ -x "$HOME/.cargo/bin/prnsd" ] && echo "$HOME/.cargo/bin/prnsd"; }"#]).output().ok()?;
        let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
        return (o.status.success() && !p.is_empty()).then(|| PathBuf::from(p));
    }
    if cfg!(target_os = "haiku") && Path::new(HAIKU_PRNSD).is_file() {
        return Some(PathBuf::from(HAIKU_PRNSD));
    }
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        let p = PathBuf::from(base).join("prnsd\\prnsd-managed.exe");
        if p.is_file() {
            return Some(p);
        }
    }
    let exe = if cfg!(windows) { "prnsd.exe" } else { "prnsd" };
    let cargo = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(|h| PathBuf::from(h).join(".cargo").join("bin"));
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(cargo)
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
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

/// What's said when no shared instance answers.
pub const NO_SHARED_INSTANCE: &str = "no Reticulum shared instance is running (start Prns, rnsd or another Reticulum)";

/// Set once connections were changed in the configuration file with no
/// way to apply them live: the shared instance must be restarted.
static CHANGED_UNAPPLIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Asks the shared instance how it is: whether it answers, and with
/// `prnsd`'s command line, its interfaces and their traffic too.
pub fn reticulum() -> Reticulum {
    let mut r = match prnsd() {
        Some(bin) => {
            let out = prnsd_command(&bin).args(["status", "--json"]).output();
            let mut r = match out {
                Ok(o) if o.status.success() => parse_status(&String::from_utf8_lossy(&o.stdout)).unwrap_or_else(|e| Reticulum { error: Some(e), ..Default::default() }),
                _ => Reticulum::default(),
            };
            r.prnsd = Some(bin);
            r
        }
        None => Reticulum::default(),
    };
    // Whatever implementation runs it: the shared instance answers or not.
    if !r.running {
        r.running = shared_instance_up();
        r.error = None;
    }
    if !r.running {
        r.error = Some(NO_SHARED_INSTANCE.into());
    } else if CHANGED_UNAPPLIED.load(std::sync::atomic::Ordering::Relaxed) {
        r.error = Some("connections changed: restart Reticulum to use them".into());
    }
    r
}

/// One line saying how Reticulum is, whichever implementation runs it:
/// details where they're known, and the configuration file in use.
pub fn summary(r: &Reticulum) -> String {
    if !r.running {
        return r.error.clone().unwrap_or_else(|| NO_SHARED_INSTANCE.into());
    }
    let mut parts = Vec::new();
    if let Some(t) = &r.transport_id {
        parts.push(format!("transport {t}"));
    }
    if let Some(s) = r.uptime_s {
        parts.push(if s > 86_400.0 { format!("up {:.0} d", s / 86_400.0) } else { format!("up {:.0} h", s / 3600.0) });
    }
    if let Some(e) = &r.error {
        parts.push(e.clone());
    }
    if parts.is_empty() {
        parts.push("a shared instance answers".into());
    }
    parts.push(format!("configuration {}", config_path().display()));
    parts.join(" · ")
}

/// Reticulum's configuration file, where every implementation looks for
/// it: `/etc/reticulum/config`, `~/.config/reticulum/config`, then
/// `~/.reticulum/config` (made there if none exists).
pub fn config_path() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default();
    let mut candidates = Vec::new();
    if cfg!(unix) {
        candidates.push(PathBuf::from("/etc/reticulum/config"));
    }
    candidates.push(home.join(".config").join("reticulum").join("config"));
    let fallback = home.join(".reticulum").join("config");
    candidates.push(fallback.clone());
    candidates.into_iter().find(|p| p.is_file()).unwrap_or(fallback)
}

/// The `[[name]]` sections under `[interfaces]` in a Reticulum
/// configuration (ConfigObj): name, `type`, and `enabled` (or the older
/// `interface_enabled`; on when unset).
pub fn parse_config_interfaces(text: &str) -> Vec<Configured> {
    let mut out: Vec<Configured> = Vec::new();
    let mut in_interfaces = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('#') || t.is_empty() {
            continue;
        }
        if t.starts_with("[[") {
            if in_interfaces {
                out.push(Configured { name: t.trim_start_matches('[').trim_end_matches(']').trim().to_string(), kind: String::new(), enabled: true });
            }
            continue;
        }
        if t.starts_with('[') {
            in_interfaces = t.trim_matches(|c| c == '[' || c == ']').trim() == "interfaces";
            continue;
        }
        let (Some(last), Some((k, v))) = (out.last_mut().filter(|_| in_interfaces), t.split_once('=')) else { continue };
        let (k, v) = (k.trim(), v.trim().trim_matches('"'));
        match k {
            "type" => last.kind = v.to_string(),
            "enabled" | "interface_enabled" => last.enabled = matches!(v.to_lowercase().as_str(), "yes" | "true" | "on" | "1"),
            _ => {}
        }
    }
    out
}

/// The configuration with a TCP client interface added under `[interfaces]`.
pub fn config_add_tcp(text: &str, name: &str, host: &str, port: u16) -> Result<String, String> {
    if parse_config_interfaces(text).iter().any(|c| c.name == name) {
        return Err(format!("there's a connection named {name} already"));
    }
    let block = format!("  [[{name}]]\n    type = TCPClientInterface\n    enabled = yes\n    target_host = {host}\n    target_port = {port}\n");
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim() == "[interfaces]") else {
        let sep = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
        return Ok(format!("{text}{sep}\n[interfaces]\n{block}"));
    };
    // Before the next top-level section, or at the end.
    let end = lines[start + 1..].iter().position(|l| { let t = l.trim(); t.starts_with('[') && !t.starts_with("[[") }).map_or(lines.len(), |i| start + 1 + i);
    let mut out: Vec<String> = lines[..end].iter().map(|l| l.to_string()).collect();
    out.push(block.trim_end().to_string());
    out.extend(lines[end..].iter().map(|l| l.to_string()));
    Ok(out.join("\n") + "\n")
}

/// The lines of interface `name`'s section: from its `[[name]]` to the next section.
fn section_span(lines: &[&str], name: &str) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| l.trim().starts_with("[[") && l.trim().trim_start_matches('[').trim_end_matches(']').trim() == name)?;
    let end = lines[start + 1..].iter().position(|l| l.trim().starts_with('[')).map_or(lines.len(), |i| start + 1 + i);
    Some((start, end))
}

/// The configuration with interface `name` switched on or off.
pub fn config_set_enabled(text: &str, name: &str, enabled: bool) -> Result<String, String> {
    let lines: Vec<&str> = text.lines().collect();
    let (start, end) = section_span(&lines, name).ok_or_else(|| format!("no connection named {name}"))?;
    let value = if enabled { "yes" } else { "no" };
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    let mut set = false;
    for line in out.iter_mut().take(end).skip(start + 1) {
        let key = line.trim().split('=').next().unwrap_or("").trim().to_string();
        if key == "enabled" || key == "interface_enabled" {
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            *line = format!("{indent}{key} = {value}");
            set = true;
        }
    }
    if !set {
        out.insert(start + 1, format!("    enabled = {value}"));
    }
    Ok(out.join("\n") + "\n")
}

/// The configuration without interface `name`.
pub fn config_remove(text: &str, name: &str) -> Result<String, String> {
    let lines: Vec<&str> = text.lines().collect();
    let (start, end) = section_span(&lines, name).ok_or_else(|| format!("no connection named {name}"))?;
    let out: Vec<&str> = lines[..start].iter().chain(lines[end..].iter()).copied().collect();
    Ok(out.join("\n") + "\n")
}

/// Changes Reticulum's configuration file with `edit` (no `prnsd` to do it live).
fn edit_config(edit: impl FnOnce(&str) -> Result<String, String>) -> Result<(), String> {
    let path = config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let new = edit(&text)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&path, new).map_err(|e| format!("{}: {e}", path.display()))?;
    CHANGED_UNAPPLIED.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
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
    if let Some(bin) = prnsd() {
        let o = prnsd_command(&bin).args(["interfaces", "list"]).output().map_err(|e| e.to_string())?;
        if o.status.success() {
            return Ok(parse_list(&String::from_utf8_lossy(&o.stdout)));
        }
    }
    Ok(parse_config_interfaces(&std::fs::read_to_string(config_path()).unwrap_or_default()))
}

fn run_prnsd(args: &[&str]) -> Result<(), String> {
    let bin = prnsd().ok_or("prnsd not found")?;
    let o = prnsd_command(&bin).args(args).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(first_line(&o.stderr, "prnsd failed")) }
}

/// prnsd asked something: on Windows without a console window of its own
/// (prnsd is a console program; the utility asks it every few seconds).
fn prnsd_command(bin: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = host(bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Changes the Reticulum configuration and puts it to use: a managed prnsd
/// (`prnsd start`) applies it live; the Prns package's job on Haiku runs
/// `prnsd run`, which can't, so it's restarted.
fn reconfigure(args: &[&str]) -> Result<(), String> {
    #[cfg(target_os = "haiku")]
    if service_state(PRNS_JOB).as_deref() == Some("active") {
        run_prnsd(args)?;
        service(PRNS_JOB, "restart")?;
        // retifsd doesn't attach to a new shared instance by itself:
        // restarted after it.
        std::thread::sleep(std::time::Duration::from_secs(2));
        return service(RETIFSD, "restart");
    }
    let mut with = args.to_vec();
    with.push("--apply");
    run_prnsd(&with)
}

/// The Prns package's Reticulum job (Haiku).
#[cfg(target_os = "haiku")]
const PRNS_JOB: &str = "x-vnd.prns-prnsd";
/// On Haiku Reticulum is the Prns package's: prnsd, and its start script
/// (which writes a new user's first configuration).
const HAIKU_PRNSD: &str = "/boot/system/bin/prnsd";
#[cfg(target_os = "haiku")]
const HAIKU_PRNSD_START: &str = "/boot/system/apps/Prns/prnsd-start";

/// Adds a TCP connection to another Reticulum node.
pub fn add_tcp(name: &str, host: &str, port: u16) -> Result<(), String> {
    if prnsd().is_none() {
        return edit_config(|t| config_add_tcp(t, name, host, port));
    }
    let port = port.to_string();
    reconfigure(&["interfaces", "add", "TCPClientInterface", "--name", name, "--target-host", host, "--target-port", &port])
}

pub fn set_interface(name: &str, enabled: bool) -> Result<(), String> {
    if prnsd().is_none() {
        return edit_config(|t| config_set_enabled(t, name, enabled));
    }
    reconfigure(&["interfaces", if enabled { "enable" } else { "disable" }, name])
}

pub fn remove_interface(name: &str) -> Result<(), String> {
    if prnsd().is_none() {
        return edit_config(|t| config_remove(t, name));
    }
    reconfigure(&["interfaces", "remove", name, "--yes"])
}

/// The user services the retifs package installs (Linux, systemd).
#[cfg(not(any(windows, target_os = "haiku")))]
pub const SERVICES: &[(&str, &str)] = &[("retifsd.service", "retifs daemon"), ("retifs-gateway.service", "Libraries as local websites")];
/// On Windows: the programs next to the utility, which it starts itself.
/// Reticulum is whatever shared instance runs, not one of them.
#[cfg(windows)]
pub const SERVICES: &[(&str, &str)] = &[("retifsd", "retifs daemon"), ("retifs-cloudfiles", "Libraries in Explorer")];
/// On Haiku: launch_daemon jobs of the user's (`~/config/settings/launch/retifs`,
/// or a package's `data/user_launch`), started at login; Reticulum is the
/// Prns package's.
#[cfg(target_os = "haiku")]
pub const SERVICES: &[(&str, &str)] = &[("x-vnd.retifs-daemon", "retifs daemon"), ("x-vnd.prns-prnsd", "Reticulum")];

/// The service retifs itself runs as.
pub const RETIFSD: &str = SERVICES[0].0;

/// A user service's state ("active", "inactive", "failed", …), None where
/// there's no systemd.
#[cfg(not(any(windows, target_os = "haiku")))]
pub fn service_state(unit: &str) -> Option<String> {
    if in_flatpak() && !host_has_unit(unit) {
        return Some(NOT_INSTALLED.into());
    }
    let o = host("systemctl").args(["--user", "is-active", unit]).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// `start`, `stop` or `restart` a user service.
#[cfg(not(any(windows, target_os = "haiku")))]
pub fn service(unit: &str, action: &str) -> Result<(), String> {
    if in_flatpak() && !host_has_unit(unit) {
        return Err(NOT_INSTALLED.into());
    }
    let o = host("systemctl").args(["--user", action, unit]).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(first_line(&o.stderr, "systemctl failed")) }
}

/// Running as a Flatpak (FLATPAK_ID set): retifs itself is the host's (the
/// retifs .deb: the FUSE mount can't work in a sandbox), its services and
/// prnsd reached through `flatpak-spawn --host`.
pub fn in_flatpak() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("FLATPAK_ID").is_some()
}

/// What the Flatpak says where the host has no retifs.
pub const NOT_INSTALLED: &str = "retifs isn't installed on this computer: install the retifs .deb";

/// A program of this machine's: in the Flatpak, run on the host.
fn host(program: impl AsRef<std::ffi::OsStr>) -> Command {
    if in_flatpak() {
        let mut cmd = Command::new("flatpak-spawn");
        cmd.arg("--host").arg(program);
        cmd
    } else {
        Command::new(program)
    }
}

/// Whether the host has a user unit (the retifs .deb installs them).
#[cfg(not(any(windows, target_os = "haiku")))]
fn host_has_unit(unit: &str) -> bool {
    host("systemctl").args(["--user", "cat", unit]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Whether retifs is missing on the host (the Flatpak); false elsewhere.
pub fn retifs_missing() -> bool {
    #[cfg(not(any(windows, target_os = "haiku")))]
    if in_flatpak() {
        return !host_has_unit(RETIFSD);
    }
    false
}

/// A launch_daemon job's state: "active" while it runs, "inactive" when it
/// is known but not running. A job the launch_daemon doesn't know yet (a
/// package's jobs are read at login, so not until the next one after
/// installing) is its program, run by the utility itself.
#[cfg(target_os = "haiku")]
pub fn service_state(job: &str) -> Option<String> {
    let o = Command::new("launch_roster").args(["info", job]).output().ok()?;
    // The job's, or the program run here.
    let running = (roster_ok(&o) && job_running(&String::from_utf8_lossy(&o.stdout))) || !haiku_teams(job).is_empty();
    Some(if running { "active" } else { "inactive" }.to_string())
}

/// A job's program: its name, what starts it, and the program that then
/// runs (as `ps` shows it).
#[cfg(target_os = "haiku")]
fn job_program(job: &str) -> Option<(&'static str, PathBuf, PathBuf)> {
    match job {
        "x-vnd.retifs-daemon" => Some(("retifsd", install_dir().join("bin/retifsd"), install_dir().join("bin/retifsd"))),
        PRNS_JOB => Some(("prnsd", PathBuf::from(HAIKU_PRNSD_START), PathBuf::from(HAIKU_PRNSD))),
        _ => None,
    }
}

/// The teams running a job's program.
#[cfg(target_os = "haiku")]
fn haiku_teams(job: &str) -> Vec<String> {
    let Some((_, _, runs)) = job_program(job) else { return Vec::new() };
    let path = runs.to_string_lossy();
    let Ok(o) = Command::new("ps").output() else { return Vec::new() };
    teams_running(&String::from_utf8_lossy(&o.stdout), &path)
}

/// Team IDs of `ps` lines running `path` (Haiku's `ps`: the command line,
/// then team ID, threads, group and user; the ID is the fourth field from
/// the end).
pub fn teams_running(ps: &str, path: &str) -> Vec<String> {
    ps.lines()
        .filter(|l| l.split_whitespace().next() == Some(path))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            f.len().checked_sub(4).map(|i| f[i].to_string())
        })
        .collect()
}

/// Runs a job's program itself (the launch_daemon doesn't know the job yet),
/// its output in `~/config/var/log/retifs/<program>.log`.
#[cfg(target_os = "haiku")]
fn run_program(job: &str, action: &str) -> Result<(), String> {
    let (prog, starts, _) = job_program(job).ok_or_else(|| format!("no job {job}"))?;
    if action == "stop" || action == "restart" {
        for team in haiku_teams(job) {
            let _ = Command::new("kill").arg(&team).output();
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    if action == "start" || action == "restart" {
        if !haiku_teams(job).is_empty() {
            return Ok(());
        }
        if prog == "prnsd" && shared_instance_up() {
            return Err("another Reticulum runs on this machine already".into());
        }
        // Reticulum first, as the job requires it.
        if prog == "retifsd" && !shared_instance_up() && Path::new(HAIKU_PRNSD_START).is_file() {
            run_program(PRNS_JOB, "start")?;
            for _ in 0..30 {
                if shared_instance_up() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
        let logs = PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("config/var/log/retifs");
        std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
        let log = std::fs::OpenOptions::new().create(true).append(true).open(logs.join(format!("{prog}.log"))).map_err(|e| e.to_string())?;
        Command::new(&starts)
            .stdin(std::process::Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .spawn()
            .map_err(|e| format!("{prog}: {e}"))?;
    }
    Ok(())
}

/// Whether `launch_roster info` says the job runs (`running = bool(true)`).
pub fn job_running(info: &str) -> bool {
    info.lines().any(|l| {
        let l = l.trim();
        l.starts_with("running") && l.contains("bool(true)")
    })
}

/// `start`, `stop` or `restart` a launch_daemon job, or, while the
/// launch_daemon doesn't know it yet, its program.
#[cfg(target_os = "haiku")]
pub fn service(job: &str, action: &str) -> Result<(), String> {
    if action == "restart" {
        service(job, "stop")?;
        std::thread::sleep(std::time::Duration::from_secs(1));
        return service(job, "start");
    }
    let known = Command::new("launch_roster").args(["info", job]).output().is_ok_and(|o| roster_ok(&o));
    if !known {
        return run_program(job, action);
    }
    let o = Command::new("launch_roster").args([action, job]).output().map_err(|e| e.to_string())?;
    if !roster_ok(&o) {
        return Err(first_line(&o.stderr, "launch_roster failed"));
    }
    if action == "stop" {
        // And one run here, if any.
        return run_program(job, "stop");
    }
    // A job known from an older install (one that requires a job no longer
    // there) "starts" and doesn't run: then the program, run here.
    for _ in 0..10 {
        if service_state(job).as_deref() == Some("active") {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    run_program(job, "start")
}

/// Whether launch_roster did it: it exits with 0 also when it didn't (an
/// unknown job: "Name not found"), saying so only on stderr.
#[cfg(target_os = "haiku")]
fn roster_ok(o: &std::process::Output) -> bool {
    o.status.success() && o.stderr.iter().all(u8::is_ascii_whitespace)
}

static INSTALL_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Says where the utility is installed, when the program running isn't it
/// (Haiku: the Deskbar replicant runs inside the Deskbar). Only the first
/// call counts.
pub fn set_install_dir(dir: PathBuf) {
    let _ = INSTALL_DIR.set(dir);
}

/// Where the utility is installed, with the programs it starts (Windows, Haiku).
pub fn install_dir() -> PathBuf {
    if let Some(dir) = INSTALL_DIR.get() {
        return dir.clone();
    }
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

/// A program's state: "active" while it runs.
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
        // retifsd is asked to stop, so it closes its store (killed, its index
        // is checked at the next start: minutes on a large store); killed
        // only if it hasn't stopped within 35 s.
        if name == "retifsd" && running(&exe) && crate::Daemon::new().stop().is_ok() {
            for _ in 0..70 {
                if !running(&exe) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
        if running(&exe) {
            let _ = Command::new("taskkill").args(["/IM", &exe, "/F"]).creation_flags(CREATE_NO_WINDOW).output();
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
    if action == "start" || action == "restart" {
        let logs = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("retifs").join("logs");
        std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
        let log = std::fs::OpenOptions::new().create(true).append(true).open(logs.join(format!("{name}.log"))).map_err(|e| e.to_string())?;
        // The Cloud Files provider: every library, a folder each of
        // %USERPROFILE%\retifs.
        let args: &[&str] = if name == "retifs-cloudfiles" { &["--all"] } else { &[] };
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

/// At login (Windows): retifsd once Reticulum is up, and the libraries in
/// Explorer (which wait for retifsd themselves). Says what it did.
#[cfg(windows)]
pub fn supervise() -> Vec<String> {
    let mut did = Vec::new();
    // retifsd can't join a shared instance that isn't there yet: Prns may
    // start after the utility at login.
    for _ in 0..60 {
        if shared_instance_up() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    if !shared_instance_up() {
        did.push("no Reticulum yet: start Prns (or another shared instance)".to_string());
    } else if !running("retifsd.exe") {
        match service("retifsd", "start") {
            Ok(()) => did.push("started retifsd".to_string()),
            Err(e) => did.push(format!("retifsd: {e}")),
        }
    }
    if install_dir().join("retifs-cloudfiles.exe").is_file() && !running("retifs-cloudfiles.exe") {
        match service("retifs-cloudfiles", "start") {
            Ok(()) => did.push("libraries in Explorer".to_string()),
            Err(e) => did.push(format!("retifs-cloudfiles: {e}")),
        }
    }
    did
}

/// Keeps retifs running while the utility does (Windows), as the Android
/// app does: retifsd started again if it stopped, and restarted when
/// Reticulum is back after being away (retifsd doesn't attach to a new
/// shared instance by itself). `away`: how many checks running Reticulum
/// wasn't up (one missed check is a busy machine). Says what it did.
#[cfg(windows)]
pub fn keep(away: &mut u32) -> Vec<String> {
    let mut did = Vec::new();
    let up = shared_instance_up();
    if up && *away >= 2 && running("retifsd.exe") {
        match service("retifsd", "restart") {
            Ok(()) => did.push("Reticulum is back: restarted retifsd".to_string()),
            Err(e) => did.push(format!("retifsd: {e}")),
        }
    } else if up && !running("retifsd.exe") {
        match service("retifsd", "start") {
            Ok(()) => did.push("retifsd had stopped: started it again".to_string()),
            Err(e) => did.push(format!("retifsd: {e}")),
        }
    }
    *away = if up { 0 } else { *away + 1 };
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

/// The settings that are sizes: retifsd reads them as `250G`, `500M`
/// (binary units); the utility shows and takes `250 GB`, `500 MB`.
pub const SIZE_SETTINGS: [&str; 3] = ["max_store", "min_free", "hold_for_others"];

pub fn is_size(key: &str) -> bool {
    SIZE_SETTINGS.contains(&key)
}

const KIB: u64 = 1 << 10;
const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
const TIB: u64 = 1 << 40;

/// A size as typed (`250 GB`, `250gb`, `250G`, `1,5 TB`, `500 MB`, `0`; a
/// bare number is in GB) as retifsd reads it (`250G`, `1.5T`, `500M`, `0`).
/// None if it isn't one.
pub fn size_typed(typed: &str) -> Option<String> {
    let t = typed.trim().replace(',', ".").to_ascii_uppercase();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = (t[..split].trim(), t[split..].trim());
    let n: f64 = num.parse().ok().filter(|n: &f64| n.is_finite() && *n >= 0.0)?;
    let unit = match unit.trim_end_matches("IB").trim_end_matches('B') {
        "" if unit.is_empty() => "G",
        "" => "",
        "K" => "K",
        "M" => "M",
        "G" => "G",
        "T" => "T",
        _ => return None,
    };
    let num = if n.fract() == 0.0 { format!("{}", n as u64) } else { format!("{n}") };
    Some(if n == 0.0 { "0".into() } else { format!("{num}{unit}") })
}

/// A size as retifsd reads it (`250G`) in bytes, as retifsd counts them.
pub fn size_bytes(v: &str) -> Option<u64> {
    let v = v.trim().trim_matches('"').to_ascii_uppercase();
    let (num, mult) = match v.chars().last() {
        Some('K') => (&v[..v.len() - 1], KIB),
        Some('M') => (&v[..v.len() - 1], MIB),
        Some('G') => (&v[..v.len() - 1], GIB),
        Some('T') => (&v[..v.len() - 1], TIB),
        _ => (&v[..], 1),
    };
    num.parse::<f64>().ok().map(|n| (n * mult as f64) as u64)
}

/// A size from retifsd's form (`250G`) as the user reads it (`250 GB`).
pub fn size_shown(v: &str) -> String {
    let v = v.trim().trim_matches('"');
    match v.chars().last().map(|c| c.to_ascii_uppercase()) {
        Some(u @ ('K' | 'M' | 'G' | 'T')) => format!("{} {u}B", &v[..v.len() - 1]),
        _ if v == "0" => "0".into(),
        _ => match v.parse::<u64>() {
            Ok(n) => bytes_shown(n),
            Err(_) => v.to_string(),
        },
    }
}

/// Bytes as a round size to show, as the utility shows usage: TB with one
/// decimal from 1 TB, whole GB from 10 GB, GB with one decimal from 1 GB, MB
/// below.
pub fn bytes_shown(n: u64) -> String {
    let n = n as f64;
    if n >= TIB as f64 {
        format!("{:.1} TB", n / TIB as f64)
    } else if n >= 10.0 * GIB as f64 {
        format!("{:.0} GB", n / GIB as f64)
    } else if n >= GIB as f64 {
        format!("{:.1} GB", n / GIB as f64)
    } else {
        format!("{:.0} MB", n / MIB as f64)
    }
}

/// The machine's name as retifsd takes it for the device's (lower case).
fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        // Haiku has neither: its `hostname` says.
        .or_else(|| {
            let cmd = ["/boot/system/bin/hostname", "hostname"].into_iter().find(|c| !c.starts_with('/') || Path::new(c).exists())?;
            Command::new(cmd).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        })
        .map(|h| h.trim().to_lowercase())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "this-device".into())
}

/// The size of the disk the store is on (of its nearest existing folder).
fn disk_total(store: &Path) -> Option<u64> {
    let mut p = Some(store);
    while let Some(dir) = p {
        if dir.exists() {
            return fs4::total_space(dir).ok();
        }
        p = dir.parent();
    }
    None
}

/// What retifsd takes for a setting left at its default, here (as shown):
/// the store limit is half the disk, at least 10 % of it (5 GB at least) is
/// left free, others' copies are held up to the store limit, 3 copies, the
/// machine's name. None where there's no value (a site).
pub fn setting_default(store: &Path, key: &str) -> Option<String> {
    let total = disk_total(store);
    match key {
        "max_store" | "hold_for_others" => total.map(|t| bytes_shown(t / 2)),
        "min_free" => total.map(|t| bytes_shown((t / 10).max(5 * GIB))),
        "min_copies" => Some("3".into()),
        "device" => Some(hostname()),
        "no_storage" => Some("false".into()),
        _ => None,
    }
}

/// A setting as a front end shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingView {
    pub section: &'static str,
    pub key: &'static str,
    pub what: &'static str,
    /// What the field holds: the file's value, else the default this machine
    /// gets (so no field is empty where there is one); sizes with their unit.
    pub value: String,
    pub default: Option<String>,
    /// Whether the file sets it (else it's the default).
    pub set: bool,
    /// The unit to show by the field ("GB, MB or TB", "copies"), or "".
    pub unit: &'static str,
}

/// The settings of the store's `retifs.toml`, as the front ends show them.
pub fn settings_view(store: &Path) -> Vec<SettingView> {
    let text = std::fs::read_to_string(settings_file(store)).unwrap_or_default();
    SETTINGS
        .iter()
        .map(|&(section, key, what)| {
            let set = get_setting(&text, section, key).map(|v| if is_size(key) { size_shown(&v) } else { shown_value(&v) });
            let default = setting_default(store, key);
            SettingView {
                section,
                key,
                what,
                value: set.clone().or_else(|| default.clone()).unwrap_or_default(),
                default,
                set: set.is_some(),
                unit: match key {
                    k if is_size(k) => "GB, MB or TB",
                    "min_copies" => "copies",
                    _ => "",
                },
            }
        })
        .collect()
}

/// Saves what the fields hold, `(section, key, typed)` (a switch: "true" or
/// ""): only what changed is written, and a field left at its default stays
/// the default. Sizes are taken with units (`250 GB`) and written as retifsd
/// reads them (`250G`). The caller restarts the daemon.
pub fn save_typed(store: &Path, typed: &[(&str, &str, String)]) -> Result<(), String> {
    let path = settings_file(store);
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    for &(section, key, what) in SETTINGS.iter() {
        let Some((_, _, t)) = typed.iter().find(|(s, k, _)| *s == section && *k == key) else { continue };
        let default = setting_default(store, key);
        let now = typed_setting(key, what, t, default.as_deref())?;
        if get_setting(&text, section, key) != now {
            text = set_setting(&text, section, key, now.as_deref());
        }
    }
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// A typed setting as the file holds it, None when it is the default (the
/// file then leaves it commented out, and retifsd keeps deriving it); an
/// error for a size that isn't one.
pub fn typed_setting(key: &str, what: &str, typed: &str, default: Option<&str>) -> Result<Option<String>, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Ok(None);
    }
    if is_size(key) {
        let v = size_typed(typed).ok_or_else(|| format!("{what}: \"{typed}\" isn't a size (like 250 GB, 500 MB or 1.5 TB)"))?;
        let same = default.and_then(size_typed).is_some_and(|d| d == v);
        return Ok((!same).then(|| toml_value(&v)));
    }
    if default.is_some_and(|d| d == typed) {
        return Ok(None);
    }
    Ok(Some(toml_value(typed)))
}

#[cfg(test)]
mod tests {

    /// Reticulum's own configuration, as Python's rnsd and prnsd both read it.
    #[test]
    fn reticulum_config_read_and_changed() {
        let text = "[reticulum]
  share_instance = Yes

[interfaces]
  [[Default Interface]]
    type = AutoInterface
    interface_enabled = True

  [[RTR-3]]
    type = TCPClientInterface
    enabled = no
    target_host = 10.0.0.1
    target_port = 4242

[logging]
  loglevel = 4
";
        let list = parse_config_interfaces(text);
        assert_eq!(list, [
            Configured { name: "Default Interface".into(), kind: "AutoInterface".into(), enabled: true },
            Configured { name: "RTR-3".into(), kind: "TCPClientInterface".into(), enabled: false },
        ]);
        let added = config_add_tcp(text, "hub", "hub.example", 4242).unwrap();
        let after = parse_config_interfaces(&added);
        assert_eq!(after.len(), 3);
        assert_eq!(after[2], Configured { name: "hub".into(), kind: "TCPClientInterface".into(), enabled: true });
        assert!(added.find("[[hub]]").unwrap() < added.find("[logging]").unwrap(), "added inside [interfaces]");
        assert!(config_add_tcp(&added, "hub", "x", 1).is_err());
        let on = config_set_enabled(&added, "RTR-3", true).unwrap();
        assert!(parse_config_interfaces(&on).iter().all(|c| c.enabled));
        let gone = config_remove(&on, "RTR-3").unwrap();
        assert_eq!(parse_config_interfaces(&gone).iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Default Interface", "hub"]);
        assert!(gone.contains("[logging]") && gone.contains("loglevel = 4"));
        // No [interfaces] yet: one is made.
        let fresh = config_add_tcp("[reticulum]
  share_instance = Yes
", "hub", "h", 1).unwrap();
        assert_eq!(parse_config_interfaces(&fresh).len(), 1);
    }

    use super::*;

    const FILE: &str = "# retifs node settings.\n\n[store]\n# max_store = \"...\"          # store limit\n# no_storage = false         # keep nothing\n\n[node]\n# device = \"...\"             # this device's name\n";

    #[test]
    fn sizes_are_shown_and_typed_with_units() {
        assert_eq!(size_typed("250 GB").as_deref(), Some("250G"));
        assert_eq!(size_typed("250gb").as_deref(), Some("250G"));
        assert_eq!(size_typed("250G").as_deref(), Some("250G"));
        assert_eq!(size_typed("1,5 TB").as_deref(), Some("1.5T"));
        assert_eq!(size_typed("500 MiB").as_deref(), Some("500M"));
        assert_eq!(size_typed("250").as_deref(), Some("250G"), "a bare number is GB");
        assert_eq!(size_typed("0 GB").as_deref(), Some("0"));
        assert_eq!(size_typed("lots"), None);
        assert_eq!(size_typed("5 PB"), None);
        assert_eq!(size_shown("\"250G\""), "250 GB");
        assert_eq!(size_shown("1.5T"), "1.5 TB");
        assert_eq!(size_shown("0"), "0");
        assert_eq!(size_bytes("\"2G\""), Some(2 << 30));
        assert_eq!(bytes_shown(500 << 30), "500 GB");
        assert_eq!(bytes_shown((20 << 40) + (3 << 39)), "21.5 TB");
        assert_eq!(bytes_shown(300 << 20), "300 MB");
        assert_eq!(bytes_shown((20 << 30) - (40 << 20)), "20 GB", "as the usage line shows it");
        assert_eq!(bytes_shown(5 << 30), "5.0 GB");
        // A default shown and saved as it is stays the default.
        assert_eq!(size_typed("5.0 GB"), size_typed("5 GB"));
    }

    #[test]
    fn a_default_typed_stays_a_default() {
        let size = |t| typed_setting("max_store", "Store limit", t, Some("237 GB"));
        assert_eq!(size("237 GB"), Ok(None), "the default shown, saved: still the default");
        assert_eq!(size("237G"), Ok(None));
        assert_eq!(size("100 GB"), Ok(Some("\"100G\"".into())));
        assert_eq!(size(""), Ok(None));
        assert!(size("much").unwrap_err().contains("isn't a size"));
        assert_eq!(typed_setting("min_copies", "", "3", Some("3")), Ok(None));
        assert_eq!(typed_setting("min_copies", "", "2", Some("3")), Ok(Some("2".into())));
        assert_eq!(typed_setting("site", "", "kodu", None), Ok(Some("\"kodu\"".into())));
    }

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
    fn haiku_ps_teams() {
        let ps = "Team                                                  Id #Threads  Gid  Uid \n\
                  kernel_team                                            1       51    0    0 \n\
                  /boot/system/apps/retifs/bin/prnsd run                76       23    0    0 \n\
                  /boot/system/apps/retifs/bin/retifsd                 105       17    0    0 \n\
                  /boot/system/apps/retifs/bin/retifsd.old             107       17    0    0 \n";
        assert_eq!(teams_running(ps, "/boot/system/apps/retifs/bin/retifsd"), ["105"]);
        assert_eq!(teams_running(ps, "/boot/system/apps/retifs/bin/prnsd"), ["76"]);
        assert!(teams_running(ps, "/boot/home/retifsd").is_empty());
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
