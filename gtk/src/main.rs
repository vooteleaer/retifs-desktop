//! retifs-desktop for Linux: native GTK 4 / libadwaita widgets over the
//! shared core. A sidebar (overview, Find, saved searches, libraries), the
//! overview, Find (a live query, as BeOS's Find), a namespace's management
//! page (sharing, members, what this device keeps, state), and a banner when
//! the daemon isn't running. It finds, but doesn't browse or catalogue
//! content: that's what apps such as a photo album are for.

mod device;
mod find;
mod library;
#[cfg(target_os = "linux")]
mod tray;

use std::{cell::RefCell, rc::Rc, sync::Arc};

use adw::prelude::*;
use gtk::{gio, glib};
use retifs_desktop_core::{human, Daemon, NsView, SavedQuery, Status};

const APP_ID: &str = "org.retifs.Desktop";

thread_local! {
    /// The window's state, once built: a link opened later finds it here.
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

fn main() -> glib::ExitCode {
    // --hidden: started at login, into the tray only.
    let hidden = std::env::args().any(|a| a == "--hidden");
    // A retifs: link (the desktop entry handles x-scheme-handler/retifs):
    // opened in the running instance, if there is one.
    let link = std::env::args().find(|a| a.starts_with("retifs:"));
    let app = adw::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_OPEN).build();
    // The Flatpak asks to start at login, into the tray (the .deb has an
    // autostart entry instead).
    app.connect_startup(|app| {
        if retifs_desktop_core::device::in_flatpak() {
            ask_autostart(app);
        }
    });
    let first = std::cell::Cell::new(true);
    app.connect_activate(move |app| {
        // One window: started again, it comes to the front.
        if let Some(w) = app.windows().first() {
            w.present();
        } else {
            build(app, hidden && first.get());
        }
        first.set(false);
    });
    // The join dialog with the link in it: nothing is joined until the user says so.
    app.connect_open(|app, files, _| {
        if app.windows().is_empty() {
            build(app, false);
        }
        let Some(me) = APP.with(|a| a.borrow().clone()) else { return };
        me.window.present();
        for f in files {
            let uri = f.uri().to_string();
            if uri.starts_with("retifs:") {
                ask_add_namespace(&me, Some(me.window.clone().upcast()), true, &uri);
            }
        }
    });
    let mut args = vec![std::env::args().next().unwrap_or_default()];
    args.extend(link);
    app.run_with_args(&args)
}

/// Asks the Background portal to start the app at login, `--hidden` (the
/// desktop may ask the user once). Without a portal: nothing.
fn ask_autostart(app: &adw::Application) {
    let Some(bus) = app.dbus_connection() else { return };
    let options = glib::VariantDict::new(None);
    options.insert_value("reason", &"Shows retifs in the tray".to_variant());
    options.insert_value("autostart", &true.to_variant());
    options.insert_value("commandline", &vec!["retifs-desktop", "--hidden"].to_variant());
    options.insert_value("dbus-activatable", &false.to_variant());
    let args = glib::Variant::tuple_from_iter(["".to_variant(), options.end()]);
    bus.call(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Background",
        "RequestBackground",
        Some(&args),
        None,
        gio::DBusCallFlags::NONE,
        -1,
        None::<&gio::Cancellable>,
        |r| {
            if let Err(e) = r {
                eprintln!("no autostart (Background portal): {e}");
            }
        },
    );
}

/// Runs `work` on a worker thread (daemon calls block), then `then` with its
/// result on the UI thread.
fn bg<T: Send + 'static>(d: &Arc<Daemon>, work: impl FnOnce(&Daemon) -> T + Send + 'static, then: impl FnOnce(T) + 'static) {
    let d = d.clone();
    glib::spawn_future_local(async move {
        if let Ok(r) = gio::spawn_blocking(move || work(&d)).await {
            then(r);
        }
    });
}

/// The overview's widgets, updated on each status poll.
struct Overview {
    node: adw::ActionRow,
    storage: Vec<(adw::ActionRow, gtk::LevelBar)>,
    namespaces: adw::PreferencesGroup,
    ns_rows: RefCell<Vec<adw::ActionRow>>,
}

