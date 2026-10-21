//! retifs-desktop for Linux: native GTK 4 / libadwaita widgets over the
//! shared core. A sidebar (overview, namespaces), the overview as
//! preference groups, a namespace's records with search and history, and a
//! banner when the daemon isn't running.

use std::{cell::RefCell, rc::Rc, sync::Arc};

use adw::prelude::*;
use gtk::{gio, glib};
use retifs_desktop_core::{changes, human, Daemon, NsView, Record, Status};

const APP_ID: &str = "org.retifs.Desktop";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build);
    app.run()
}

/// Runs `work` on a worker thread (the daemon calls block), then `then` with
/// its result on the UI thread.
fn bg<T: Send + 'static>(d: &Arc<Daemon>, work: impl FnOnce(&Daemon) -> T + Send + 'static, then: impl FnOnce(T) + 'static) {
    let d = d.clone();
    glib::spawn_future_local(async move {
        if let Ok(r) = gio::spawn_blocking(move || work(&d)).await {
            then(r);
        }
    });
}

/// The widgets the overview updates on each status poll.
struct Overview {
    node: adw::ActionRow,
    storage: Vec<(adw::ActionRow, gtk::LevelBar)>,
    namespaces: adw::PreferencesGroup,
    ns_rows: RefCell<Vec<adw::ActionRow>>,
}

/// A namespace's page: search, the records found, the selected record.
struct NsPage {
    current: RefCell<Option<String>>,
    search: gtk::SearchEntry,
    count: gtk::Label,
    list: gtk::ListBox,
    found: RefCell<Vec<Record>>,
    detail: gtk::ScrolledWindow,
}

