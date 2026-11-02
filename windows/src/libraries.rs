//! Libraries and Announces, as in the Haiku and GTK utilities: each library
//! with who it's shared with and what's here of it, a right-click menu
//! (favourite, keep on this device, copy link, members, invite, announce,
//! leave), the members window (roles, removing, invitations not used yet)
//! and the invitation window; Announces: the public libraries heard, with
//! Join. Every daemon call runs on a thread and posts back.

use std::cell::RefCell;
use std::sync::Mutex;

use retifs_desktop_core::{Daemon, HeardLibrary, InviteInfo};
use serde_json::Value;
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{child, note, poke, scale, set_text, spawn, text_of, wide, MAIN};

pub const ID_LIBS: i32 = 130;
pub const ID_HEARD: i32 = 131;
const ID_NEW: i32 = 132;
const ID_MEMBERS: i32 = 133;
const ID_INVITE: i32 = 134;
const ID_COPY: i32 = 135;
const ID_JOIN: i32 = 136;
// The right-click menu.
const IDM_FAVORITE: u32 = 40;
const IDM_KEEP: u32 = 41;
const IDM_COPY: u32 = 42;
const IDM_MEMBERS: u32 = 43;
const IDM_INVITE: u32 = 44;
const IDM_ANNOUNCE: u32 = 45;
const IDM_LEAVE: u32 = 46;
// The members window.
const ID_M_LIST: i32 = 400;
const ID_M_INVITES: i32 = 401;
const ID_M_INVITE: i32 = 402;
const ID_M_REMOVE: i32 = 403;
const ID_M_WITHDRAW: i32 = 404;
const ID_M_CLOSE: i32 = 405;
// The invitation window.
const ID_I_READ: i32 = 500;
const ID_I_WRITE: i32 = 501;
const ID_I_GRANT: i32 = 502;
const ID_I_DAYS: i32 = 503;
const ID_I_MAKE: i32 = 504;
const ID_I_LINK: i32 = 505;
const ID_I_COPY: i32 = 506;
// The new library window.
const ID_N_NAME: i32 = 600;
const ID_N_PUBLIC: i32 = 601;
const ID_N_OK: i32 = 602;
const ID_N_CANCEL: i32 = 603;

const WM_MEMBERS: u32 = WM_APP + 10;
const WM_INVITED: u32 = WM_APP + 11;

const MEMBERS_CLASS: PCWSTR = w!("retifs-desktop-members");
const INVITE_CLASS: PCWSTR = w!("retifs-desktop-invite");
const NEW_CLASS: PCWSTR = w!("retifs-desktop-new-library");

/// The pages' controls.
#[derive(Clone)]
pub struct Pages {
    pub libs: HWND,
    pub libs_buttons: Vec<HWND>,
    pub heard: HWND,
    pub join: HWND,
    pub heard_note: HWND,
}

/// One library as listed (Daemon::libraries, favourites first).
#[derive(Clone, Default)]
struct Lib {
    name: String,
    json: Value,
}

thread_local! {
    static LIBS: RefCell<Vec<Lib>> = const { RefCell::new(Vec::new()) };
    static HEARD: RefCell<Vec<HeardLibrary>> = const { RefCell::new(Vec::new()) };
    /// The members window: its library, its members' writer keys and its
    /// invitations' IDs, row by row.
    static MEMBERS: RefCell<Option<(HWND, String, Vec<String>, Vec<String>)>> = const { RefCell::new(None) };
    static FONT: RefCell<windows::Win32::Graphics::Gdi::HFONT> = RefCell::new(Default::default());
}

/// What the members window's thread found, for it to show.
static MEMBERS_FOUND: Mutex<Option<(Result<Value, String>, Result<Vec<InviteInfo>, String>)>> = Mutex::new(None);
static INVITE_LINK: Mutex<String> = Mutex::new(String::new());

pub unsafe fn register(class: &WNDCLASSW) {
    for (name, proc_) in [
        (MEMBERS_CLASS, members_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT),
        (INVITE_CLASS, invite_proc),
        (NEW_CLASS, new_proc),
    ] {
        let c = WNDCLASSW { lpfnWndProc: Some(proc_), lpszClassName: name, ..*class };
        RegisterClassW(&c);
    }
}