struct App {
    daemon: Arc<Daemon>,
    title: adw::WindowTitle,
    stack: gtk::Stack,
    overview: Overview,
    ns_page: adw::Bin,
    side_top: gtk::ListBox,
    side_saved: gtk::ListBox,
    side_ns: gtk::ListBox,
    find: Rc<find::Find>,
    device: Rc<device::DevicePage>,
    retifs_up: std::cell::Cell<bool>,
    node: RefCell<Option<String>>,
    saved: RefCell<Vec<(SavedQuery, Vec<String>, usize)>>,
    views: RefCell<Vec<NsView>>,
    /// `Daemon::libraries`: access, public, favourite, by library.
    libs: RefCell<Vec<serde_json::Value>>,
    shown: RefCell<Option<String>>,
    window: adw::ApplicationWindow,
    announces: RefCell<Option<adw::PreferencesGroup>>,
    announce_rows: RefCell<Vec<adw::ActionRow>>,
}

impl App {
    fn window(&self) -> adw::ApplicationWindow {
        self.window.clone()
    }
}

fn build(app: &adw::Application, hidden: bool) {
    let title = adw::WindowTitle::new("Overview", "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));
    let banner = adw::Banner::new("retifsd is not running");
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);

    let overview = overview_widgets();
    let ov_page = adw::PreferencesPage::new();
    let group_node = adw::PreferencesGroup::builder().title("This node").build();
    group_node.add(&overview.node);
    let group_storage = adw::PreferencesGroup::builder().title("Storage").build();
    for (row, _) in &overview.storage {
        group_storage.add(row);
    }
    ov_page.add(&group_node);
    ov_page.add(&group_storage);
    ov_page.add(&overview.namespaces);
    stack.add_named(&ov_page, Some("overview"));
    let ns_page = adw::Bin::new();
    stack.add_named(&ns_page, Some("namespace"));
    let daemon = Arc::new(Daemon::new());
    let finder = find::Find::new(daemon.clone());
    stack.add_named(&finder.page, Some("find"));
    let device_page = device::DevicePage::new(daemon.clone());
    stack.add_named(&device_page.page, Some("device"));

    let content = adw::ToolbarView::new();
    content.add_top_bar(&header);
    content.add_top_bar(&banner);
    content.set_content(Some(&stack));
    let content_page = adw::NavigationPage::builder().title("retifs").child(&content).build();

    let side_top = gtk::ListBox::new();
    side_top.add_css_class("navigation-sidebar");
    side_top.append(&label("Overview"));
    side_top.append(&label("Find"));
    side_top.append(&label("This device"));
    side_top.append(&label("Announces"));
    let saved_head = gtk::Label::builder().label("Saved searches").xalign(0.0).margin_start(18).margin_top(12).margin_bottom(4).build();
    saved_head.add_css_class("dim-label");
    saved_head.add_css_class("caption-heading");
    let side_saved = gtk::ListBox::new();
    side_saved.add_css_class("navigation-sidebar");
    // The interface's words (docs/plan.md): every namespace is a library;
    // one shared with others is marked: they're its club.
    let ns_head = gtk::Label::builder().label("Libraries").xalign(0.0).hexpand(true).build();
    ns_head.add_css_class("dim-label");
    ns_head.add_css_class("caption-heading");
    let new_library = gtk::Button::builder().label("New library…").build();
    let join_library = gtk::Button::builder().label("Join library…").build();
    let add_menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
    for b in [&new_library, &join_library] {
        b.add_css_class("flat");
        b.child().and_downcast::<gtk::Label>().map(|l| l.set_xalign(0.0));
        add_menu.append(b);
    }
    let add_popover = gtk::Popover::builder().child(&add_menu).build();
    let add_ns = gtk::MenuButton::builder().icon_name("list-add-symbolic").popover(&add_popover).tooltip_text("New library, or join one").build();
    add_ns.add_css_class("flat");
    let side_head = gtk::Box::builder().margin_start(18).margin_end(6).margin_top(6).build();
    side_head.append(&ns_head);
    side_head.append(&add_ns);
    let side_ns = gtk::ListBox::new();
    side_ns.add_css_class("navigation-sidebar");
    let side_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    side_box.append(&side_top);
    side_box.append(&saved_head);
    side_box.append(&side_saved);
    side_box.append(&side_head);
    side_box.append(&side_ns);
    let side_view = adw::ToolbarView::new();
    side_view.add_top_bar(&adw::HeaderBar::new());
    side_view.set_content(Some(&gtk::ScrolledWindow::builder().child(&side_box).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build()));
    let side_page = adw::NavigationPage::builder().title("retifs").child(&side_view).build();

    let split = adw::NavigationSplitView::builder().sidebar(&side_page).content(&content_page).build();
    let window = adw::ApplicationWindow::builder().application(app).title("retifs").default_width(960).default_height(660).content(&split).build();

    let me = Rc::new(App {
        daemon,
        title,
        stack,
        overview,
        ns_page,
        side_top: side_top.clone(),
        side_saved: side_saved.clone(),
        side_ns: side_ns.clone(),
        find: finder.clone(),
        device: device_page.clone(),
        retifs_up: std::cell::Cell::new(false),
        node: RefCell::new(None),
        saved: RefCell::new(Vec::new()),
        views: RefCell::new(Vec::new()),
        libs: RefCell::new(Vec::new()),
        shown: RefCell::new(None),
        window: window.clone(),
        announces: RefCell::new(None),
        announce_rows: RefCell::new(Vec::new()),
    });
    APP.with(|a| *a.borrow_mut() = Some(me.clone()));
    let announces = library::announces_page(&me);
    me.stack.add_named(&announces, Some("announces"));
    library::add_actions(&me, &window);

    {
        let me = me.clone();
        side_top.connect_row_activated(move |_, row| {
            me.side_ns.unselect_all();
            me.side_saved.unselect_all();
            *me.shown.borrow_mut() = None;
            if row.index() == 1 {
                me.title.set_title("Find");
                me.stack.set_visible_child_name("find");
            } else if row.index() == 2 {
                me.title.set_title("This device");
                me.stack.set_visible_child_name("device");
                me.device.refresh();
            } else if row.index() == 3 {
                me.title.set_title("Announces");
                me.stack.set_visible_child_name("announces");
                library::fill_announces(&me);
            } else {
                me.title.set_title("Overview");
                me.stack.set_visible_child_name("overview");
            }
            reload(&me);
        });
    }
    {
        let me = me.clone();
        side_saved.connect_row_activated(move |_, row| {
            let i = row.index();
            me.side_top.unselect_all();
            me.side_ns.unselect_all();
            *me.shown.borrow_mut() = None;
            let saved = me.saved.borrow();
            let Some((q, here, missing)) = saved.get(i as usize) else { return };
            me.title.set_title(&q.name);
            me.stack.set_visible_child_name("find");
            me.find.open_saved(&q.terms(), here.clone(), *missing);
        });
    }
    {
        let me2 = me.clone();
        *finder.on_saved.borrow_mut() = Some(Box::new(move || reload(&me2)));
    }
    {
        let me = me.clone();
        let (me2, pop) = (me.clone(), add_popover.clone());
        new_library.connect_clicked(move |b| {
            pop.popdown();
            ask_add_namespace(&me2, b.root().and_downcast(), false, "");
        });
        let pop = add_popover.clone();
        join_library.connect_clicked(move |b| {
            pop.popdown();
            ask_add_namespace(&me, b.root().and_downcast(), true, "");
        });
    }
    {
        let me = me.clone();
        side_ns.connect_row_activated(move |_, row| {
            if let Some(name) = row.widget_name().as_str().strip_prefix("ns:") {
                open_ns(&me, name);
            }
        });
    }

    // The daemon's state every 5 s; namespaces again when it comes up, and
    // every 30 s while it's up.
    let poll = {
        let (me, banner, was_up, ticks) = (me.clone(), banner.clone(), Rc::new(RefCell::new(false)), Rc::new(RefCell::new(0u32)));
        move || {
            let (me2, banner, was_up, ticks) = (me.clone(), banner.clone(), was_up.clone(), ticks.clone());
            // The Flatpak: retifs is the host's; without it, the banner says so.
            bg(&me.daemon, |d| {
                let s = d.status();
                let missing = !s.up && retifs_desktop_core::device::retifs_missing();
                (s, missing)
            }, move |(s, missing): (Status, bool)| {
                me2.retifs_up.set(s.up);
                *me2.node.borrow_mut() = s.node.clone();
                banner.set_title(if missing { retifs_desktop_core::device::NOT_INSTALLED } else { "retifsd is not running" });
                banner.set_revealed(!s.up);
                me2.title.set_subtitle(&match &s.node {
                    Some(n) => format!("node {}…", &n[..8.min(n.len())]),
                    None => "not connected".into(),
                });
                show_status(&me2.overview, &s);
                let mut t = ticks.borrow_mut();
                if s.up && (!*was_up.borrow() || *t % 6 == 0) {
                    reload(&me2);
                }
                *t += 1;
                *was_up.borrow_mut() = s.up;
            });
        }
    };
    poll();
    glib::timeout_add_seconds_local(5, move || {
        poll();
        glib::ControlFlow::Continue
    });

    // RETIFS_DESKTOP_PAGE=device: opens on This device (demos, screenshots).
    let start = if std::env::var("RETIFS_DESKTOP_PAGE").as_deref() == Ok("device") { 2 } else { 0 };
    side_top.select_row(side_top.row_at_index(start).as_ref());
    if let Some(row) = side_top.row_at_index(start) {
        row.activate();
    }
    let in_tray = start_tray(app, &window, &me);
    if !(hidden && in_tray) {
        window.present();
    }

    // RETIFS_DESKTOP_SNAPSHOT=<file.png>: the window drawn to a file every
    // 2 s (demos and screenshots without a screen grab).
    if let Ok(path) = std::env::var("RETIFS_DESKTOP_SNAPSHOT") {
        let window = window.clone();
        glib::timeout_add_seconds_local(2, move || {
            let paintable = gtk::WidgetPaintable::new(Some(&window));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
            if let (Some(node), Some(renderer)) = (snapshot.to_node(), window.renderer()) {
                let _ = renderer.render_texture(&node, None).save_to_png(&path);
            }
            glib::ControlFlow::Continue
        });
    }
}

