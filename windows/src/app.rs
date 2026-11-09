//! The window (Overview, Network, Settings), the notification-area icon and
//! the worker thread that asks the daemon and Reticulum every few seconds.
//! The UI thread only draws: everything that can block runs on a thread and
//! posts a message back.

use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use retifs_desktop_core::device::{self, Configured, Reticulum};
use retifs_desktop_core::status_icon::{self, Health, TrayState};
use retifs_desktop_core::{human, Daemon, Status};
use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: PCWSTR = w!("retifs-desktop");
const ADD_CLASS: PCWSTR = w!("retifs-desktop-add");
const WM_TRAY: u32 = WM_APP + 1;
const WM_REFRESHED: u32 = WM_APP + 2;
const WM_NOTE: u32 = WM_APP + 3;

const ID_TABS: i32 = 100;
const ID_START: i32 = 102;
const ID_STOP: i32 = 103;
const ID_RESTART: i32 = 104;
const ID_LIST: i32 = 111;
const ID_ADD: i32 = 112;
const ID_TOGGLE: i32 = 113;
const ID_REMOVE: i32 = 114;
const ID_SAVE: i32 = 120;
const ID_ADD_OK: i32 = 201;
const ID_ADD_CANCEL: i32 = 202;
const IDM_OPEN: u32 = 1;
const IDM_RESTART: u32 = 2;
const IDM_QUIT: u32 = 3;

/// What the worker found, for the UI thread to show.
#[derive(Clone, Default)]
struct Snapshot {
    status: Status,
    libraries: usize,
    reticulum: Reticulum,
    configured: Vec<Configured>,
    services: Vec<Option<String>>,
    /// Daemon::libraries, and the libraries heard (the Libraries and Announces pages).
    libs: Vec<serde_json::Value>,
    heard: Vec<retifs_desktop_core::HeardLibrary>,
}

static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);
static NOTE: Mutex<String> = Mutex::new(String::new());
/// The main window, for the worker threads to post to.
pub(crate) static MAIN: AtomicIsize = AtomicIsize::new(0);
/// Wakes the worker before its next round (after a change).
static POKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

struct Ui {
    hwnd: HWND,
    tabs: HWND,
    /// Overview, Libraries, Announces, Network, Settings.
    pages: [Vec<HWND>; 5],
    lib_pages: crate::libraries::Pages,
    overview: HWND,
    net: HWND,
    list: HWND,
    edits: Vec<(&'static str, &'static str, HWND, HWND)>,
    note: HWND,
    font: HFONT,
    icons: [HICON; 3],
    taskbar_created: u32,
    snapshot: Snapshot,
    add: Option<HWND>,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    static SNAPSHOT_PATH: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The window as it's drawn, to a .bmp file (PrintWindow: works off screen).
unsafe fn snapshot(hwnd: HWND, path: &str) {
    let mut rc = RECT::default();
    let _ = GetWindowRect(hwnd, &mut rc);
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    let screen = GetDC(None);
    let dc = CreateCompatibleDC(screen);
    let bmp = CreateCompatibleBitmap(screen, w, h);
    let old = SelectObject(dc, bmp);
    let _ = PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(2)); // PW_RENDERFULLCONTENT
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() },
        ..Default::default()
    };
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    GetDIBits(dc, bmp, 0, h as u32, Some(pixels.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
    SelectObject(dc, old);
    let _ = DeleteObject(bmp);
    let _ = DeleteDC(dc);
    ReleaseDC(None, screen);
    // A plain 32-bit top-down BMP.
    let mut file = Vec::with_capacity(54 + pixels.len());
    file.extend_from_slice(b"BM");
    file.extend_from_slice(&(54 + pixels.len() as u32).to_le_bytes());
    file.extend_from_slice(&[0, 0, 0, 0]);
    file.extend_from_slice(&54u32.to_le_bytes());
    file.extend_from_slice(&40u32.to_le_bytes());
    file.extend_from_slice(&w.to_le_bytes());
    file.extend_from_slice(&(-h).to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&32u16.to_le_bytes());
    file.extend_from_slice(&[0; 24]);
    file.extend_from_slice(&pixels);
    let _ = std::fs::write(path, file);
}

pub(crate) fn wide(s: &str) -> HSTRING {
    HSTRING::from(s)
}

fn post(msg: u32) {
    let h = MAIN.load(Ordering::SeqCst);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(HWND(h as _), msg, WPARAM(0), LPARAM(0));
        }
    }
}