unsafe fn columns(list: HWND, cols: &[(&str, i32)]) {
    SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, WPARAM(0), LPARAM((LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize));
    for (i, (title, width)) in cols.iter().enumerate() {
        let text = wide(title);
        let col = LVCOLUMNW { mask: LVCF_TEXT | LVCF_WIDTH, cx: *width, pszText: PWSTR(text.as_ptr() as *mut u16), ..Default::default() };
        SendMessageW(list, LVM_INSERTCOLUMNW, WPARAM(i), LPARAM(&col as *const _ as isize));
    }
}

unsafe fn set_row(list: HWND, i: usize, cells: &[String]) {
    for (col, text) in cells.iter().enumerate() {
        let t = wide(text);
        let item = LVITEMW { mask: LVIF_TEXT, iItem: i as i32, iSubItem: col as i32, pszText: PWSTR(t.as_ptr() as *mut u16), ..Default::default() };
        let msg = if col == 0 { LVM_INSERTITEMW } else { LVM_SETITEMTEXTW };
        SendMessageW(list, msg, WPARAM(i), LPARAM(&item as *const _ as isize));
    }
}

unsafe fn selected(list: HWND) -> Option<usize> {
    let i = SendMessageW(list, LVM_GETNEXTITEM, WPARAM(usize::MAX), LPARAM(LVNI_SELECTED as isize)).0;
    (i >= 0).then_some(i as usize)
}

unsafe fn report(parent: HWND, id: i32, font: windows::Win32::Graphics::Gdi::HFONT) -> HWND {
    child(parent, WC_LISTVIEWW, "", WS_BORDER | WS_TABSTOP | WINDOW_STYLE((LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS) as u32), id, font)
}

/// The Libraries and Announces pages' controls.
pub unsafe fn build(hwnd: HWND, font: windows::Win32::Graphics::Gdi::HFONT) -> Pages {
    FONT.with(|f| *f.borrow_mut() = font);
    let libs = report(hwnd, ID_LIBS, font);
    columns(libs, &[("Library", 170), ("Shared", 80), ("You may", 160), ("Records", 80), ("Peers", 60), ("On this device", 170)]);
    let button = |text: &str, id: i32| child(hwnd, WC_BUTTONW, text, WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), id, font);
    let libs_buttons = vec![button("New library…", ID_NEW), button("Members…", ID_MEMBERS), button("Invite…", ID_INVITE), button("Copy link", ID_COPY)];
    let heard_note = child(hwnd, WC_STATICW, "Public libraries others announce. Joining fetches only what you look at.", WINDOW_STYLE(0), 0, font);
    let heard = report(hwnd, ID_HEARD, font);
    columns(heard, &[("Library", 160), ("About", 260), ("Holds", 150), ("Hops", 50), ("Here", 120)]);
    let join = button("Join", ID_JOIN);
    Pages { libs, libs_buttons, heard, join, heard_note }
}

pub unsafe fn layout(p: &Pages, hwnd: HWND, x: i32, y: i32, pw: i32, ph: i32) {
    let s = |v| scale(hwnd, v);
    let (bw, bh) = (s(130), s(28));
    let _ = MoveWindow(p.libs, x, y, pw, ph - bh - s(10), true);
    for (i, b) in p.libs_buttons.iter().enumerate() {
        let _ = MoveWindow(*b, x + i as i32 * (bw + s(8)), y + ph - bh, bw, bh, true);
    }
    let _ = MoveWindow(p.heard_note, x, y, pw, s(22), true);
    let _ = MoveWindow(p.heard, x, y + s(28), pw, ph - s(28) - bh - s(10), true);
    let _ = MoveWindow(p.join, x, y + ph - bh, bw, bh, true);
}

/// What the worker asks for these pages (on its thread).
pub fn fetch(daemon: &Daemon) -> (Vec<Value>, Vec<HeardLibrary>) {
    let libs = match daemon.libraries() {
        Ok(Value::Array(l)) => l,
        _ => Vec::new(),
    };
    (libs, daemon.heard().unwrap_or_default())
}

