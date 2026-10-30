//! This device: Reticulum and its connections, the services, and the node's
//! settings. What a tester needs to get a fresh install talking to others.

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use adw::prelude::*;
use gtk::glib;
use retifs_desktop_core::{
    device::{self, Configured, Reticulum},
    human, Daemon,
};

use crate::bg;

pub struct DevicePage {
    pub page: adw::PreferencesPage,
    daemon: Arc<Daemon>,
    shared: adw::ActionRow,
    connections: adw::PreferencesGroup,
    connection_rows: RefCell<Vec<gtk::Widget>>,
    services: Vec<(&'static str, &'static str, adw::ActionRow)>,
    settings: Vec<(&'static str, &'static str, gtk::Widget)>,
    save: gtk::Button,
    /// What the file said when last read: only changes are written.
    loaded: RefCell<HashMap<(String, String), Option<String>>>,
}

impl DevicePage {
    pub fn new(daemon: Arc<Daemon>) -> Rc<DevicePage> {
        let page = adw::PreferencesPage::new();

        let reticulum = adw::PreferencesGroup::builder()
            .title("Reticulum")
            .description("The network retifs runs on. Out of the box it finds others on the local network; add a connection to reach further.")
            .build();
        let shared = adw::ActionRow::builder().title("Shared instance").subtitle("…").subtitle_selectable(true).build();
        reticulum.add(&shared);
        page.add(&reticulum);

        let add = gtk::Button::builder().label("Add connection…").valign(gtk::Align::Center).build();
        let connections = adw::PreferencesGroup::builder().title("Connections").header_suffix(&add).build();
        page.add(&connections);

        let svc_group = adw::PreferencesGroup::builder().title("Services").description("systemd user services: they start with your session.").build();
        let mut services = Vec::new();
        for &(unit, what) in device::SERVICES {
            let row = adw::ActionRow::builder().title(what).subtitle(unit).build();
            svc_group.add(&row);
            services.push((unit, what, row));
        }
        // Only where there's systemd (the Linux package).
        svc_group.set_visible(device::service_state(device::RETIFSD).is_some());
        page.add(&svc_group);

        let save = gtk::Button::builder().label("Save and restart").valign(gtk::Align::Center).css_classes(["suggested-action"]).sensitive(false).build();
        let set_group = adw::PreferencesGroup::builder()
            .title("Settings")
            .description("This node's retifs.toml. Empty means the default.")
            .header_suffix(&save)
            .build();
        let mut settings = Vec::new();
        for (section, key, what) in device::SETTINGS {
            let w: gtk::Widget = if key == "no_storage" {
                adw::SwitchRow::builder().title(what).build().upcast()
            } else {
                adw::EntryRow::builder().title(what).build().upcast()
            };
            set_group.add(&w);
            settings.push((section, key, w));
        }
        page.add(&set_group);

        let me = Rc::new(DevicePage {
            page,
            daemon,
            shared,
            connections,
            connection_rows: RefCell::new(Vec::new()),
            services,
            settings,
            save: save.clone(),
            loaded: RefCell::new(HashMap::new()),
        });
        for (unit, _, row) in &me.services {
            for (icon, action, tip) in [("media-playback-start-symbolic", "start", "Start"), ("view-refresh-symbolic", "restart", "Restart"), ("media-playback-stop-symbolic", "stop", "Stop")] {
                let b = gtk::Button::builder().icon_name(icon).tooltip_text(tip).valign(gtk::Align::Center).css_classes(["flat"]).build();
                let (me2, unit) = (me.clone(), *unit);
                b.connect_clicked(move |_| {
                    let me3 = me2.clone();
                    bg(&me2.daemon, move |_| device::service(unit, action), move |r| {
                        if let Err(e) = r {
                            me3.shared.set_subtitle(&format!("{unit}: {e}"));
                        }
                        me3.refresh();
                    });
                });
                row.add_suffix(&b);
            }
        }
        for (_, _, w) in &me.settings {
            let me2 = me.clone();
            if let Some(e) = w.downcast_ref::<adw::EntryRow>() {
                e.connect_changed(move |_| me2.save.set_sensitive(true));
            } else if let Some(s) = w.downcast_ref::<adw::SwitchRow>() {
                s.connect_active_notify(move |_| me2.save.set_sensitive(true));
            }
        }
        {
            let me2 = me.clone();
            save.connect_clicked(move |_| me2.save_settings());
        }
        {
            let me2 = me.clone();
            add.connect_clicked(move |b| me2.ask_add(b.root().and_downcast()));
        }
        me.load_settings();
        me
    }

