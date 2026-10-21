//! retifs-desktop: a tray icon and a window over the local retifsd, through
//! libretifs. It keeps no data of its own (docs/plan.md, milestone 1).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Duration;

use libretifs::{Client, NsView, Request, Response, StoreUsage};
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WindowEvent,
};

/// What the tray and the overview show.
#[derive(Serialize, Clone, Default)]
struct Status {
    up: bool,
    store: String,
    node: Option<String>,
    chunks: u64,
    usage: Option<StoreUsage>,
    error: Option<String>,
}

async fn client() -> Result<Client, String> {
    Client::connect(&libretifs::default_store_dir()).await.map_err(|e| e.to_string())
}

async fn read_status() -> Status {
    let store = libretifs::default_store_dir().display().to_string();
    let mut c = match client().await {
        Ok(c) => c,
        Err(e) => return Status { store, error: Some(e), ..Default::default() },
    };
    match c.ping().await {
        Ok((node, chunks, usage)) => Status { up: true, store, node: Some(node), chunks, usage, error: None },
        Err(e) => Status { store, error: Some(e.to_string()), ..Default::default() },
    }
}

#[tauri::command]
async fn status() -> Status {
    read_status().await
}

/// Every namespace with its view (writers, peers, keep mode, checkpoint).
#[tauri::command]
async fn namespaces() -> Result<Vec<NsView>, String> {
    let mut c = client().await?;
    let names = match c.call(Request::Namespaces).await.map_err(|e| e.to_string())? {
        Response::Names { names } => names,
        _ => Vec::new(),
    };
    let mut out = Vec::new();
    for ns in names {
        // Followed channels' namespaces have no member view: skipped here.
        if let Ok(Response::Namespace(v)) = c.call(Request::NsInfo { ns }).await {
            out.push(v);
        }
    }
    Ok(out)
}

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn tooltip(s: &Status) -> String {
    match (&s.node, &s.usage) {
        (Some(node), Some(u)) => format!("retifs: running, node {}…\nown data {}", &node[..8.min(node.len())], human(u.pinned)),
        (Some(node), None) => format!("retifs: running, node {}…", &node[..8.min(node.len())]),
        _ => "retifs: daemon not running".to_string(),
    }
}

fn human(n: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", units[i])
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![status, namespaces])
        .setup(|app| {
            let open = MenuItem::with_id(app, "open", "Open retifs", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().expect("an app icon").clone())
                .tooltip("retifs")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show_window(tray.app_handle());
                    }
                })
                .build(app)?;

            // The tray and the window follow the daemon: polled every 5 s.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let s = read_status().await;
                    if let Some(tray) = handle.tray_by_id("main") {
                        let _ = tray.set_tooltip(Some(tooltip(&s)));
                    }
                    let _ = handle.emit("status", &s);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });
            Ok(())
        })
        // Closing the window keeps the app in the tray, like a sync client.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("retifs-desktop");
}