/// The tray icon, where the desktop has a tray: closing the window then
/// leaves the app running there. Returns whether it's there.
#[cfg(target_os = "linux")]
fn start_tray(app: &adw::Application, window: &adw::ApplicationWindow, me: &Rc<App>) -> bool {
    let (commands, received) = async_channel::unbounded();
    let tray = match tray::start(commands) {
        Ok(t) => Rc::new(t),
        Err(e) => {
            eprintln!("no tray: {e}");
            return false;
        }
    };
    // Kept running with the window closed, where the icon is seen.
    let host = tray.host;
    if host {
        std::mem::forget(app.hold());
        window.set_hide_on_close(true);
    } else {
        eprintln!("no tray host on this desktop (GNOME: the AppIndicator extension): the window only");
    }
    {
        let (app, window, me) = (app.clone(), window.clone(), me.clone());
        glib::spawn_future_local(async move {
            while let Ok(c) = received.recv().await {
                match c {
                    tray::TrayCommand::Open => window.present(),
                    tray::TrayCommand::RestartRetifs => {
                        bg(&me.daemon, |_| retifs_desktop_core::device::service(retifs_desktop_core::device::RETIFSD, "restart"), |_| {});
                    }
                    tray::TrayCommand::Quit => app.quit(),
                }
            }
        });
    }
    // Reticulum every 15 s (prnsd is asked as a program), retifs from the
    // window's own 5 s poll.
    let me = me.clone();
    let update = move || {
        let (me2, tray) = (me.clone(), tray.clone());
        bg(&me.daemon, |_| retifs_desktop_core::device::reticulum(), move |r| {
            let outward: Vec<_> = r.interfaces.iter().filter(|i| !i.is_local()).collect();
            tray.show(tray::TrayState {
                retifs_up: me2.retifs_up.get(),
                node: me2.node.borrow().clone(),
                reticulum_up: r.running,
                links_up: outward.iter().filter(|i| i.up).count(),
                links: outward.len(),
            });
        });
    };
    let first = update.clone();
    glib::timeout_add_seconds_local_once(2, first);
    glib::timeout_add_seconds_local(15, move || {
        update();
        glib::ControlFlow::Continue
    });
    host
}