    /// Reticulum and the services, asked again.
    pub fn refresh(self: &Rc<Self>) {
        let me = self.clone();
        bg(&self.daemon, |_| (device::reticulum(), device::configured(), device::SERVICES.iter().map(|(u, _)| device::service_state(u)).collect::<Vec<_>>()), move |(r, conf, states)| {
            me.show_reticulum(&r, conf.unwrap_or_default());
            for ((_, what, row), state) in me.services.iter().zip(states) {
                row.set_title(&format!("{what} — {}", state.unwrap_or_else(|| "?".into())));
            }
        });
    }

    fn show_reticulum(self: &Rc<Self>, r: &Reticulum, configured: Vec<Configured>) {
        let bin = r.prnsd.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "not found".into());
        if r.running {
            let up = r.uptime_s.map(|s| if s > 86_400.0 { format!("{:.0} d", s / 86_400.0) } else { format!("{:.0} h", s / 3600.0) }).unwrap_or_default();
            self.shared.set_title("Running");
            self.shared.set_subtitle(&format!("transport {} · up {up} · {bin}", r.transport_id.clone().unwrap_or_default()));
        } else {
            self.shared.set_title("Not running");
            self.shared.set_subtitle(&format!("{} · {bin}", r.error.clone().unwrap_or_default()));
        }
        for w in self.connection_rows.borrow_mut().drain(..) {
            self.connections.remove(&w);
        }
        let mut rows = Vec::new();
        for c in &configured {
            let live = r.interfaces.iter().find(|i| i.name == c.name);
            let state = match live {
                Some(i) if i.up => format!("connected · ↓ {} ↑ {}", human(i.rx), human(i.tx)),
                Some(_) => "not connected".to_string(),
                None if c.enabled => "enabled".to_string(),
                None => "off".to_string(),
            };
            let row = adw::ActionRow::builder().title(glib::markup_escape_text(&c.name)).subtitle(format!("{} · {state}", c.kind.trim_end_matches("Interface"))).build();
            let on = gtk::Switch::builder().active(c.enabled).valign(gtk::Align::Center).tooltip_text("Use this connection").build();
            let (me, name) = (self.clone(), c.name.clone());
            on.connect_state_set(move |_, enabled| {
                let (me2, name) = (me.clone(), name.clone());
                bg(&me.daemon, move |_| device::set_interface(&name, enabled), move |r| {
                    if let Err(e) = r {
                        me2.shared.set_subtitle(&e);
                    }
                    me2.refresh();
                });
                glib::Propagation::Proceed
            });
            row.add_suffix(&on);
            let remove = gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text("Remove this connection").valign(gtk::Align::Center).css_classes(["flat"]).build();
            let (me, name) = (self.clone(), c.name.clone());
            remove.connect_clicked(move |b| me.ask_remove(&name, b.root().and_downcast()));
            row.add_suffix(&remove);
            self.connections.add(&row);
            rows.push(row.upcast());
        }
        if configured.is_empty() {
            let row = adw::ActionRow::builder().title("None yet").subtitle("Only the local network is reached. Add a connection to a Reticulum node you trust.").build();
            self.connections.add(&row);
            rows.push(row.upcast());
        }
        *self.connection_rows.borrow_mut() = rows;
    }

    fn ask_add(self: &Rc<Self>, window: Option<gtk::Window>) {
        let name = adw::EntryRow::builder().title("Name").build();
        let host = adw::EntryRow::builder().title("Host (name or address)").build();
        let port = adw::EntryRow::builder().title("Port").text("4242").build();
        let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
        for r in [&name, &host, &port] {
            list.append(r);
        }
        let d = adw::MessageDialog::builder()
            .heading("Add a connection")
            .body("A TCP connection to another Reticulum node: someone's you trust, or a public one. Everything retifs sends is encrypted end to end either way.")
            .extra_child(&list)
            .build();
        d.add_responses(&[("cancel", "Cancel"), ("add", "Add")]);
        d.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        let me = self.clone();
        d.connect_response(None, move |_, r| {
            if r != "add" {
                return;
            }
            let (n, h) = (name.text().trim().to_string(), host.text().trim().to_string());
            let Ok(p) = port.text().trim().parse::<u16>() else {
                me.shared.set_subtitle("The port is a number (4242 by default)");
                return;
            };
            if h.is_empty() {
                return;
            }
            let n = if n.is_empty() { h.clone() } else { n };
            let me2 = me.clone();
            bg(&me.daemon, move |_| device::add_tcp(&n, &h, p), move |r| {
                if let Err(e) = r {
                    me2.shared.set_subtitle(&e);
                }
                me2.refresh();
            });
        });
        d.set_transient_for(window.as_ref());
        d.present();
    }

    fn ask_remove(self: &Rc<Self>, name: &str, window: Option<gtk::Window>) {
        let d = adw::MessageDialog::builder().heading(format!("Remove {name}?")).body("The connection is taken out of the Reticulum configuration.").build();
        d.add_responses(&[("cancel", "Cancel"), ("remove", "Remove")]);
        d.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        let (me, name) = (self.clone(), name.to_string());
        d.connect_response(None, move |_, r| {
            if r != "remove" {
                return;
            }
            let (me2, name) = (me.clone(), name.clone());
            bg(&me.daemon, move |_| device::remove_interface(&name), move |r| {
                if let Err(e) = r {
                    me2.shared.set_subtitle(&e);
                }
                me2.refresh();
            });
        });
        d.set_transient_for(window.as_ref());
        d.present();
    }

    fn load_settings(&self) {
        let text = std::fs::read_to_string(device::settings_file(self.daemon.store())).unwrap_or_default();
        let mut loaded = self.loaded.borrow_mut();
        for (section, key, w) in &self.settings {
            let v = device::get_setting(&text, section, key);
            loaded.insert((section.to_string(), key.to_string()), v.clone());
            if let Some(e) = w.downcast_ref::<adw::EntryRow>() {
                e.set_text(&v.as_deref().map(device::shown_value).unwrap_or_default());
            } else if let Some(s) = w.downcast_ref::<adw::SwitchRow>() {
                s.set_active(v.as_deref() == Some("true"));
            }
        }
        self.save.set_sensitive(false);
    }

    /// Writes the settings that changed and restarts the daemon to use them.
    fn save_settings(self: &Rc<Self>) {
        let path = device::settings_file(self.daemon.store());
        let mut text = std::fs::read_to_string(&path).unwrap_or_default();
        let loaded = self.loaded.borrow().clone();
        for (section, key, w) in &self.settings {
            let now: Option<String> = if let Some(e) = w.downcast_ref::<adw::EntryRow>() {
                let t = e.text().trim().to_string();
                (!t.is_empty()).then(|| device::toml_value(&t))
            } else if let Some(s) = w.downcast_ref::<adw::SwitchRow>() {
                s.is_active().then(|| "true".to_string())
            } else {
                None
            };
            if loaded.get(&(section.to_string(), key.to_string())).cloned().flatten() != now {
                text = device::set_setting(&text, section, key, now.as_deref());
            }
        }
        if let Err(e) = std::fs::write(&path, text) {
            self.shared.set_subtitle(&format!("{}: {e}", path.display()));
            return;
        }
        self.load_settings();
        let me = self.clone();
        bg(&self.daemon, |_| device::service(device::RETIFSD, "restart"), move |r| {
            me.shared.set_subtitle(&match r {
                Ok(()) => "Settings saved; retifs restarted".to_string(),
                Err(_) => "Settings saved; restart retifsd to use them".to_string(),
            });
            me.refresh();
        });
    }
}