/// Says something at the bottom of the window (from any thread).
pub(crate) fn note(text: impl Into<String>) {
    *NOTE.lock().unwrap() = text.into();
    post(WM_NOTE);
}

pub(crate) fn poke() {
    let (m, c) = &POKE;
    *m.lock().unwrap() = true;
    c.notify_all();
}

/// Runs `work` on a thread; its error (if any) goes to the note line, and
/// the worker looks again.
pub(crate) fn spawn(what: &'static str, work: impl FnOnce() -> Result<(), String> + Send + 'static) {
    note(format!("{what}…"));
    std::thread::spawn(move || {
        match work() {
            Ok(()) => note(format!("{what}: done")),
            Err(e) => note(format!("{what}: {e}")),
        }
        poke();
    });
}

/// Where the utility writes what went wrong (a windowed program has no console).
fn log_line(text: &str) {
    use std::io::Write;
    let dir = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("retifs").join("logs");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("retifs-desktop.log")) {
        let _ = writeln!(f, "{text}");
    }
}

pub fn run() {
    std::panic::set_hook(Box::new(|info| log_line(&format!("panic: {info}"))));
    let autostart = std::env::args().any(|a| a == "--autostart");
    // --show: the window too (the installer's first start).
    let show = std::env::args().any(|a| a == "--show");
    unsafe {
        // One instance: started again, the running one shows its window.
        // (Not for snapshots: a test copy mustn't bring the user's to the front.)
        let snapshotting = std::env::var_os("RETIFS_DESKTOP_SNAPSHOT").is_some();
        let _single = CreateMutexW(None, true, w!("Local\\retifs-desktop"));
        if GetLastError() == ERROR_ALREADY_EXISTS && !snapshotting {
            if let Ok(h) = FindWindowW(CLASS, None) {
                let _ = ShowWindow(h, SW_SHOWNORMAL);
                let _ = SetForegroundWindow(h);
            }
            return;
        }
        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_TAB_CLASSES | ICC_LISTVIEW_CLASSES | ICC_STANDARD_CLASSES });
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let icons = [icon(Health::Good), icon(Health::Partly), icon(Health::Down)];
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as _),
            hIcon: icons[0],
            ..Default::default()
        };
        RegisterClassW(&class);
        let add_class = WNDCLASSW { lpfnWndProc: Some(addproc), lpszClassName: ADD_CLASS, ..class };
        RegisterClassW(&add_class);
        crate::libraries::register(&class);
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!("retifs"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            760,
            560,
            None,
            None,
            instance,
            None,
        ) else {
            return;
        };
        MAIN.store(hwnd.0 as isize, Ordering::SeqCst);
        build(hwnd, icons);
        // At login: retifs once Reticulum (Prns's) is up; the window stays hidden.
        if autostart {
            std::thread::spawn(|| {
                let did = device::supervise();
                if !did.is_empty() {
                    note(did.join("; "));
                }
                poke();
                // From then on: retifsd kept running, and restarted after Reticulum was.
                let mut away = 0;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(10));
                    let did = device::keep(&mut away);
                    if !did.is_empty() {
                        note(did.join("; "));
                        poke();
                    }
                }
            });
            if show {
                let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
            }
        } else if let Ok(path) = std::env::var("RETIFS_DESKTOP_SNAPSHOT") {
            // Demos and screenshots: drawn off screen, never activated, to a
            // file every 2 s (RETIFS_DESKTOP_PAGE=1 or 2 for another tab).
            let _ = SetWindowPos(hwnd, None, -4000, 100, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            if let Some(page) = std::env::var("RETIFS_DESKTOP_PAGE").ok().and_then(|p| p.parse::<usize>().ok()) {
                let tabs = UI.with(|u| u.borrow().as_ref().map(|u| u.tabs)).unwrap_or_default();
                SendMessageW(tabs, TCM_SETCURSEL, WPARAM(page), LPARAM(0));
                show_page(page);
            }
            SNAPSHOT_PATH.with(|p| *p.borrow_mut() = Some(path));
            SetTimer(hwnd, 7, 2000, None);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        }
        std::thread::spawn(worker);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // Tab between controls, Enter for the default button.
            let dialog = UI.with(|u| u.borrow().as_ref().and_then(|u| u.add)).unwrap_or(hwnd);
            if IsDialogMessageW(dialog, &msg).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Asks the daemon, Reticulum and the services, every 5 s or when poked.
fn worker() {
    let daemon = Daemon::new();
    loop {
        let status = daemon.status();
        let libraries = if status.up { daemon.namespace_names().map(|n| n.len()).unwrap_or(0) } else { 0 };
        let (libs, heard) = if status.up { crate::libraries::fetch(&daemon) } else { Default::default() };
        let snapshot = Snapshot {
            status,
            libraries,
            reticulum: device::reticulum(),
            configured: device::configured().unwrap_or_default(),
            services: device::SERVICES.iter().map(|(s, _)| device::service_state(s)).collect(),
            libs,
            heard,
        };
        *SNAPSHOT.lock().unwrap() = Some(snapshot);
        post(WM_REFRESHED);
        let (m, c) = &POKE;
        let mut poked = m.lock().unwrap();
        if !*poked {
            poked = c.wait_timeout(poked, Duration::from_secs(5)).unwrap().0;
        }
        *poked = false;
    }
}

/// The icon for `health`, from the core's drawing (ARGB, which is the
/// byte order of a 32-bit Windows bitmap).
unsafe fn icon(health: Health) -> HICON {
    let size = 32;
    let pixels = status_icon::draw(size as u32, health);
    let header = BITMAPV5HEADER {
        bV5Size: std::mem::size_of::<BITMAPV5HEADER>() as u32,
        bV5Width: size,
        bV5Height: -size,
        bV5Planes: 1,
        bV5BitCount: 32,
        bV5Compression: BI_BITFIELDS,
        bV5RedMask: 0x00ff_0000,
        bV5GreenMask: 0x0000_ff00,
        bV5BlueMask: 0x0000_00ff,
        bV5AlphaMask: 0xff00_0000,
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let dc = GetDC(None);
    let color = CreateDIBSection(dc, &header as *const _ as *const BITMAPINFO, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
    ReleaseDC(None, dc);
    if !bits.is_null() {
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u32, pixels.len());
    }
    let mask = CreateBitmap(size, size, 1, 1, None);
    let info = ICONINFO { fIcon: TRUE, hbmMask: mask, hbmColor: color, ..Default::default() };
    let icon = CreateIconIndirect(&info).unwrap_or_default();
    let _ = DeleteObject(color);
    let _ = DeleteObject(mask);
    icon
}

pub(crate) unsafe fn child(parent: HWND, class: PCWSTR, text: &str, style: WINDOW_STYLE, id: i32, font: HFONT) -> HWND {
    let h = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        class,
        &wide(text),
        WS_CHILD | style,
        0,
        0,
        10,
        10,
        parent,
        HMENU(id as _),
        GetModuleHandleW(None).unwrap_or_default(),
        None,
    )
    .unwrap_or_default();
    SendMessageW(h, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    h
}

unsafe fn message_font() -> HFONT {
    let mut m = NONCLIENTMETRICSW { cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
    let _ = SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, m.cbSize, Some(&mut m as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    CreateFontIndirectW(&m.lfMessageFont)
}

unsafe fn build(hwnd: HWND, icons: [HICON; 3]) {
    let font = message_font();
    let tabs = child(hwnd, WC_TABCONTROLW, "", WS_VISIBLE | WS_CLIPSIBLINGS | WS_TABSTOP, ID_TABS, font);
    for (i, t) in ["Overview", "Libraries", "Announces", "Network", "Settings"].iter().enumerate() {
        let text = wide(t);
        let item = TCITEMW { mask: TCIF_TEXT, pszText: PWSTR(text.as_ptr() as *mut u16), ..Default::default() };
        SendMessageW(tabs, TCM_INSERTITEMW, WPARAM(i), LPARAM(&item as *const _ as isize));
    }
    let label = |text: &str| child(hwnd, WC_STATICW, text, WINDOW_STYLE(0), 0, font);
    let button = |text: &str, id: i32| child(hwnd, WC_BUTTONW, text, WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), id, font);

    // Overview.
    let overview = label("Asking retifs…");
    let start = button("Start retifs", ID_START);
    let stop = button("Stop retifs", ID_STOP);
    let restart = button("Restart retifs", ID_RESTART);

    // Network.
    let net = label("Asking Reticulum…");
    let list = child(hwnd, WC_LISTVIEWW, "", WS_BORDER | WS_TABSTOP | WINDOW_STYLE((LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS) as u32), ID_LIST, font);
    SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, WPARAM(0), LPARAM((LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize));
    for (i, (title, width)) in [("Connection", 180), ("Type", 130), ("State", 120), ("Traffic", 180)].iter().enumerate() {
        let text = wide(title);
        let col = LVCOLUMNW { mask: LVCF_TEXT | LVCF_WIDTH, cx: *width, pszText: PWSTR(text.as_ptr() as *mut u16), ..Default::default() };
        SendMessageW(list, LVM_INSERTCOLUMNW, WPARAM(i), LPARAM(&col as *const _ as isize));
    }
    let add = button("Add connection…", ID_ADD);
    let toggle = button("Switch on/off", ID_TOGGLE);
    let remove = button("Remove", ID_REMOVE);

    // Settings.
    let mut edits = Vec::new();
    let mut settings_page = Vec::new();
    for s in device::settings_view(&retifs_desktop_core::default_store()) {
        let (section, key) = (s.section, s.key);
        let l = if s.unit.is_empty() { label(s.what) } else { label(&format!("{} ({})", s.what, s.unit)) };
        let e = if key == "no_storage" {
            child(hwnd, WC_BUTTONW, "Yes", WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32), 0, font)
        } else {
            child(hwnd, WC_EDITW, "", WS_BORDER | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32), 0, font)
        };
        settings_page.push(l);
        settings_page.push(e);
        edits.push((section, key, l, e));
    }
    let lib_pages = crate::libraries::build(hwnd, font);
    let save = button("Save and restart retifs", ID_SAVE);
    settings_page.push(save);
    let note = label("");
    let _ = ShowWindow(note, SW_SHOW);

    UI.with(|u| {
        *u.borrow_mut() = Some(Ui {
            hwnd,
            tabs,
            pages: [
                vec![overview, start, stop, restart],
                [vec![lib_pages.libs], lib_pages.libs_buttons.clone()].concat(),
                vec![lib_pages.heard_note, lib_pages.heard, lib_pages.join],
                vec![net, list, add, toggle, remove],
                settings_page,
            ],
            lib_pages,
            overview,
            net,
            list,
            edits,
            note,
            font,
            icons,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            snapshot: Snapshot::default(),
            add: None,
        })
    });
    load_settings();
    show_page(0);
    add_tray(hwnd, icons[2], "retifs: starting");
    layout(hwnd);
}

pub(crate) unsafe fn scale(hwnd: HWND, v: i32) -> i32 {
    v * GetDpiForWindow(hwnd).max(96) as i32 / 96
}

unsafe fn layout(hwnd: HWND) {
    UI.with(|u| {
        let u = u.borrow();
        let Some(u) = u.as_ref() else { return };
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let s = |v| scale(hwnd, v);
        let (w, h) = (rc.right, rc.bottom);
        let m = s(12);
        // The tabs as a strip at the top; the page below it, on the window.
        let strip = s(30);
        let _ = MoveWindow(u.tabs, m, m, w - 2 * m, strip, true);
        let (x, y, pw, ph) = (m + s(4), m + strip + s(12), w - 2 * m - s(8), h - 2 * m - strip - s(12) - s(30));
        let (bw, bh) = (s(150), s(28));
        let place = |h: HWND, x: i32, y: i32, w: i32, hh: i32| {
            let _ = MoveWindow(h, x, y, w, hh, true);
        };
        // Overview: text, then the buttons.
        place(u.overview, x, y, pw, ph - bh - s(10));
        for (i, b) in u.pages[0][1..].iter().enumerate() {
            place(*b, x + i as i32 * (bw + s(8)), y + ph - bh, bw, bh);
        }
        crate::libraries::layout(&u.lib_pages, hwnd, x, y, pw, ph);
        // Network: a line, the list, the buttons.
        place(u.net, x, y, pw, s(40));
        place(u.list, x, y + s(46), pw, ph - s(46) - bh - s(10));
        for (i, b) in u.pages[3][2..].iter().enumerate() {
            place(*b, x + i as i32 * (bw + s(8)), y + ph - bh, bw, bh);
        }
        // Settings: a label and a field a row.
        let lw = (pw * 55 / 100).min(s(380));
        for (i, (_, _, l, e)) in u.edits.iter().enumerate() {
            let ry = y + i as i32 * s(34);
            place(*l, x, ry + s(4), lw - s(8), s(24));
            place(*e, x + lw, ry, pw - lw, s(26));
        }
        if let Some(save) = u.pages[4].last() {
            place(*save, x + pw - s(200), y + ph - bh, s(200), bh);
        }
        place(u.note, m, h - m - s(20), w - 2 * m, s(20));
    });
}

unsafe fn show_page(n: usize) {
    UI.with(|u| {
        let u = u.borrow();
        let Some(u) = u.as_ref() else { return };
        for (i, page) in u.pages.iter().enumerate() {
            for h in page {
                let _ = ShowWindow(*h, if i == n { SW_SHOW } else { SW_HIDE });
            }
        }
    });
}

pub(crate) unsafe fn set_text(h: HWND, text: &str) {
    let _ = SetWindowTextW(h, &wide(text));
}

pub(crate) unsafe fn text_of(h: HWND) -> String {
    let len = GetWindowTextLengthW(h);
    let mut buf = vec![0u16; len as usize + 1];
    let n = GetWindowTextW(h, &mut buf);
    String::from_utf16_lossy(&buf[..n as usize])
}

fn tray_state(s: &Snapshot) -> TrayState {
    let outward: Vec<_> = s.reticulum.interfaces.iter().filter(|i| !i.is_local()).collect();
    TrayState {
        retifs_up: s.status.up,
        node: s.status.node.clone(),
        reticulum_up: s.reticulum.running,
        links_up: outward.iter().filter(|i| i.up).count(),
        links: outward.len(),
    }
}

unsafe fn tray_data(hwnd: HWND, icon: HICON, tip: &str) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    for (d, s) in data.szTip.iter_mut().zip(tip.encode_utf16().take(127)) {
        *d = s;
    }
    data
}