#[cfg(not(target_os = "linux"))]
fn start_tray(_: &adw::Application, _: &adw::ApplicationWindow, _: &Rc<App>) -> bool {
    false
}

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build()
}

fn overview_widgets() -> Overview {
    let node = adw::ActionRow::builder().title("Node").subtitle("—").subtitle_selectable(true).build();
    let storage = ["Own data", "Held for others", "Cache", "Disk"]
        .into_iter()
        .map(|t| {
            let row = adw::ActionRow::builder().title(t).subtitle("—").build();
            let bar = gtk::LevelBar::for_interval(0.0, 1.0);
            bar.set_valign(gtk::Align::Center);
            bar.set_width_request(220);
            row.add_suffix(&bar);
            (row, bar)
        })
        .collect();
    let namespaces = adw::PreferencesGroup::builder().title("Libraries").build();
    Overview { node, storage, namespaces, ns_rows: RefCell::new(Vec::new()) }
}

fn show_status(o: &Overview, s: &Status) {
    match &s.node {
        Some(n) => {
            o.node.set_title(n);
            o.node.set_subtitle(&format!("{} chunks · store {}", s.chunks, s.store));
        }
        None => {
            o.node.set_title("Not connected");
            o.node.set_subtitle(&format!("{} ({})", s.error.clone().unwrap_or_default(), s.store));
        }
    }
    let Some(u) = &s.usage else { return };
    let values = [
        (u.pinned, u.store_limit, None),
        (u.leased, u.store_limit, None),
        (u.cache, u.cache_max, None),
        (u.disk_total.saturating_sub(u.disk_available), u.disk_total, Some(format!("{} free of {}", human(u.disk_available), human(u.disk_total)))),
    ];
    for ((row, bar), (used, of, text)) in o.storage.iter().zip(values) {
        bar.set_value(if of > 0 { (used as f64 / of as f64).min(1.0) } else { 0.0 });
        row.set_subtitle(&text.unwrap_or_else(|| format!("{} of {}", human(used), human(of))));
    }
}

