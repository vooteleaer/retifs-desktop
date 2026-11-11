//! The recovery kit (retifs ADR-0033): making one (the words shown once, in
//! a window that closes only once the user says they wrote them down), and
//! recovering from one on a new device (the words checked as they're typed).

use std::sync::Mutex;

use retifs_desktop_core::{check_recovery_words, Daemon, RecoveryKit, RECOVERY_ABOUT};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{child, note, scale, set_text, text_of, wide, MAIN};

const WORDS_CLASS: PCWSTR = w!("retifs-desktop-kit");
const RECOVER_CLASS: PCWSTR = w!("retifs-desktop-recover");
/// Posted to the main window: a kit was made (or failed), or a recovery ended.
pub const WM_KIT: u32 = WM_APP + 20;
pub const WM_RECOVERED: u32 = WM_APP + 21;

const ID_WRITTEN: i32 = 401;
const ID_DONE: i32 = 402;
const ID_TYPED: i32 = 411;
const ID_SAY: i32 = 412;
const ID_RECOVER: i32 = 413;
const ID_CANCEL: i32 = 414;

static KIT: Mutex<Option<Result<RecoveryKit, String>>> = Mutex::new(None);
static RECOVERED: Mutex<Option<Result<Vec<String>, String>>> = Mutex::new(None);

pub unsafe fn register(class: &WNDCLASSW) {
    for (name, proc_) in [
        (WORDS_CLASS, words_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT),
        (RECOVER_CLASS, recover_proc),
    ] {
        let c = WNDCLASSW { lpfnWndProc: Some(proc_), lpszClassName: name, ..*class };
        RegisterClassW(&c);
    }
}

fn post(msg: u32) {
    let h = MAIN.load(std::sync::atomic::Ordering::SeqCst);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(HWND(h as _), msg, WPARAM(0), LPARAM(0));
        }
    }
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

unsafe fn ui_font() -> HFONT {
    let mut m = NONCLIENTMETRICSW { cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
    let _ = SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, m.cbSize, Some(&mut m as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    CreateFontIndirectW(&m.lfMessageFont)
}

unsafe fn mono_font(owner: HWND) -> HFONT {
    CreateFontW(-scale(owner, 17), 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET.0 as u32, OUT_DEFAULT_PRECIS.0 as u32, CLIP_DEFAULT_PRECIS.0 as u32, CLEARTYPE_QUALITY.0 as u32, FF_MODERN.0 as u32, w!("Consolas"))
}

unsafe fn close(hwnd: HWND) {
    let owner = HWND(MAIN.load(std::sync::atomic::Ordering::SeqCst) as _);
    let _ = EnableWindow(owner, true);
    let _ = DestroyWindow(hwnd);
    let _ = SetForegroundWindow(owner);
}

/// Make a kit: what it is first, then made on a thread.
pub unsafe fn make(owner: HWND) {
    let text = wide(&format!("{RECOVERY_ABOUT}\n\nMake your recovery kit now?"));
    if MessageBoxW(owner, &text, w!("Make a recovery kit"), MB_YESNO | MB_ICONINFORMATION) != IDYES {
        return;
    }
    note("Making your recovery kit…");
    std::thread::spawn(|| {
        *KIT.lock().unwrap() = Some(Daemon::new().recovery_create());
        post(WM_KIT);
    });
}

/// The main window got the kit: the words, or what went wrong.
pub unsafe fn kit_ready(owner: HWND) {
    let Some(result) = KIT.lock().unwrap().take() else { return };
    let kit = match result {
        Ok(k) => k,
        Err(e) => {
            note("");
            MessageBoxW(owner, &wide(&e), w!("No kit made"), MB_OK | MB_ICONWARNING);
            return;
        }
    };
    note("");
    let Some(h) = popup(owner, WORDS_CLASS, "Your recovery kit", 560, 470) else { return };
    let s = |v| scale(owner, v);
    let font = ui_font();
    let intro = child(h, WC_STATICW, "Write these 24 words down on paper, in order, and keep the paper somewhere safe. They won't be shown again.", WS_VISIBLE, 0, font);
    let _ = MoveWindow(intro, s(16), s(12), s(510), s(44), true);
    // Four columns of six, numbered, as they're written down.
    let mut lines = vec![String::new(); 6];
    for (i, w) in kit.words.iter().enumerate() {
        lines[i % 6].push_str(&format!("{:>2}. {w:<10}", i + 1));
    }
    let words = child(h, WC_EDITW, &lines.join("\r\n"), WS_VISIBLE | WS_BORDER | WINDOW_STYLE((ES_MULTILINE | ES_READONLY) as u32), 0, mono_font(owner));
    let _ = MoveWindow(words, s(16), s(62), s(510), s(150), true);
    let covers = if kit.not_covered.is_empty() {
        format!("It covers all {} of your libraries, and those you make or join later.", kit.covered.len())
    } else {
        format!("It covers {}. Not covered (no device of yours may invite there): {}.", kit.covered.join(", "), kit.not_covered.join(", "))
    };
    let covers = child(h, WC_STATICW, &covers, WS_VISIBLE, 0, font);
    let _ = MoveWindow(covers, s(16), s(224), s(510), s(60), true);
    let written = child(h, WC_BUTTONW, "I have written the 24 words down, in order", WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32), ID_WRITTEN, font);
    let _ = MoveWindow(written, s(16), s(296), s(510), s(28), true);
    let done = child(h, WC_BUTTONW, "Done", WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32), ID_DONE, font);
    let _ = MoveWindow(done, s(426), s(370), s(100), s(30), true);
    let _ = EnableWindow(done, false);
    let _ = EnableWindow(owner, false);
    let _ = SetFocus(written);
}

