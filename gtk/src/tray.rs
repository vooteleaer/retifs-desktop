//! The tray icon (StatusNotifierItem: KDE natively, GNOME with the
//! AppIndicator extension): whether retifs and Reticulum are up at a glance,
//! and a menu to open the window, restart retifs or quit. It runs on a
//! thread of its own with a small runtime (ksni is async); the window's
//! status poll sends it what to show, and its menu sends commands back.

use std::sync::mpsc;

use ksni::menu::StandardItem;
use ksni::TrayMethods;

pub use retifs_desktop_core::status_icon::TrayState;
use retifs_desktop_core::status_icon::{self, Health};

/// What the menu asks of the app.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrayCommand {
    Open,
    RestartRetifs,
    Quit,
}

struct Icon {
    state: TrayState,
    commands: async_channel::Sender<TrayCommand>,
}

impl Icon {
    fn send(&self, c: TrayCommand) {
        let _ = self.commands.send_blocking(c);
    }
}

impl ksni::Tray for Icon {
    fn id(&self) -> String {
        "org.retifs.Desktop".into()
    }

    fn title(&self) -> String {
        "retifs".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [22, 32, 64].into_iter().map(|s| draw(s, self.state.health())).collect()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: "retifs".into(), description: self.state.lines().join("\n"), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::Open);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let mut items: Vec<ksni::MenuItem<Self>> =
            self.state.lines().into_iter().map(|label| StandardItem { label, enabled: false, ..Default::default() }.into()).collect();
        items.push(ksni::MenuItem::Separator);
        items.push(StandardItem { label: "Open retifs".into(), activate: Box::new(|t: &mut Self| t.send(TrayCommand::Open)), ..Default::default() }.into());
        items.push(StandardItem { label: "Restart retifs".into(), activate: Box::new(|t: &mut Self| t.send(TrayCommand::RestartRetifs)), ..Default::default() }.into());
        items.push(ksni::MenuItem::Separator);
        items.push(StandardItem { label: "Quit".into(), activate: Box::new(|t: &mut Self| t.send(TrayCommand::Quit)), ..Default::default() }.into());
        items
    }
}

/// The icon at `size` pixels, ARGB32 big-endian as StatusNotifierItem wants it.
fn draw(size: i32, health: Health) -> ksni::Icon {
    let data = status_icon::draw(size as u32, health).iter().flat_map(|p| p.to_be_bytes()).collect();
    ksni::Icon { width: size, height: size, data }
}

/// A running tray: send it states to show.
pub struct Tray {
    states: tokio::sync::mpsc::UnboundedSender<TrayState>,
    /// A tray host (StatusNotifierWatcher) was there at start: the icon is
    /// seen. Without one the icon waits for a host, unseen.
    pub host: bool,
}

impl Tray {
    pub fn show(&self, state: TrayState) {
        let _ = self.states.send(state);
    }
}

/// Starts the tray; Err when there's no tray host (a desktop without
/// StatusNotifierItem support), and then the window is all there is.
pub fn start(commands: async_channel::Sender<TrayCommand>) -> Result<Tray, String> {
    let (states, mut incoming) = tokio::sync::mpsc::unbounded_channel::<TrayState>();
    let (ready, started) = mpsc::channel::<Result<bool, String>>();
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready.send(Err(e.to_string()));
                    return;
                }
            };
            rt.block_on(async move {
                let host = has_host().await;
                let handle = match (Icon { state: TrayState::default(), commands }).spawn().await {
                    Ok(h) => {
                        let _ = ready.send(Ok(host));
                        h
                    }
                    Err(e) => {
                        let _ = ready.send(Err(e.to_string()));
                        return;
                    }
                };
                while let Some(state) = incoming.recv().await {
                    if handle.update(|icon: &mut Icon| icon.state = state).await.is_none() {
                        break;
                    }
                }
            });
        })
        .map_err(|e| e.to_string())?;
    let host = started.recv().map_err(|e| e.to_string())??;
    Ok(Tray { states, host })
}

/// Whether the session has a tray host (KDE, or GNOME's AppIndicator
/// extension, own the StatusNotifierWatcher name).
async fn has_host() -> bool {
    let Ok(conn) = zbus::Connection::session().await else { return false };
    let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else { return false };
    let Ok(name) = zbus::names::BusName::try_from("org.kde.StatusNotifierWatcher") else { return false };
    dbus.name_has_owner(name).await.unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_bytes() {
        let icon = draw(32, Health::Good);
        assert_eq!(icon.data.len(), 32 * 32 * 4);
        assert!(icon.data.chunks(4).any(|p| p == [0xff, 0x5f, 0xb0, 0x6e]), "ARGB, alpha first");
    }
}
