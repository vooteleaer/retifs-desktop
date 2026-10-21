//! retifs-desktop for Linux: native GTK 4 / libadwaita widgets over the
//! shared core. A sidebar (overview, namespaces), the overview, a
//! namespace's management page (sharing, members, what this device keeps,
//! state), and a banner when the daemon isn't running. It doesn't browse
//! content: that's what apps such as a photo album are for.

use std::{cell::RefCell, rc::Rc, sync::Arc};

use adw::prelude::*;
use gtk::{gio, glib};
use retifs_desktop_core::{human, Daemon, NsView, Status};

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
    side_ns: gtk::ListBox,
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

    let content = adw::ToolbarView::new();
    content.add_top_bar(&header);
    content.add_top_bar(&banner);
    content.set_content(Some(&stack));
    let content_page = adw::NavigationPage::builder().title("retifs").child(&content).build();

    let side_top = gtk::ListBox::new();
    side_top.add_css_class("navigation-sidebar");
    side_top.append(&label("Overview"));
    let side_head = gtk::Label::builder().label("Namespaces").xalign(0.0).margin_start(18).margin_top(12).margin_bottom(4).build();
    side_head.add_css_class("dim-label");
    side_head.add_css_class("caption-heading");
    let side_ns = gtk::ListBox::new();
    side_ns.add_css_class("navigation-sidebar");
    let side_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    side_box.append(&side_top);
    side_box.append(&side_head);
    side_box.append(&side_ns);
    let side_view = adw::ToolbarView::new();
    side_view.add_top_bar(&adw::HeaderBar::new());
    side_view.set_content(Some(&gtk::ScrolledWindow::builder().child(&side_box).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build()));
    let side_page = adw::NavigationPage::builder().title("retifs").child(&side_view).build();

    let split = adw::NavigationSplitView::builder().sidebar(&side_page).content(&content_page).build();
    let window = adw::ApplicationWindow::builder().application(app).title("retifs").default_width(960).default_height(660).content(&split).build();

    let me = Rc::new(App {
        daemon: Arc::new(Daemon::new()),
        title,
        stack,
        overview,
        ns_page,
        side_top: side_top.clone(),
        side_ns: side_ns.clone(),
        views: RefCell::new(Vec::new()),
        shown: RefCell::new(None),
    });

    {
        let me = me.clone();
        side_top.connect_row_activated(move |_, _| {
            me.side_ns.unselect_all();
            *me.shown.borrow_mut() = None;
            me.title.set_title("Overview");
            me.stack.set_visible_child_name("overview");
            reload(&me);
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
    let namespaces = adw::PreferencesGroup::builder().title("Namespaces").build();
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
    bg(&me.daemon, |d| d.namespaces(), move |r| {
        let list = r.unwrap_or_default();
        show_namespaces(&me2, &list);
        *me2.views.borrow_mut() = list;
        // A namespace on screen is redrawn with what's new.
        let shown = me2.shown.borrow().clone();
        if let Some(name) = shown {
            show_ns(&me2, &name);
        }
    });
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

        let item = gtk::ListBoxRow::builder().child(&label(&ns.name)).build();
        item.set_widget_name(&format!("ns:{}", ns.name));
        me.side_ns.append(&item);
        if selected.as_deref() == Some(item.widget_name().as_str()) {
            me.side_ns.select_row(Some(&item));
        }
    }
    if list.is_empty() {
        let row = adw::ActionRow::builder().title("No namespaces yet").build();
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
        .description("Anyone with this address can read the namespace. To let someone write, grant their writer key below (invitations come later).")
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
