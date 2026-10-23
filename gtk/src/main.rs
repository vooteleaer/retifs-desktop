//! retifs-desktop for Linux: native GTK 4 / libadwaita widgets over the
//! shared core. A sidebar (overview, Find, saved searches, libraries), the
//! overview, Find (a live query, as BeOS's Find), a namespace's management
//! page (sharing, members, what this device keeps, state), and a banner when
//! the daemon isn't running. It finds, but doesn't browse or catalogue
//! content: that's what apps such as a photo album are for.

mod find;

use std::{cell::RefCell, rc::Rc, sync::Arc};

use adw::prelude::*;
use gtk::{gio, glib};
use retifs_desktop_core::{human, Daemon, NsView, SavedQuery, Status};

const APP_ID: &str = "org.retifs.Desktop";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build);
    app.run()
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
    saved: RefCell<Vec<(SavedQuery, Vec<String>, usize)>>,
    views: RefCell<Vec<NsView>>,
    shown: RefCell<Option<String>>,
}

fn build(app: &adw::Application) {
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

    let content = adw::ToolbarView::new();
    content.add_top_bar(&header);
    content.add_top_bar(&banner);
    content.set_content(Some(&stack));
    let content_page = adw::NavigationPage::builder().title("retifs").child(&content).build();

    let side_top = gtk::ListBox::new();
    side_top.add_css_class("navigation-sidebar");
    side_top.append(&label("Overview"));
    side_top.append(&label("Find"));
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
        saved: RefCell::new(Vec::new()),
        views: RefCell::new(Vec::new()),
        shown: RefCell::new(None),
    });

    {
        let me = me.clone();
        side_top.connect_row_activated(move |_, row| {
            me.side_ns.unselect_all();
            me.side_saved.unselect_all();
            *me.shown.borrow_mut() = None;
            if row.index() == 1 {
                me.title.set_title("Find");
                me.stack.set_visible_child_name("find");
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
            ask_add_namespace(&me2, b.root().and_downcast(), false);
        });
        let pop = add_popover.clone();
        join_library.connect_clicked(move |b| {
            pop.popdown();
            ask_add_namespace(&me, b.root().and_downcast(), true);
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
            bg(&me.daemon, |d| d.status(), move |s: Status| {
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

    side_top.select_row(side_top.row_at_index(0).as_ref());
    window.present();

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
    bg(&me.daemon, |d| (d.namespaces(), d.saved()), move |(r, saved)| {
        let list = r.unwrap_or_default();
        me2.find.set_namespaces(list.iter().map(|v| v.name.clone()).filter(|n| n != "me").collect());
        show_saved(&me2, saved);
        show_namespaces(&me2, &list);
        *me2.views.borrow_mut() = list;
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
/// library from an invitation or link (as `retifs open`).
fn ask_add_namespace(me: &Rc<App>, window: Option<gtk::Window>, join: bool) {
    let name = gtk::Entry::builder().placeholder_text("Name").activates_default(true).build();
    let link = gtk::Entry::builder().placeholder_text("Invitation or link").activates_default(true).visible(join).build();
    {
        // A link names itself: its suggested name until the user types one.
        let name = name.clone();
        link.connect_changed(move |l| {
            name.set_placeholder_text(Some(&Daemon::link_name(&l.text()).unwrap_or_else(|_| "Name".into())));
        });
    }
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
    for ns in list {
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
        let name = label(&ns.name);
        name.set_hexpand(true);
        line.append(&name);
        if is_club(ns) {
            let mark = gtk::Image::from_icon_name("system-users-symbolic");
            mark.add_css_class("dim-label");
            mark.set_tooltip_text(Some(&format!("Shared: a club of {}", ns.writers.len().max(2))));
            line.append(&mark);
        }
        let item = gtk::ListBoxRow::builder().child(&line).build();
        item.set_widget_name(&format!("ns:{}", ns.name));
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

/// A row with a value the user may want to copy (an address, a key).
fn copy_row(title: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).subtitle(value).subtitle_selectable(true).build();
    let button = gtk::Button::from_icon_name("edit-copy-symbolic");
    button.set_valign(gtk::Align::Center);
    button.add_css_class("flat");
    button.set_tooltip_text(Some("Copy"));
    let value = value.to_string();
    button.connect_clicked(move |b| {
        b.clipboard().set_text(&value);
    });
    row.add_suffix(&button);
    row
}

/// A namespace's management page, from its view.
fn show_ns(me: &Rc<App>, name: &str) {
    let views = me.views.borrow();
    let Some(ns) = views.iter().find(|v| v.name == name) else {
        me.ns_page.set_child(Some(&adw::StatusPage::builder().title(name).description("Not found on this node.").build()));
        return;
    };
    let page = adw::PreferencesPage::new();

    let sharing = adw::PreferencesGroup::builder()
        .title("Sharing")
        .description("Anyone with this address can read it. To let someone write, grant their writer key below (invitations come later).")
        .build();
    sharing.add(&copy_row("Address", &ns.address));
    page.add(&sharing);

    let members = adw::PreferencesGroup::builder().title("Members").build();
    for (writer, rights) in &ns.writers {
        let mut title = format!("{}…", &writer[..16.min(writer.len())]);
        if *writer == ns.own_writer {
            title.push_str("  (this node)");
        }
        let row = copy_row(&title, writer);
        row.set_subtitle(&rights.join(", "));
        row.set_tooltip_text(Some(writer));
        members.add(&row);
    }
    page.add(&members);

    let device = adw::PreferencesGroup::builder().title("On this device").build();
    device.add(&adw::ActionRow::builder().title(keeps(ns)).subtitle(format!("{} file(s) pulled here because they're kept", ns.kept)).build());
    device.add(&adw::ActionRow::builder().title("Device name").subtitle(&ns.device).build());
    device.add(&adw::ActionRow::builder().title("Peers").subtitle(format!("{} node(s) this one syncs with", ns.peers)).build());
    page.add(&device);

    let state = adw::PreferencesGroup::builder().title("State").build();
    state.add(&adw::ActionRow::builder().title("Records").subtitle(ns.records.to_string()).build());
    match &ns.checkpoint {
        Some(cp) => state.add(&copy_row("Checkpoint", cp)),
        None => state.add(&adw::ActionRow::builder().title("Checkpoint").subtitle("None yet").build()),
    }
    page.add(&state);

    me.ns_page.set_child(Some(&page));
}