unsafe fn add_tray(hwnd: HWND, icon: HICON, tip: &str) {
    let _ = Shell_NotifyIconW(NIM_ADD, &tray_data(hwnd, icon, tip));
}

/// Shows what the worker found.
unsafe fn refreshed() {
    let Some(s) = SNAPSHOT.lock().unwrap().take() else { return };
    // Drawing sends messages, which come back to wndproc: only shared
    // borrows while drawing; the snapshot is kept after.
    UI.with(|u| {
        let u = u.borrow();
        let Some(u) = u.as_ref() else { return };
        let st = &s.status;
        let mut lines = vec![match &st.node {
            Some(n) if st.up => format!("retifs is running.\r\nNode {n}"),
            _ => format!("retifs isn't running ({}).", st.error.clone().unwrap_or_default()),
        }];
        lines.push(format!("Store: {}", st.store));
        if let Some(us) = &st.usage {
            lines.push(String::new());
            lines.push(format!("Own data:  {} of {}", human(us.pinned), human(us.store_limit)));
            lines.push(format!("Held for others:  {}", human(us.leased)));
            lines.push(format!("Cache:  {} of {}", human(us.cache), human(us.cache_max)));
            lines.push(format!("Disk:  {} free of {}", human(us.disk_available), human(us.disk_total)));
        }
        if st.up {
            lines.push(format!("Libraries: {}", s.libraries));
        }
        lines.push(String::new());
        for ((name, what), state) in device::SERVICES.iter().zip(&s.services) {
            lines.push(format!("{what} ({name}.exe): {}", state.clone().unwrap_or_else(|| "?".into())));
        }
        set_text(u.overview, &lines.join("\r\n"));

        let r = &s.reticulum;
        let bin = r.prnsd.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "prnsd not found".into());
        set_text(
            u.net,
            &if r.running {
                let up = r.uptime_s.map(|x| if x > 86_400.0 { format!("{:.0} d", x / 86_400.0) } else { format!("{:.0} h", x / 3600.0) }).unwrap_or_default();
                format!("Reticulum is running: transport {}, up {up}.\r\n{bin}", r.transport_id.clone().unwrap_or_default())
            } else {
                format!("Reticulum isn't running: {}\r\n{bin}", r.error.clone().unwrap_or_default())
            },
        );
        // The connections, keeping the selection.
        let selected = SendMessageW(u.list, LVM_GETNEXTITEM, WPARAM(usize::MAX), LPARAM(LVNI_SELECTED as isize)).0;
        SendMessageW(u.list, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
        for (i, c) in s.configured.iter().enumerate() {
            let live = r.interfaces.iter().find(|x| x.name == c.name);
            let state = match live {
                Some(x) if x.up => "connected",
                Some(_) => "not connected",
                None if c.enabled => "on",
                None => "off",
            };
            let traffic = live.map(|x| format!("↓ {}  ↑ {}", human(x.rx), human(x.tx))).unwrap_or_default();
            for (col, text) in [c.name.clone(), c.kind.trim_end_matches("Interface").to_string(), state.to_string(), traffic].iter().enumerate() {
                let t = wide(text);
                let item = LVITEMW { mask: LVIF_TEXT, iItem: i as i32, iSubItem: col as i32, pszText: PWSTR(t.as_ptr() as *mut u16), ..Default::default() };
                let msg = if col == 0 { LVM_INSERTITEMW } else { LVM_SETITEMTEXTW };
                SendMessageW(u.list, msg, WPARAM(i), LPARAM(&item as *const _ as isize));
            }
        }
        if selected >= 0 && (selected as usize) < s.configured.len() {
            let item = LVITEMW { stateMask: LVIS_SELECTED, state: LVIS_SELECTED, ..Default::default() };
            SendMessageW(u.list, LVM_SETITEMSTATE, WPARAM(selected as usize), LPARAM(&item as *const _ as isize));
        }

        crate::libraries::show(&u.lib_pages, &s.libs, &s.heard);

        let t = tray_state(&s);
        let icon = u.icons[match t.health() {
            Health::Good => 0,
            Health::Partly => 1,
            Health::Down => 2,
        }];
        let _ = Shell_NotifyIconW(NIM_MODIFY, &tray_data(u.hwnd, icon, &t.lines().join("\n")));
        let _ = SendMessageW(u.hwnd, WM_SETICON, WPARAM(ICON_SMALL as usize), LPARAM(icon.0 as isize));
    });
    UI.with(|u| {
        if let Some(u) = u.borrow_mut().as_mut() {
            u.snapshot = s;
        }
    });
}