fn number(n: u64) -> String {
    let d = n.to_string();
    let mut out = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Shows the libraries and the libraries heard (on the UI thread).
pub unsafe fn show(p: &Pages, libs: &[Value], heard: &[HeardLibrary]) {
    let keep = selected(p.libs).and_then(|i| LIBS.with(|l| l.borrow().get(i).map(|l| l.name.clone())));
    SendMessageW(p.libs, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
    let list: Vec<Lib> = libs.iter().map(|l| Lib { name: l["name"].as_str().unwrap_or("").to_string(), json: l.clone() }).collect();
    for (i, l) in list.iter().enumerate() {
        let j = &l.json;
        let name = if j["favorite"].as_bool().unwrap_or(false) { format!("★ {}", l.name) } else { l.name.clone() };
        let (kept, here) = (j["kept"].as_u64().unwrap_or(0), j["kept_here"].as_u64().unwrap_or(0));
        let coming = j["coming"].as_bool().unwrap_or(false);
        let on_device = if coming && kept == 0 {
            "fetching its records…".to_string()
        } else if coming {
            format!("fetching {} of {} files…", number(here), number(kept))
        } else if j["lazy"].as_bool().unwrap_or(false) {
            "fetched as needed".to_string()
        } else if j["keep"] == "all" {
            "kept here".to_string()
        } else {
            "content as needed".to_string()
        };
        set_row(
            p.libs,
            i,
            &[
                name,
                if j["public"].as_bool().unwrap_or(false) { "public" } else { "private" }.to_string(),
                format!("{}{}", j["access"].as_str().unwrap_or(""), if j["own"].as_bool().unwrap_or(false) { " · yours" } else { "" }),
                number(j["records"].as_u64().unwrap_or(0)),
                j["peers"].as_u64().unwrap_or(0).to_string(),
                on_device,
            ],
        );
    }
    if let Some(i) = keep.and_then(|k| list.iter().position(|l| l.name == k)) {
        let item = LVITEMW { stateMask: LVIS_SELECTED, state: LVIS_SELECTED, ..Default::default() };
        SendMessageW(p.libs, LVM_SETITEMSTATE, WPARAM(i), LPARAM(&item as *const _ as isize));
    }
    LIBS.with(|l| *l.borrow_mut() = list);

    SendMessageW(p.heard, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
    for (i, h) in heard.iter().enumerate() {
        let holds: Vec<String> = h.has.iter().take(2).map(|(t, n)| format!("{} {}", number(*n), t.rsplit(['/', '.']).next().unwrap_or(t))).collect();
        let here = match &h.here {
            Some(n) => format!("joined as {n}"),
            None if !h.readable => "needs a newer retifs".into(),
            None => String::new(),
        };
        set_row(p.heard, i, &[h.name.clone(), h.about.clone(), holds.join(", "), h.hops.to_string(), here]);
    }
    HEARD.with(|l| *l.borrow_mut() = heard.to_vec());
}

fn lib(name: &str) -> Option<Lib> {
    LIBS.with(|l| l.borrow().iter().find(|l| l.name == name).cloned())
}

fn flag(name: &str, key: &str) -> bool {
    lib(name).is_some_and(|l| l.json[key].as_bool().unwrap_or(false))
}

fn kept_all(name: &str) -> bool {
    lib(name).is_some_and(|l| !l.json["lazy"].as_bool().unwrap_or(false) && l.json["keep"] == "all")
}

fn may_invite(name: &str) -> bool {
    lib(name).is_some_and(|l| l.json["access"].as_str().unwrap_or("").contains("invite"))
}

unsafe fn selected_lib(p: &Pages) -> Option<String> {
    selected(p.libs).and_then(|i| LIBS.with(|l| l.borrow().get(i).map(|l| l.name.clone())))
}

/// Copies `text` to the clipboard.
pub unsafe fn copy(owner: HWND, text: &str) {
    let utf16: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    if OpenClipboard(owner).is_err() {
        return;
    }
    let _ = EmptyClipboard();
    if let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, utf16.len() * 2) {
        let ptr = GlobalLock(mem) as *mut u16;
        if !ptr.is_null() {
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr, utf16.len());
            let _ = GlobalUnlock(mem);
            let _ = SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(mem.0));
        }
    }
    let _ = CloseClipboard();
}