fn build(app: &adw::Application) {
    let daemon = Arc::new(Daemon::new());

    // ---- content: a stack of the overview and a namespace
    let title = adw::WindowTitle::new("Overview", "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));
    let banner = adw::Banner::new("retifsd is not running");
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);

    let overview = Rc::new(overview_page());
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

    let ns = Rc::new(ns_page());
    let ns_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    search_row.set_margin_top(12);
    search_row.set_margin_start(12);
    search_row.set_margin_end(12);
    ns.search.set_hexpand(true);
    ns.search.set_placeholder_text(Some("Search records, e.g. album=\"Prangli 2026\" type=image"));
    search_row.append(&ns.search);
    search_row.append(&ns.count);
    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    let list_scroll = gtk::ScrolledWindow::builder().child(&ns.list).hscrollbar_policy(gtk::PolicyType::Never).build();
    paned.set_start_child(Some(&list_scroll));
    paned.set_end_child(Some(&ns.detail));
    paned.set_position(300);
    paned.set_vexpand(true);
    ns_box.append(&search_row);
    ns_box.append(&paned);
    stack.add_named(&ns_box, Some("namespace"));

    let content = adw::ToolbarView::new();
    content.add_top_bar(&header);
    content.add_top_bar(&banner);
    content.set_content(Some(&stack));
    let content_page = adw::NavigationPage::builder().title("retifs").child(&content).build();

    // ---- sidebar: overview, then one row per namespace
    let side_top = gtk::ListBox::new();
    side_top.add_css_class("navigation-sidebar");
    side_top.append(&sidebar_label("Overview"));
    let side_head = gtk::Label::builder().label("Namespaces").xalign(0.0).margin_start(18).margin_top(12).margin_bottom(4).build();
    side_head.add_css_class("dim-label");
    side_head.add_css_class("caption-heading");
    let side_ns = gtk::ListBox::new();
    side_ns.add_css_class("navigation-sidebar");
    let side_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    side_box.append(&side_top);
    side_box.append(&side_head);
    side_box.append(&side_ns);
    let side_scroll = gtk::ScrolledWindow::builder().child(&side_box).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
    let side_view = adw::ToolbarView::new();
    side_view.add_top_bar(&adw::HeaderBar::new());
    side_view.set_content(Some(&side_scroll));
    let side_page = adw::NavigationPage::builder().title("retifs").child(&side_view).build();

    let split = adw::NavigationSplitView::builder().sidebar(&side_page).content(&content_page).build();
    let window = adw::ApplicationWindow::builder().application(app).title("retifs").default_width(1000).default_height(680).content(&split).build();

    // ---- behaviour
    let open_ns: Rc<dyn Fn(&str)> = {
        let (ns, stack, title, daemon, side_top) = (ns.clone(), stack.clone(), title.clone(), daemon.clone(), side_top.clone());
        Rc::new(move |name: &str| {
            side_top.unselect_all();
            *ns.current.borrow_mut() = Some(name.to_string());
            title.set_title(name);
            ns.search.set_text("");
            ns.detail.set_child(Some(&hint("Select a record to see its attributes and history.")));
            stack.set_visible_child_name("namespace");
            run_query(&daemon, &ns);
        })
    };
    let reload_ns: Rc<dyn Fn()> = {
        let (daemon, overview, side_ns, open_ns) = (daemon.clone(), overview.clone(), side_ns.clone(), open_ns.clone());
        Rc::new(move || {
            let (overview, side_ns, open_ns) = (overview.clone(), side_ns.clone(), open_ns.clone());
            bg(&daemon, |d| d.namespaces(), move |r| show_namespaces(&overview, &side_ns, &open_ns, r.unwrap_or_default()))
        })
    };
    {
        let (stack, title, side_ns, reload) = (stack.clone(), title.clone(), side_ns.clone(), reload_ns.clone());
        side_top.connect_row_activated(move |_, _| {
            side_ns.unselect_all();
            title.set_title("Overview");
            stack.set_visible_child_name("overview");
            reload();
        });
    }
    {
        let open_ns = open_ns.clone();
        side_ns.connect_row_activated(move |_, row| {
            if let Some(name) = row.widget_name().as_str().strip_prefix("ns:") {
                open_ns(name);
            }
        });
    }
    {
        let (daemon, ns2) = (daemon.clone(), ns.clone());
        ns.search.connect_activate(move |_| run_query(&daemon, &ns2));
    }
    {
        let (daemon, ns2) = (daemon.clone(), ns.clone());
        ns.list.connect_row_activated(move |_, row| {
            let r = ns2.found.borrow().get(row.index() as usize).cloned();
            if let Some(r) = r {
                show_record(&daemon, &ns2, r);
            }
        });
    }

    // The daemon's state, every 5 s: banner, subtitle, storage.
    let poll = {
        let (daemon, banner, title, overview, reload, was_up) = (daemon.clone(), banner.clone(), title.clone(), overview.clone(), reload_ns.clone(), Rc::new(RefCell::new(false)));
        move || {
            let (banner, title, overview, reload, was_up) = (banner.clone(), title.clone(), overview.clone(), reload.clone(), was_up.clone());
            bg(&daemon, |d| d.status(), move |s: Status| {
                banner.set_revealed(!s.up);
                title.set_subtitle(&match &s.node {
                    Some(n) => format!("node {}…", &n[..8.min(n.len())]),
                    None => "not connected".into(),
                });
                show_status(&overview, &s);
                if s.up && !*was_up.borrow() {
                    reload(); // came up: (re)load the namespaces
                }
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

fn sidebar_label(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build()
}

fn hint(text: &str) -> adw::StatusPage {
    adw::StatusPage::builder().description(text).build()
}

fn overview_page() -> Overview {
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

fn show_namespaces(o: &Overview, side: &gtk::ListBox, open: &Rc<dyn Fn(&str)>, list: Vec<NsView>) {
    for row in o.ns_rows.borrow_mut().drain(..) {
        o.namespaces.remove(&row);
    }
    side.remove_all();
    for ns in &list {
        let keeps = if ns.lazy { format!("keeps {} (lazy)", ns.keep) } else { format!("keeps {}", ns.keep) };
        let row = adw::ActionRow::builder()
            .title(&ns.name)
            .subtitle(format!("{} records · {} writers · {} peers · {keeps}", ns.records, ns.writers.len(), ns.peers))
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let (open, name) = (open.clone(), ns.name.clone());
        row.connect_activated(move |_| open(&name));
        o.namespaces.add(&row);
        o.ns_rows.borrow_mut().push(row);

        let item = gtk::ListBoxRow::builder().child(&sidebar_label(&ns.name)).build();
        item.set_widget_name(&format!("ns:{}", ns.name));
        side.append(&item);
    }
    if list.is_empty() {
        let row = adw::ActionRow::builder().title("No namespaces yet").build();
        o.namespaces.add(&row);
        o.ns_rows.borrow_mut().push(row);
    }
}

fn ns_page() -> NsPage {
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    let count = gtk::Label::new(None);
    count.add_css_class("dim-label");
    NsPage {
        current: RefCell::new(None),
        search: gtk::SearchEntry::new(),
        count,
        list,
        found: RefCell::new(Vec::new()),
        detail: gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).hexpand(true).build(),
    }
}

const SHOWN: usize = 1000;

fn run_query(daemon: &Arc<Daemon>, page: &Rc<NsPage>) {
    let Some(ns) = page.current.borrow().clone() else { return };
    let line = page.search.text().to_string();
    let page = page.clone();
    bg(daemon, move |d| d.query(&ns, &line), move |r| {
        page.list.remove_all();
        match r {
            Ok(found) => {
                page.count.set_text(&if found.len() > SHOWN { format!("{} records, first {SHOWN} shown", found.len()) } else { format!("{} records", found.len()) });
                for rec in found.iter().take(SHOWN) {
                    page.list.append(&sidebar_label(&rec.label()));
                }
                *page.found.borrow_mut() = found.into_iter().take(SHOWN).collect();
            }
            Err(e) => {
                page.count.set_text(&e);
                page.found.borrow_mut().clear();
            }
        }
    });
}

fn show_record(daemon: &Arc<Daemon>, page: &Rc<NsPage>, r: Record) {
    let pref = adw::PreferencesPage::new();
    let attrs = adw::PreferencesGroup::builder().title(r.label()).description(&r.id).build();
    for (k, v) in &r.attrs {
        attrs.add(&adw::ActionRow::builder().title(k).subtitle(v).subtitle_selectable(true).build());
    }
    pref.add(&attrs);
    if !r.blocks.is_empty() {
        let content = adw::PreferencesGroup::builder().title("Content").build();
        for b in &r.blocks {
            content.add(&adw::ActionRow::builder().title("Read capability").subtitle(b).subtitle_selectable(true).build());
        }
        pref.add(&content);
    }
    let history = adw::PreferencesGroup::builder().title("History").build();
    pref.add(&history);
    page.detail.set_child(Some(&pref));

    let Some(ns) = page.current.borrow().clone() else { return };
    let id = r.id.clone();
    bg(daemon, move |d| d.history(&ns, &id), move |res| match res {
        Ok(versions) => {
            for (i, v) in versions.iter().enumerate() {
                let when = glib::DateTime::from_unix_local((v.time_ms / 1000) as i64)
                    .and_then(|t| t.format("%x %X"))
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| v.time_ms.to_string());
                let what = changes(v.record.as_ref(), versions.get(i + 1).and_then(|o| o.record.as_ref())).join("\n");
                history.add(&adw::ActionRow::builder().title(when).subtitle(what).build());
            }
        }
        Err(e) => history.add(&adw::ActionRow::builder().title("History not available").subtitle(e).build()),
    });
}