/// Fills the fields: the file's values, else the defaults this device gets.
unsafe fn load_settings() {
    let views = device::settings_view(&retifs_desktop_core::default_store());
    UI.with(|u| {
        let u = u.borrow();
        let Some(u) = u.as_ref() else { return };
        for (section, key, _, e) in &u.edits {
            let Some(v) = views.iter().find(|v| v.section == *section && v.key == *key) else { continue };
            if *key == "no_storage" {
                SendMessageW(*e, BM_SETCHECK, WPARAM(if v.value == "true" { 1 } else { 0 }), LPARAM(0));
            } else {
                set_text(*e, &v.value);
            }
        }
    });
}

/// Writes the settings that changed, then restarts retifs to use them.
unsafe fn save_settings() {
    let typed: Vec<(&str, &str, String)> = UI.with(|u| {
        let u = u.borrow();
        let Some(u) = u.as_ref() else { return Vec::new() };
        u.edits
            .iter()
            .map(|(section, key, _, e)| {
                let t = if *key == "no_storage" {
                    if SendMessageW(*e, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1 { "true".to_string() } else { String::new() }
                } else {
                    text_of(*e)
                };
                (*section, *key, t)
            })
            .collect()
    });
    if let Err(e) = device::save_typed(&retifs_desktop_core::default_store(), &typed) {
        note(e);
        return;
    }
    load_settings();
    spawn("Settings saved; restarting retifs", || device::service(device::RETIFSD, "restart"));
}

unsafe fn selected_connection() -> Option<Configured> {
    UI.with(|u| {
        let u = u.borrow();
        let u = u.as_ref()?;
        let i = SendMessageW(u.list, LVM_GETNEXTITEM, WPARAM(usize::MAX), LPARAM(LVNI_SELECTED as isize)).0;
        (i >= 0).then(|| u.snapshot.configured.get(i as usize).cloned()).flatten()
    })
}

unsafe fn tray_menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let lines = UI.with(|u| u.borrow().as_ref().map(|u| tray_state(&u.snapshot).lines()).unwrap_or_default());
    for l in &lines {
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &wide(l));
    }
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
    let _ = AppendMenuW(menu, MF_STRING, IDM_OPEN as usize, w!("Open retifs"));
    let _ = AppendMenuW(menu, MF_STRING, IDM_RESTART as usize, w!("Restart retifs"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
    let _ = AppendMenuW(menu, MF_STRING, IDM_QUIT as usize, w!("Quit"));
    let _ = SetMenuDefaultItem(menu, IDM_OPEN, 0);
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, None);
    let _ = DestroyMenu(menu);
    match cmd.0 as u32 {
        IDM_OPEN => open(hwnd),
        IDM_RESTART => spawn("Restarting retifs", || device::service(device::RETIFSD, "restart")),
        IDM_QUIT => {
            let _ = DestroyWindow(hwnd);
        }
        _ => {}
    }
}