unsafe extern "system" fn words_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            match (wparam.0 & 0xffff) as i32 {
                ID_WRITTEN => {
                    let checked = SendMessageW(GetDlgItem(hwnd, ID_WRITTEN).unwrap_or_default(), BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == BST_CHECKED.0 as isize;
                    let _ = EnableWindow(GetDlgItem(hwnd, ID_DONE).unwrap_or_default(), checked);
                }
                ID_DONE => close(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        // Not closed before the words are written down: they aren't shown again.
        WM_CLOSE => {
            let checked = SendMessageW(GetDlgItem(hwnd, ID_WRITTEN).unwrap_or_default(), BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == BST_CHECKED.0 as isize;
            if checked
                || MessageBoxW(hwnd, w!("These words won't be shown again. Close without them?"), w!("retifs"), MB_YESNO | MB_ICONWARNING) == IDYES
            {
                close(hwnd);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Recover from a kit: the words typed and checked as they come.
pub unsafe fn recover(owner: HWND) {
    let Some(h) = popup(owner, RECOVER_CLASS, "Recover from a kit", 560, 400) else { return };
    let s = |v| scale(owner, v);
    let font = ui_font();
    let intro = child(h, WC_STATICW, "Type your 24 words, in order. A device of yours that keeps your keyring must be running somewhere on the network.", WS_VISIBLE, 0, font);
    let _ = MoveWindow(intro, s(16), s(12), s(510), s(44), true);
    let typed = child(h, WC_EDITW, "", WS_VISIBLE | WS_BORDER | WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL) as u32), ID_TYPED, mono_font(owner));
    let _ = MoveWindow(typed, s(16), s(62), s(510), s(170), true);
    let say = child(h, WC_STATICW, "0 of 24 words", WS_VISIBLE, ID_SAY, font);
    let _ = MoveWindow(say, s(16), s(240), s(510), s(40), true);
    let ok = child(h, WC_BUTTONW, "Recover", WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32), ID_RECOVER, font);
    let cancel = child(h, WC_BUTTONW, "Cancel", WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), ID_CANCEL, font);
    let _ = MoveWindow(ok, s(318), s(304), s(100), s(30), true);
    let _ = MoveWindow(cancel, s(426), s(304), s(100), s(30), true);
    let _ = EnableWindow(ok, false);
    let _ = EnableWindow(owner, false);
    let _ = SetFocus(typed);
}

unsafe extern "system" fn recover_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as i32;
            let code = (wparam.0 >> 16) as u32;
            match id {
                ID_TYPED if code == EN_CHANGE => {
                    let typed = text_of(GetDlgItem(hwnd, ID_TYPED).unwrap_or_default());
                    let (say, ok) = match check_recovery_words(&typed) {
                        Ok(_) => ("The words check out.".to_string(), true),
                        Err(e) => (e, false),
                    };
                    set_text(GetDlgItem(hwnd, ID_SAY).unwrap_or_default(), &say);
                    let _ = EnableWindow(GetDlgItem(hwnd, ID_RECOVER).unwrap_or_default(), ok);
                }
                ID_RECOVER => {
                    let typed = text_of(GetDlgItem(hwnd, ID_TYPED).unwrap_or_default());
                    let Ok(words) = check_recovery_words(&typed) else { return LRESULT(0) };
                    close(hwnd);
                    // It takes minutes: the note line says what's going on.
                    note("Recovering: looking for a device of yours that keeps your keyring, then bringing each library back (several minutes)…");
                    std::thread::spawn(move || {
                        *RECOVERED.lock().unwrap() = Some(Daemon::new().recovery_restore(&words));
                        post(WM_RECOVERED);
                    });
                }
                ID_CANCEL => close(hwnd),
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

/// The main window got the recovery's outcome.
pub unsafe fn recovered(owner: HWND) {
    let Some(result) = RECOVERED.lock().unwrap().take() else { return };
    note("");
    match result {
        Ok(libs) if libs.is_empty() => {
            MessageBoxW(owner, w!("The keyring lists no library this device doesn't have."), w!("Nothing to bring back"), MB_OK | MB_ICONINFORMATION);
        }
        Ok(libs) => {
            let text = format!(
                "Back on this device: {}.\n\nTheir content arrives in the background. In each library, remove the devices you lost (Members…).",
                libs.join(", ")
            );
            MessageBoxW(owner, &wide(&text), w!("Your libraries are back"), MB_OK | MB_ICONINFORMATION);
        }
        Err(e) => {
            MessageBoxW(owner, &wide(&e), w!("Not recovered"), MB_OK | MB_ICONWARNING);
        }
    }
    crate::app::poke();
}