/// The pages' buttons and the list's right-click. Returns whether it was theirs.
pub unsafe fn command(p: &Pages, hwnd: HWND, id: i32) -> bool {
    let picked = selected_lib(p);
    match id {
        ID_NEW => ask_new(hwnd),
        ID_MEMBERS | ID_INVITE | ID_COPY => {
            let Some(ns) = picked else {
                note("Pick a library first");
                return true;
            };
            act(hwnd, &ns, match id {
                ID_MEMBERS => IDM_MEMBERS,
                ID_INVITE => IDM_INVITE,
                _ => IDM_COPY,
            });
        }
        ID_JOIN => {
            let Some(h) = selected(p.heard).and_then(|i| HEARD.with(|l| l.borrow().get(i).cloned())) else {
                note("Pick a library heard first");
                return true;
            };
            if h.here.is_some() {
                note(format!("{} is here already", h.name));
                return true;
            }
            spawn("Joining", move || Daemon::new().join_heard(&h).map(|_| ()));
        }
        _ => return false,
    }
    true
}

/// The right-click menu of the library under the pointer.
pub unsafe fn context(p: &Pages, hwnd: HWND) {
    let Some(ns) = selected_lib(p) else { return };
    let Ok(menu) = CreatePopupMenu() else { return };
    let me = ns == "me";
    let own = flag(&ns, "own");
    let add = |id: u32, text: &str, enabled: bool| {
        let _ = AppendMenuW(menu, MF_STRING | if enabled { MF_ENABLED } else { MF_GRAYED }, id as usize, &wide(text));
    };
    add(IDM_FAVORITE, if flag(&ns, "favorite") { "Remove from favorites" } else { "Add to favorites" }, !me);
    add(IDM_KEEP, if kept_all(&ns) { "Fetch as needed only" } else { "Keep on this device" }, true);
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
    add(IDM_MEMBERS, "Members…", !me);
    add(IDM_INVITE, "Invite…", !me && may_invite(&ns));
    add(IDM_COPY, if flag(&ns, "public") { "Copy link" } else { "Copy link (whoever has it can read)" }, !me);
    if own && !me {
        add(IDM_ANNOUNCE, if flag(&ns, "announced") { "Stop announcing" } else { "Announce on the network…" }, true);
    }
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
    add(IDM_LEAVE, "Leave library…", !me && !own);
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, None);
    let _ = DestroyMenu(menu);
    if cmd.0 != 0 {
        act(hwnd, &ns, cmd.0 as u32);
    }
}

unsafe fn act(hwnd: HWND, ns: &str, what: u32) {
    let ns = ns.to_string();
    match what {
        IDM_FAVORITE => {
            let on = !flag(&ns, "favorite");
            spawn(if on { "Adding to favorites" } else { "Removing from favorites" }, move || Daemon::new().favorite(&ns, on));
        }
        IDM_KEEP => {
            let all = !kept_all(&ns);
            spawn(if all { "Keeping it on this device" } else { "Keeping only what's used" }, move || Daemon::new().keep(&ns, all));
        }
        IDM_COPY => {
            let owner = hwnd.0 as isize;
            std::thread::spawn(move || match Daemon::new().link(&ns) {
                Ok(link) => {
                    *INVITE_LINK.lock().unwrap() = link;
                    let _ = PostMessageW(HWND(owner as _), WM_APP + 12, WPARAM(0), LPARAM(0));
                }
                Err(e) => note(format!("Couldn't make its link: {e}")),
            });
        }
        IDM_MEMBERS => open_members(hwnd, &ns),
        IDM_INVITE => open_invite(hwnd, &ns),
        IDM_ANNOUNCE => {
            let on = !flag(&ns, "announced");
            if on {
                let q = wide(&format!("Announce {ns} on the network?\n\nAnyone who hears it can then join it and read everything in it. Only you (and those you invite) can write."));
                if MessageBoxW(hwnd, &q, w!("retifs"), MB_YESNO | MB_ICONQUESTION) != IDYES {
                    return;
                }
            }
            spawn(if on { "Announcing" } else { "Stopping announcing" }, move || Daemon::new().announce(&ns, on));
        }
        IDM_LEAVE => {
            let q = wide(&format!("Leave {ns}?\n\nIts records go from this device. Others who have it keep theirs; you can join it again."));
            if MessageBoxW(hwnd, &q, w!("retifs"), MB_YESNO | MB_ICONWARNING) == IDYES {
                spawn("Leaving", move || Daemon::new().leave(&ns));
            }
        }
        _ => {}
    }
}