unsafe fn open(hwnd: HWND) {
    let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
    let _ = SetForegroundWindow(hwnd);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let taskbar_created = UI.with(|u| u.borrow().as_ref().map(|u| u.taskbar_created)).unwrap_or(0);
    match msg {
        // Labels and checkboxes on the window's own background.
        WM_CTLCOLORSTATIC => {
            let dc = HDC(wparam.0 as _);
            SetBkColor(dc, COLORREF(GetSysColor(COLOR_WINDOW)));
            LRESULT(GetSysColorBrush(COLOR_WINDOW).0 as isize)
        }
        WM_SIZE => {
            layout(hwnd);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let r = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            layout(hwnd);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let m = &mut *(lparam.0 as *mut MINMAXINFO);
            m.ptMinTrackSize = POINT { x: scale(hwnd, 620), y: scale(hwnd, 440) };
            LRESULT(0)
        }
        WM_NOTIFY => {
            let hdr = &*(lparam.0 as *const NMHDR);
            if hdr.idFrom == ID_TABS as usize && hdr.code == TCN_SELCHANGE {
                let tabs = UI.with(|u| u.borrow().as_ref().map(|u| u.tabs)).unwrap_or_default();
                show_page(SendMessageW(tabs, TCM_GETCURSEL, WPARAM(0), LPARAM(0)).0 as usize);
            }
            // A library right-clicked: its menu (no borrow held: the menu's
            // own loop dispatches messages here).
            if hdr.idFrom == crate::libraries::ID_LIBS as usize && hdr.code == NM_RCLICK {
                if let Some(p) = UI.with(|u| u.borrow().as_ref().map(|u| u.lib_pages.clone())) {
                    crate::libraries::context(&p, hwnd);
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            match (wparam.0 & 0xffff) as i32 {
                ID_START => spawn("Starting retifs", || device::service(device::RETIFSD, "start")),
                ID_STOP => spawn("Stopping retifs", || device::service(device::RETIFSD, "stop")),
                ID_RESTART => spawn("Restarting retifs", || device::service(device::RETIFSD, "restart")),
                ID_ADD => ask_add(hwnd),
                ID_TOGGLE => match selected_connection() {
                    Some(c) => spawn(if c.enabled { "Switching the connection off" } else { "Switching the connection on" }, move || device::set_interface(&c.name, !c.enabled)),
                    None => note("Pick a connection first"),
                },
                ID_REMOVE => match selected_connection() {
                    Some(c) => {
                        let q = wide(&format!("Remove the connection {}?", c.name));
                        if MessageBoxW(hwnd, &q, w!("retifs"), MB_YESNO | MB_ICONQUESTION) == IDYES {
                            spawn("Removing the connection", move || device::remove_interface(&c.name));
                        }
                    }
                    None => note("Pick a connection first"),
                },
                ID_SAVE => save_settings(),
                id => {
                    if let Some(p) = UI.with(|u| u.borrow().as_ref().map(|u| u.lib_pages.clone())) {
                        crate::libraries::command(&p, hwnd, id);
                    }
                }
            }
            LRESULT(0)
        }
        // A library's link, made on a thread: to the clipboard.
        m if m == WM_APP + 12 => {
            crate::libraries::link_ready(hwnd);
            LRESULT(0)
        }
        WM_TRAY => {
            match (lparam.0 & 0xffff) as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => open(hwnd),
                WM_RBUTTONUP | WM_CONTEXTMENU => tray_menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_REFRESHED => {
            refreshed();
            LRESULT(0)
        }
        WM_TIMER => {
            if let Some(path) = SNAPSHOT_PATH.with(|p| p.borrow().clone()) {
                snapshot(hwnd, &path);
            }
            LRESULT(0)
        }
        WM_NOTE => {
            let text = NOTE.lock().unwrap().clone();
            let note = UI.with(|u| u.borrow().as_ref().map(|u| u.note)).unwrap_or_default();
            set_text(note, &text);
            LRESULT(0)
        }
        // Closed, it stays in the notification area; Quit is in its menu.
        WM_CLOSE => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = Shell_NotifyIconW(NIM_DELETE, &tray_data(hwnd, HICON::default(), ""));
            UI.with(|u| {
                if let Some(u) = u.borrow().as_ref() {
                    let _ = DeleteObject(u.font);
                }
            });
            PostQuitMessage(0);
            LRESULT(0)
        }
        // Explorer restarted: the icon again.
        m if m == taskbar_created && m != 0 => {
            let icon = UI.with(|u| u.borrow().as_ref().map(|u| u.icons[2])).unwrap_or_default();
            add_tray(hwnd, icon, "retifs");
            poke();
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// The Add connection window: name, host, port.
unsafe fn ask_add(owner: HWND) {
    if let Some(h) = UI.with(|u| u.borrow().as_ref().and_then(|u| u.add)) {
        let _ = SetForegroundWindow(h);
        return;
    }
    let s = |v| scale(owner, v);
    let Ok(h) = CreateWindowExW(
        WS_EX_DLGMODALFRAME,
        ADD_CLASS,
        w!("Add a connection"),
        WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        s(420),
        s(260),
        owner,
        None,
        GetModuleHandleW(None).unwrap_or_default(),
        None,
    ) else {
        return;
    };
    let font = UI.with(|u| u.borrow().as_ref().map(|u| u.font)).unwrap_or_default();
    let intro = child(h, WC_STATICW, "A TCP connection to another Reticulum node: someone's you trust, or a public one. Everything retifs sends is encrypted end to end either way.", WS_VISIBLE, 0, font);
    let _ = MoveWindow(intro, s(14), s(10), s(380), s(52), true);
    let mut fields = Vec::new();
    for (i, (label, value)) in [("Name", ""), ("Host", ""), ("Port", "4242")].iter().enumerate() {
        let l = child(h, WC_STATICW, label, WS_VISIBLE, 0, font);
        let e = child(h, WC_EDITW, value, WS_VISIBLE | WS_BORDER | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32), 300 + i as i32, font);
        let _ = MoveWindow(l, s(14), s(72 + i as i32 * 34), s(80), s(24), true);
        let _ = MoveWindow(e, s(100), s(68 + i as i32 * 34), s(290), s(26), true);
        fields.push(e);
    }
    let ok = child(h, WC_BUTTONW, "Add", WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32), ID_ADD_OK, font);
    let cancel = child(h, WC_BUTTONW, "Cancel", WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), ID_ADD_CANCEL, font);
    let _ = MoveWindow(ok, s(210), s(178), s(90), s(28), true);
    let _ = MoveWindow(cancel, s(306), s(178), s(90), s(28), true);
    let _ = SetFocus(fields[0]);
    let _ = EnableWindow(owner, false);
    UI.with(|u| {
        if let Some(u) = u.borrow_mut().as_mut() {
            u.add = Some(h);
        }
    });
}

unsafe extern "system" fn addproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let close = |hwnd: HWND| {
        let owner = HWND(MAIN.load(Ordering::SeqCst) as _);
        let _ = EnableWindow(owner, true);
        let _ = DestroyWindow(hwnd);
        UI.with(|u| {
            if let Some(u) = u.borrow_mut().as_mut() {
                u.add = None;
            }
        });
        let _ = SetForegroundWindow(owner);
    };
    match msg {
        WM_COMMAND => {
            match (wparam.0 & 0xffff) as i32 {
                ID_ADD_OK => {
                    let field = |id: i32| text_of(GetDlgItem(hwnd, id).unwrap_or_default()).trim().to_string();
                    let (name, host, port) = (field(300), field(301), field(302));
                    let Ok(port) = port.parse::<u16>() else {
                        MessageBoxW(hwnd, w!("The port is a number (4242 by default)."), w!("retifs"), MB_OK | MB_ICONWARNING);
                        return LRESULT(0);
                    };
                    if host.is_empty() {
                        MessageBoxW(hwnd, w!("Which host? A name or an address."), w!("retifs"), MB_OK | MB_ICONWARNING);
                        return LRESULT(0);
                    }
                    let name = if name.is_empty() { host.clone() } else { name };
                    close(hwnd);
                    spawn("Adding the connection", move || device::add_tcp(&name, &host, port));
                }
                ID_ADD_CANCEL => close(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            close(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