fn reload(me: &Rc<App>) {
    let me2 = me.clone();
    bg(&me.daemon, |d| (d.namespaces(), d.saved(), d.libraries()), move |(r, saved, libs)| {
        let list = r.unwrap_or_default();
        if let Ok(serde_json::Value::Array(libs)) = libs {
            *me2.libs.borrow_mut() = libs;
        }
        me2.find.set_namespaces(list.iter().map(|v| v.name.clone()).filter(|n| n != "me").collect());
        show_saved(&me2, saved);
        show_namespaces(&me2, &list);
        *me2.views.borrow_mut() = list;
        // RETIFS_DESKTOP_OPEN=lib:<name> or =announces: open it on start (demos, screenshots).
        thread_local! { static OPENED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
        if !OPENED.with(|o| o.replace(true)) {
            match std::env::var("RETIFS_DESKTOP_OPEN").ok().as_deref() {
                Some(o) if o.starts_with("lib:") => open_ns(&me2, &o[4..]),
                Some("announces") => {
                    if let Some(row) = me2.side_top.row_at_index(3) {
                        me2.side_top.select_row(Some(&row));
                        row.activate();
                    }
                }
                _ => {}
            }
        }
        // A namespace on screen is redrawn with what's new.
        let shown = me2.shown.borrow().clone();
        if let Some(name) = shown {
            show_ns(&me2, &name);
        }
    });
}

/// The saved searches in the sidebar (from `me`: the same on every device).
fn show_saved(me: &Rc<App>, saved: Vec<(SavedQuery, Vec<String>, usize)>) {
    let names: Vec<String> = saved.iter().map(|(q, _, _)| q.name.clone()).collect();
    let before: Vec<String> = me.saved.borrow().iter().map(|(q, _, _)| q.name.clone()).collect();
    *me.saved.borrow_mut() = saved;
    if names == before {
        return;
    }
    me.side_saved.remove_all();
    for (q, _, _) in me.saved.borrow().iter() {
        let item = gtk::ListBoxRow::builder().child(&label(&q.name)).tooltip_text(&q.query).build();
        me.side_saved.append(&item);
    }
    // RETIFS_DESKTOP_OPEN=find or =<saved search>: open it on start (demos, screenshots).
    if before.is_empty() {
        if let Ok(open) = std::env::var("RETIFS_DESKTOP_OPEN") {
            let row = if open == "find" { me.side_top.row_at_index(1) } else { names.iter().position(|n| *n == open).and_then(|i| me.side_saved.row_at_index(i as i32)) };
            if let Some(row) = row {
                // Selected and focused, as a click leaves it: the window's
                // focus coming back mustn't land on (and open) Overview.
                if let Some(list) = row.parent().and_downcast::<gtk::ListBox>() {
                    list.select_row(Some(&row));
                }
                row.grab_focus();
                row.activate();
            }
        }
    }
}

/// A new library (a namespace of the user's: just a name), or `join`: a
/// library from an invitation or link (as `retifs open`); `given`: a link
/// opened from elsewhere, already in the field.
fn ask_add_namespace(me: &Rc<App>, window: Option<gtk::Window>, join: bool, given: &str) {
    let name = gtk::Entry::builder().placeholder_text("Name").activates_default(true).build();
    let link = gtk::Entry::builder().placeholder_text("Invitation or link").activates_default(true).visible(join).build();
    {
        // A link names itself: its suggested name until the user types one.
        let name = name.clone();
        link.connect_changed(move |l| {
            name.set_placeholder_text(Some(&Daemon::link_name(&l.text()).unwrap_or_else(|_| "Name".into())));
        });
    }
    link.set_text(given);
    let fields = gtk::Box::new(gtk::Orientation::Vertical, 8);
    fields.append(&link);
    fields.append(&name);
    let (heading, body, ok) = if join {
        ("Join library", "Paste the invitation or link a member gave you. The name is how it's called here; the invitation suggests one.", "Join")
    } else {
        ("New library", "A place for photos, notes, anything. Invite others to it and they become its club.", "Create")
    };
    let dialog = adw::MessageDialog::builder().heading(heading).body(body).extra_child(&fields).default_response("add").close_response("cancel").build();
    dialog.add_responses(&[("cancel", "Cancel"), ("add", ok)]);
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_transient_for(window.as_ref());
    let me = me.clone();
    dialog.connect_response(None, move |_, r| {
        if r != "add" {
            return;
        }
        let link = link.text().trim().to_string();
        let typed = name.text().trim().to_string();
        let n = if typed.is_empty() && !link.is_empty() { Daemon::link_name(&link).unwrap_or_default() } else { typed };
        if n.is_empty() || (join && link.is_empty()) {
            return;
        }
        let (me2, window, n2) = (me.clone(), window.clone(), n.clone());
        bg(
            &me.daemon,
            move |d| if link.is_empty() { d.create_namespace(&n) } else { d.open_link(&link, &n) },
            move |r| match r {
                Ok(()) => {
                    let me3 = me2.clone();
                    bg(&me2.daemon, |d| d.namespaces(), move |list| {
                        let list = list.unwrap_or_default();
                        show_namespaces(&me3, &list);
                        *me3.views.borrow_mut() = list;
                        open_ns(&me3, &n2);
                        reload(&me3);
                    });
                }
                Err(e) => {
                    let err = adw::MessageDialog::builder().heading("Couldn't add it").body(e).build();
                    err.add_response("close", "Close");
                    err.set_transient_for(window.as_ref());
                    err.present();
                }
            },
        );
    });
    dialog.present();
}

/// A club: others are members too, or it's someone else's that this user joined.
fn is_club(ns: &NsView) -> bool {
    ns.writers.len() > 1 || ns.address.split(':').nth(2) != Some(ns.own_writer.as_str())
}

fn keeps(ns: &NsView) -> String {
    let mode = if ns.keep == "rules" { "by rules" } else { "everything" };
    if ns.lazy { format!("Keeps {mode}, lazily") } else { format!("Keeps {mode}") }
}

fn show_namespaces(me: &Rc<App>, list: &[NsView]) {
    let o = &me.overview;
    for row in o.ns_rows.borrow_mut().drain(..) {
        o.namespaces.remove(&row);
    }
    let selected = me.side_ns.selected_row().map(|r| r.widget_name().to_string());
    me.side_ns.remove_all();
    // Favourites first, `me` last (Daemon::libraries' order).
    let order: Vec<String> = me.libs.borrow().iter().filter_map(|l| l["name"].as_str().map(String::from)).collect();
    let mut list: Vec<&NsView> = list.iter().collect();
    list.sort_by_key(|ns| order.iter().position(|n| *n == ns.name).unwrap_or(usize::MAX));
    for ns in list.iter().copied() {
        let favorite = me.libs.borrow().iter().any(|l| l["name"] == ns.name.as_str() && l["favorite"].as_bool().unwrap_or(false));
        let row = adw::ActionRow::builder()
            .title(&ns.name)
            .subtitle(format!("{} members · {} peers · {}", ns.writers.len(), ns.peers, keeps(ns).to_lowercase()))
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let (me2, name) = (me.clone(), ns.name.clone());
        row.connect_activated(move |_| open_ns(&me2, &name));
        o.namespaces.add(&row);
        o.ns_rows.borrow_mut().push(row);

        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let name = label(&if favorite { format!("★ {}", ns.name) } else { ns.name.clone() });
        name.set_hexpand(true);
        line.append(&name);
        if is_club(ns) {
            let mark = gtk::Image::from_icon_name("system-users-symbolic");
            mark.add_css_class("dim-label");
            mark.set_tooltip_text(Some(&format!("Shared: a club of {}", ns.writers.len().max(2))));
            line.append(&mark);
        }
        if ns.coming() {
            let coming = gtk::Spinner::builder().spinning(true).tooltip_text("Coming in").build();
            line.append(&coming);
        }
        let item = gtk::ListBoxRow::builder().child(&line).build();
        item.set_widget_name(&format!("ns:{}", ns.name));
        // Right-click: what can be done with it.
        let click = gtk::GestureClick::builder().button(3).build();
        {
            let (me2, name, item2) = (me.clone(), ns.name.clone(), item.clone());
            click.connect_pressed(move |_, _, x, y| {
                let pop = gtk::PopoverMenu::from_model(Some(&library::menu(&me2, &name)));
                pop.set_parent(&item2);
                pop.set_has_arrow(false);
                pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                pop.connect_closed(|p| {
                    let p = p.clone();
                    glib::idle_add_local_once(move || p.unparent());
                });
                pop.popup();
            });
        }
        item.add_controller(click);
        me.side_ns.append(&item);
        if selected.as_deref() == Some(item.widget_name().as_str()) {
            me.side_ns.select_row(Some(&item));
        }
    }
    if list.is_empty() {
        let row = adw::ActionRow::builder().title("Nothing here yet").build();
        o.namespaces.add(&row);
        o.ns_rows.borrow_mut().push(row);
    }
}

fn open_ns(me: &Rc<App>, name: &str) {
    me.side_top.unselect_all();
    let mut i = 0;
    while let Some(row) = me.side_ns.row_at_index(i) {
        if row.widget_name().as_str() == format!("ns:{name}") {
            me.side_ns.select_row(Some(&row));
        }
        i += 1;
    }
    *me.shown.borrow_mut() = Some(name.to_string());
    me.title.set_title(name);
    show_ns(me, name);
    me.stack.set_visible_child_name("namespace");
}

/// A library's page, from its view (the library module's).
fn show_ns(me: &Rc<App>, name: &str) {
    let views = me.views.borrow();
    let Some(ns) = views.iter().find(|v| v.name == name) else {
        me.ns_page.set_child(Some(&adw::StatusPage::builder().title(name).description("Not found on this node.").build()));
        return;
    };
    me.ns_page.set_child(Some(&library::page(me, ns)));
}