/// The main window got a library's link (Copy link): to the clipboard.
pub unsafe fn link_ready(hwnd: HWND) {
    let link = INVITE_LINK.lock().unwrap().clone();
    copy(hwnd, &link);
    note("Its link is copied: whoever has it can read the library");
}

unsafe fn font() -> windows::Win32::Graphics::Gdi::HFONT {
    FONT.with(|f| *f.borrow())
}

unsafe fn popup(owner: HWND, class: PCWSTR, title: &str, w: i32, h: i32) -> Option<HWND> {
    let s = |v| scale(owner, v);
    CreateWindowExW(
        WS_EX_DLGMODALFRAME,
        class,
        &wide(title),
        WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        s(w),
        s(h),
        owner,
        None,
        GetModuleHandleW(None).unwrap_or_default(),
        None,
    )
    .ok()
}

// The members window.

unsafe fn open_members(owner: HWND, ns: &str) {
    let manage = may_invite(ns);
    let Some(h) = popup(owner, MEMBERS_CLASS, &format!("Members of {ns}"), 620, 480) else { return };
    let s = |v| scale(owner, v);
    let f = font();
    let intro = child(h, WC_STATICW, "Everyone here can read the library. Removing someone: they keep what they've seen, not what's written from then on.", WS_VISIBLE, 0, f);
    let _ = MoveWindow(intro, s(14), s(10), s(580), s(36), true);
    let list = report(h, ID_M_LIST, f);
    columns(list, &[("Member", 200), ("May", 170), ("Details", 200)]);
    let _ = ShowWindow(list, SW_SHOW);
    let _ = MoveWindow(list, s(14), s(50), s(576), s(190), true);
    let head = child(h, WC_STATICW, "Invitations not used yet", WS_VISIBLE, 0, f);
    let _ = MoveWindow(head, s(14), s(250), s(300), s(22), true);
    let invites = report(h, ID_M_INVITES, f);
    columns(invites, &[("Invitation", 160), ("For", 200), ("Left", 120)]);
    let _ = ShowWindow(invites, SW_SHOW);
    let _ = MoveWindow(invites, s(14), s(274), s(576), s(110), true);
    let mut x = s(14);
    for (text, id, enabled) in [("Invite…", ID_M_INVITE, manage), ("Remove", ID_M_REMOVE, manage), ("Withdraw", ID_M_WITHDRAW, manage), ("Close", ID_M_CLOSE, true)] {
        let b = child(h, WC_BUTTONW, text, WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), id, f);
        let _ = EnableWindow(b, enabled);
        let _ = MoveWindow(b, x, s(396), s(110), s(28), true);
        x += s(118);
    }
    MEMBERS.with(|m| *m.borrow_mut() = Some((h, ns.to_string(), Vec::new(), Vec::new())));
    read_members(h, ns);
}

unsafe fn read_members(h: HWND, ns: &str) {
    let (hw, ns) = (h.0 as isize, ns.to_string());
    std::thread::spawn(move || {
        let d = Daemon::new();
        *MEMBERS_FOUND.lock().unwrap() = Some((d.members(&ns), d.invites(&ns)));
        let _ = PostMessageW(HWND(hw as _), WM_MEMBERS, WPARAM(0), LPARAM(0));
    });
}

unsafe fn fill_members(h: HWND) {
    let Some((members, invites)) = MEMBERS_FOUND.lock().unwrap().take() else { return };
    let list = GetDlgItem(h, ID_M_LIST).unwrap_or_default();
    let inv_list = GetDlgItem(h, ID_M_INVITES).unwrap_or_default();
    SendMessageW(list, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
    SendMessageW(inv_list, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
    let mut writers = Vec::new();
    match members {
        Ok(Value::Array(list_)) => {
            for (i, m) in list_.iter().enumerate() {
                let words = m["words"].as_str().unwrap_or("").to_string();
                let name = m["name"].as_str().filter(|n| !n.is_empty()).map(String::from).unwrap_or_else(|| words.clone());
                let mut details: Vec<String> = Vec::new();
                if m["creator"].as_bool().unwrap_or(false) {
                    details.push("made it".into());
                }
                details.extend(m["limits"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(String::from)));
                if name != words {
                    details.push(words);
                }
                set_row(list, i, &[name, m["role"].as_str().unwrap_or("").to_string(), details.join(" · ")]);
                writers.push(if m["you"].as_bool().unwrap_or(false) || m["creator"].as_bool().unwrap_or(false) { String::new() } else { m["writer"].as_str().unwrap_or("").to_string() });
            }
        }
        Ok(_) => {}
        Err(e) => set_row(list, 0, &["Couldn't read them".into(), e, String::new()]),
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let mut ids = Vec::new();
    for (i, inv) in invites.unwrap_or_default().iter().filter(|i| !i.spent).enumerate() {
        let role = if !inv.write { "read" } else if inv.grant { "read, write and invite" } else { "read and write" };
        let left = if inv.expires < now { "expired".to_string() } else { format!("{} h", (inv.expires - now) / 3_600_000) };
        set_row(inv_list, i, &[inv.id[..8.min(inv.id.len())].to_string(), role.into(), left]);
        ids.push(inv.id.clone());
    }
    MEMBERS.with(|m| {
        if let Some(m) = m.borrow_mut().as_mut() {
            m.2 = writers;
            m.3 = ids;
        }
    });
}

unsafe extern "system" fn members_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ns = MEMBERS.with(|m| m.borrow().as_ref().map(|m| m.1.clone())).unwrap_or_default();
    match msg {
        WM_MEMBERS => {
            fill_members(hwnd);
            LRESULT(0)
        }
        WM_COMMAND => {
            match (wparam.0 & 0xffff) as i32 {
                ID_M_INVITE => open_invite(hwnd, &ns),
                ID_M_REMOVE => {
                    let list = GetDlgItem(hwnd, ID_M_LIST).unwrap_or_default();
                    let writer = selected(list).and_then(|i| MEMBERS.with(|m| m.borrow().as_ref().and_then(|m| m.2.get(i).cloned()))).unwrap_or_default();
                    if writer.is_empty() {
                        MessageBoxW(hwnd, w!("Pick a member to remove (not this device, not its maker)."), w!("retifs"), MB_OK);
                        return LRESULT(0);
                    }
                    let q = wide("Remove this member?\n\nWhat they write stops counting, and the library's key changes: they keep what they've read, but can't read what's written from now on.");
                    if MessageBoxW(hwnd, &q, w!("retifs"), MB_YESNO | MB_ICONWARNING) == IDYES {
                        let (hw, ns2) = (hwnd.0 as isize, ns.clone());
                        std::thread::spawn(move || {
                            if let Err(e) = Daemon::new().remove_member(&ns2, &writer) {
                                note(format!("Couldn't remove them: {e}"));
                            }
                            let d = Daemon::new();
                            *MEMBERS_FOUND.lock().unwrap() = Some((d.members(&ns2), d.invites(&ns2)));
                            let _ = PostMessageW(HWND(hw as _), WM_MEMBERS, WPARAM(0), LPARAM(0));
                            poke();
                        });
                    }
                }
                ID_M_WITHDRAW => {
                    let list = GetDlgItem(hwnd, ID_M_INVITES).unwrap_or_default();
                    let Some(id) = selected(list).and_then(|i| MEMBERS.with(|m| m.borrow().as_ref().and_then(|m| m.3.get(i).cloned()))) else {
                        MessageBoxW(hwnd, w!("Pick an invitation to withdraw."), w!("retifs"), MB_OK);
                        return LRESULT(0);
                    };
                    let (hw, ns2) = (hwnd.0 as isize, ns.clone());
                    std::thread::spawn(move || {
                        if let Err(e) = Daemon::new().cancel_invite(&ns2, &id) {
                            note(format!("Couldn't withdraw it: {e}"));
                        }
                        let d = Daemon::new();
                        *MEMBERS_FOUND.lock().unwrap() = Some((d.members(&ns2), d.invites(&ns2)));
                        let _ = PostMessageW(HWND(hw as _), WM_MEMBERS, WPARAM(0), LPARAM(0));
                    });
                }
                ID_M_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            MEMBERS.with(|m| *m.borrow_mut() = None);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// The invitation window.

thread_local! {
    static INVITING: RefCell<String> = const { RefCell::new(String::new()) };
}

unsafe fn open_invite(owner: HWND, ns: &str) {
    let Some(h) = popup(owner, INVITE_CLASS, &format!("Invite someone to {ns}"), 520, 400) else { return };
    INVITING.with(|i| *i.borrow_mut() = ns.to_string());
    let s = |v| scale(owner, v);
    let f = font();
    let l = child(h, WC_STATICW, "They may:", WS_VISIBLE, 0, f);
    let _ = MoveWindow(l, s(14), s(12), s(200), s(22), true);
    for (i, (text, id)) in [("Read", ID_I_READ), ("Read and write", ID_I_WRITE), ("Read, write and invite others", ID_I_GRANT)].iter().enumerate() {
        let style = WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTORADIOBUTTON as u32) | if i == 0 { WS_GROUP } else { WINDOW_STYLE(0) };
        let r = child(h, WC_BUTTONW, text, style, *id, f);
        let _ = MoveWindow(r, s(24), s(38 + i as i32 * 26), s(360), s(24), true);
        if i == 0 {
            SendMessageW(r, BM_SETCHECK, WPARAM(1), LPARAM(0));
        }
    }
    let days = child(h, WC_COMBOBOXW, "", WS_VISIBLE | WS_TABSTOP | WS_GROUP | WINDOW_STYLE(CBS_DROPDOWNLIST as u32), ID_I_DAYS, f);
    for t in ["Usable for 1 day", "Usable for 1 week", "Usable for 30 days"] {
        SendMessageW(days, CB_ADDSTRING, WPARAM(0), LPARAM(wide(t).as_ptr() as isize));
    }
    SendMessageW(days, CB_SETCURSEL, WPARAM(1), LPARAM(0));
    let _ = MoveWindow(days, s(24), s(122), s(220), s(200), true);
    let make = child(h, WC_BUTTONW, "Make invitation", WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32), ID_I_MAKE, f);
    let _ = MoveWindow(make, s(330), s(120), s(160), s(28), true);
    let link = child(h, WC_EDITW, "", WS_VISIBLE | WS_BORDER | WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32), ID_I_LINK, f);
    let _ = MoveWindow(link, s(14), s(160), s(476), s(110), true);
    let note_ = child(h, WC_STATICW, "Pass it in person or end to end: whoever sees it can read; the first to open it becomes the member.", WS_VISIBLE, 0, f);
    let _ = MoveWindow(note_, s(14), s(278), s(476), s(36), true);
    let copy_ = child(h, WC_BUTTONW, "Copy", WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), ID_I_COPY, f);
    let _ = EnableWindow(copy_, false);
    let _ = MoveWindow(copy_, s(380), s(320), s(110), s(28), true);
}

unsafe extern "system" fn invite_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            match (wparam.0 & 0xffff) as i32 {
                ID_I_MAKE => {
                    let checked = |id| SendMessageW(GetDlgItem(hwnd, id).unwrap_or_default(), BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1;
                    let role = if checked(ID_I_GRANT) { "invite" } else if checked(ID_I_WRITE) { "write" } else { "read" };
                    let pick = SendMessageW(GetDlgItem(hwnd, ID_I_DAYS).unwrap_or_default(), CB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
                    let ms = [1u64, 7, 30][pick.clamp(0, 2) as usize] * 86_400_000;
                    let ns = INVITING.with(|i| i.borrow().clone());
                    set_text(GetDlgItem(hwnd, ID_I_LINK).unwrap_or_default(), "Making it…");
                    let hw = hwnd.0 as isize;
                    std::thread::spawn(move || {
                        *INVITE_LINK.lock().unwrap() = match Daemon::new().invite(&ns, role, ms) {
                            Ok(link) => link,
                            Err(e) => format!("error: {e}"),
                        };
                        let _ = PostMessageW(HWND(hw as _), WM_INVITED, WPARAM(0), LPARAM(0));
                        poke();
                    });
                }
                ID_I_COPY => {
                    let link = text_of(GetDlgItem(hwnd, ID_I_LINK).unwrap_or_default());
                    copy(hwnd, &link);
                    note("The invitation is copied");
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_INVITED => {
            let link = INVITE_LINK.lock().unwrap().clone();
            set_text(GetDlgItem(hwnd, ID_I_LINK).unwrap_or_default(), &link);
            let _ = EnableWindow(GetDlgItem(hwnd, ID_I_COPY).unwrap_or_default(), !link.starts_with("error"));
            // An open members window shows the new invitation.
            if let Some((h, ns, _, _)) = MEMBERS.with(|m| m.borrow().clone()) {
                read_members(h, &ns);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// The new library window.

unsafe fn ask_new(owner: HWND) {
    let Some(h) = popup(owner, NEW_CLASS, "New library", 440, 220) else { return };
    let s = |v| scale(owner, v);
    let f = font();
    let l = child(h, WC_STATICW, "Name", WS_VISIBLE, 0, f);
    let _ = MoveWindow(l, s(14), s(18), s(80), s(24), true);
    let name = child(h, WC_EDITW, "", WS_VISIBLE | WS_BORDER | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32), ID_N_NAME, f);
    let _ = MoveWindow(name, s(100), s(14), s(310), s(26), true);
    let public = child(h, WC_BUTTONW, "Announce it on the network (public: anyone may read it)", WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32), ID_N_PUBLIC, f);
    let _ = MoveWindow(public, s(14), s(52), s(400), s(24), true);
    let hint = child(h, WC_STATICW, "A private one is shared from its menu (right-click): members and invitations.", WS_VISIBLE, 0, f);
    let _ = MoveWindow(hint, s(14), s(84), s(400), s(36), true);
    let ok = child(h, WC_BUTTONW, "Create", WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32), ID_N_OK, f);
    let cancel = child(h, WC_BUTTONW, "Cancel", WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), ID_N_CANCEL, f);
    let _ = MoveWindow(ok, s(220), s(136), s(90), s(28), true);
    let _ = MoveWindow(cancel, s(318), s(136), s(90), s(28), true);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(name);
}

unsafe extern "system" fn new_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            match (wparam.0 & 0xffff) as i32 {
                ID_N_OK => {
                    let name = text_of(GetDlgItem(hwnd, ID_N_NAME).unwrap_or_default()).trim().to_string();
                    if name.is_empty() {
                        return LRESULT(0);
                    }
                    let public = SendMessageW(GetDlgItem(hwnd, ID_N_PUBLIC).unwrap_or_default(), BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1;
                    let _ = DestroyWindow(hwnd);
                    spawn("Making the library", move || {
                        let d = Daemon::new();
                        d.create_namespace(&name)?;
                        if public {
                            d.announce(&name, true)?;
                        }
                        Ok(())
                    });
                }
                ID_N_CANCEL => {
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Unused for now (the main window keeps its own handle).
#[allow(dead_code)]
fn main_window() -> HWND {
    HWND(MAIN.load(std::sync::atomic::Ordering::SeqCst) as _)
}
