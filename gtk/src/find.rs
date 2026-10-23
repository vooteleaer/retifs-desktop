//! Find: a live query, in the spirit of BeOS's Find in Tracker. Rows of
//! attribute, condition and value over a scope of namespaces; the results
//! update as records change; a search can be saved (in `me`, so every device
//! of the user's has it) and opened again from the sidebar.

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use adw::prelude::*;
use gtk::{gio, glib};
use retifs_desktop_core::{Daemon, Hit};

use crate::bg;

/// Conditions as the window names them, and as terms write them.
const OPS: [(&str, &str); 7] = [
    ("is", "="),
    ("starts with", "^="),
    ("is set", "has:"),
    ("more than", ">"),
    ("at least", ">="),
    ("less than", "<"),
    ("at most", "<="),
];

struct Condition {
    row: gtk::ListBoxRow,
    attr: gtk::Entry,
    op: gtk::DropDown,
    value: gtk::Entry,
}

pub struct Find {
    pub page: gtk::Box,
    daemon: Arc<Daemon>,
    scope: gtk::DropDown,
    scope_names: RefCell<Vec<String>>,
    /// A saved query's own namespaces, until the scope is changed by hand.
    saved_scope: RefCell<Option<Vec<String>>>,
    scope_note: gtk::Label,
    conditions_box: gtk::ListBox,
    conditions: RefCell<Vec<Condition>>,
    results: gtk::ListBox,
    count: gtk::Label,
    /// The search on screen: (scope, terms), and the change counts it saw.
    active: RefCell<Option<(Vec<String>, Vec<String>)>>,
    seen: RefCell<HashMap<String, u64>>,
    /// Called after a search is saved (the sidebar lists saved ones).
    pub on_saved: RefCell<Option<Box<dyn Fn()>>>,
}

impl Find {
    pub fn new(daemon: Arc<Daemon>) -> Rc<Find> {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        page.set_margin_top(18);
        page.set_margin_bottom(18);
        page.set_margin_start(18);
        page.set_margin_end(18);

        let scope_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        scope_row.append(&gtk::Label::new(Some("Look in")));
        let scope = gtk::DropDown::from_strings(&["All namespaces"]);
        scope_row.append(&scope);
        let scope_note = gtk::Label::new(None);
        scope_note.add_css_class("dim-label");
        scope_row.append(&scope_note);
        page.append(&scope_row);

        let conditions_box = gtk::ListBox::new();
        conditions_box.set_selection_mode(gtk::SelectionMode::None);
        conditions_box.add_css_class("boxed-list");
        page.append(&conditions_box);

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let add = gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text("Add a condition").build();
        let run = gtk::Button::with_label("Find");
        run.add_css_class("suggested-action");
        let save = gtk::Button::with_label("Save…");
        buttons.append(&add);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        buttons.append(&spacer);
        buttons.append(&save);
        buttons.append(&run);
        page.append(&buttons);

        let count = gtk::Label::builder().xalign(0.0).label("Set conditions and press Find").build();
        count.add_css_class("dim-label");
        page.append(&count);
        let results = gtk::ListBox::new();
        results.add_css_class("boxed-list");
        results.set_selection_mode(gtk::SelectionMode::None);
        let scroll = gtk::ScrolledWindow::builder().child(&results).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        page.append(&scroll);

        let f = Rc::new(Find {
            page,
            daemon,
            scope,
            scope_names: RefCell::new(Vec::new()),
            saved_scope: RefCell::new(None),
            scope_note,
            conditions_box,
            conditions: RefCell::new(Vec::new()),
            results,
            count,
            active: RefCell::new(None),
            seen: RefCell::new(HashMap::new()),
            on_saved: RefCell::new(None),
        });
        f.add_condition("", "is", "");
        {
            let f2 = f.clone();
            add.connect_clicked(move |_| f2.add_condition("", "is", ""));
        }
        {
            let f2 = f.clone();
            run.connect_clicked(move |_| f2.run());
        }
        {
            let f2 = f.clone();
            save.connect_clicked(move |b| f2.ask_to_save(b));
        }
        {
            let f2 = f.clone();
            f.scope.connect_selected_notify(move |_| {
                *f2.saved_scope.borrow_mut() = None;
                f2.scope_note.set_text("");
            });
        }
        // Live: when a namespace in scope changes, the search runs again.
        {
            let f2 = f.clone();
            glib::timeout_add_seconds_local(3, move || {
                f2.refresh_if_changed();
                glib::ControlFlow::Continue
            });
        }
        f
    }

    /// The namespaces the scope menu offers.
    pub fn set_namespaces(self: &Rc<Self>, names: Vec<String>) {
        if *self.scope_names.borrow() == names {
            return;
        }
        let selected = self.selected_namespace();
        let mut items = vec!["All namespaces".to_string()];
        items.extend(names.iter().cloned());
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        self.scope.set_model(Some(&gtk::StringList::new(&refs)));
        let i = selected.and_then(|s| names.iter().position(|n| *n == s)).map_or(0, |i| i + 1);
        self.scope.set_selected(i as u32);
        *self.scope_names.borrow_mut() = names;
    }

    fn selected_namespace(&self) -> Option<String> {
        let i = self.scope.selected() as usize;
        (i > 0).then(|| self.scope_names.borrow().get(i - 1).cloned()).flatten()
    }

    fn scope(&self) -> Vec<String> {
        if let Some(s) = self.saved_scope.borrow().as_ref() {
            return s.clone();
        }
        self.selected_namespace().into_iter().collect()
    }

    fn add_condition(self: &Rc<Self>, attr: &str, op: &str, value: &str) {
        let attr_e = gtk::Entry::builder().placeholder_text("attribute (album, type, path…)").text(attr).hexpand(true).build();
        let names: Vec<&str> = OPS.iter().map(|(n, _)| *n).collect();
        let op_d = gtk::DropDown::from_strings(&names);
        op_d.set_selected(OPS.iter().position(|(n, _)| *n == op).unwrap_or(0) as u32);
        let value_e = gtk::Entry::builder().placeholder_text("value").text(value).hexpand(true).build();
        let remove = gtk::Button::builder().icon_name("list-remove-symbolic").tooltip_text("Remove").build();
        remove.add_css_class("flat");
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        line.set_margin_top(6);
        line.set_margin_bottom(6);
        line.set_margin_start(6);
        line.set_margin_end(6);
        line.append(&attr_e);
        line.append(&op_d);
        line.append(&value_e);
        line.append(&remove);
        let row = gtk::ListBoxRow::builder().child(&line).activatable(false).build();
        self.conditions_box.append(&row);
        {
            let value_e = value_e.clone();
            op_d.connect_selected_notify(move |d| value_e.set_sensitive(OPS[d.selected() as usize].1 != "has:"));
        }
        value_e.set_sensitive(OPS[op_d.selected() as usize].1 != "has:");
        for e in [&attr_e, &value_e] {
            let f = self.clone();
            e.connect_activate(move |_| f.run());
        }
        {
            let (f, row) = (self.clone(), row.clone());
            remove.connect_clicked(move |_| {
                f.conditions_box.remove(&row);
                f.conditions.borrow_mut().retain(|c| c.row != row);
            });
        }
        self.conditions.borrow_mut().push(Condition { row, attr: attr_e, op: op_d, value: value_e });
    }

    /// The conditions as terms (rows without an attribute left out).
    fn terms(&self) -> Vec<String> {
        self.conditions
            .borrow()
            .iter()
            .filter_map(|c| {
                let attr = c.attr.text().trim().to_string();
                if attr.is_empty() {
                    return None;
                }
                let op = OPS[c.op.selected() as usize].1;
                Some(if op == "has:" { format!("has:{attr}") } else { format!("{attr}{op}{}", c.value.text().trim()) })
            })
            .collect()
    }

    pub fn run(self: &Rc<Self>) {
        let (scope, terms) = (self.scope(), self.terms());
        if terms.is_empty() {
            self.count.set_text("Add a condition: an attribute, and what it should be");
            return;
        }
        *self.active.borrow_mut() = Some((scope.clone(), terms.clone()));
        let f = self.clone();
        let (s2, t2) = (scope.clone(), terms.clone());
        bg(&self.daemon, move |d| (d.find(&s2, &t2), d.change_counts(&s2)), move |(found, counts)| {
            *f.seen.borrow_mut() = counts;
            f.show(found);
        });
    }

    fn refresh_if_changed(self: &Rc<Self>) {
        let Some((scope, terms)) = self.active.borrow().clone() else { return };
        let f = self.clone();
        bg(&self.daemon, move |d| d.change_counts(&scope), move |counts| {
            if counts != *f.seen.borrow() {
                *f.active.borrow_mut() = Some((f.scope(), terms));
                f.run();
            }
        });
    }

    fn show(self: &Rc<Self>, found: Result<Vec<Hit>, String>) {
        self.results.remove_all();
        let hits = match found {
            Ok(h) => h,
            Err(e) => {
                self.count.set_text(&e);
                return;
            }
        };
        self.count.set_text(&format!("{} record{} · updates as they change", hits.len(), if hits.len() == 1 { "" } else { "s" }));
        for h in hits.iter().take(1000) {
            let row = adw::ActionRow::builder().title(glib::markup_escape_text(&h.title())).subtitle(glib::markup_escape_text(&format!("{} · {}", h.ns, h.summary()))).build();
            row.set_subtitle_lines(2);
            // Opening it: the file, where the namespace is mounted.
            if let Some(path) = h.path() {
                let file = std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join("retifs").join(&h.ns).join(path.trim_start_matches('/')));
                if let Some(file) = file.filter(|f| f.exists()) {
                    row.set_activatable(true);
                    row.add_suffix(&gtk::Image::from_icon_name("document-open-symbolic"));
                    row.connect_activated(move |r| {
                        let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(&file)));
                        launcher.launch(r.root().and_downcast_ref::<gtk::Window>(), gio::Cancellable::NONE, |_| {});
                    });
                }
            }
            self.results.append(&row);
        }
    }

    /// Opens a saved query: its conditions, its namespaces, and runs it.
    pub fn open_saved(self: &Rc<Self>, terms: &[String], scope: Vec<String>, missing: usize) {
        for c in self.conditions.borrow_mut().drain(..) {
            self.conditions_box.remove(&c.row);
        }
        for t in terms {
            let (attr, op, value) = split_term(t);
            self.add_condition(&attr, op, &value);
        }
        if terms.is_empty() {
            self.add_condition("", "is", "");
        }
        match scope.len() {
            0 => self.scope.set_selected(0),
            1 => {
                let i = self.scope_names.borrow().iter().position(|n| *n == scope[0]).map_or(0, |i| i + 1);
                self.scope.set_selected(i as u32);
            }
            _ => self.scope.set_selected(0),
        }
        let note = match (scope.len(), missing) {
            (n, 0) if n > 1 => format!("{}", scope.join(", ")),
            (_, 0) => String::new(),
            (_, m) => format!("{} (and {m} not on this device)", scope.join(", ")),
        };
        *self.saved_scope.borrow_mut() = (scope.len() > 1 || missing > 0).then_some(scope);
        self.scope_note.set_text(&note);
        self.run();
    }

    fn ask_to_save(self: &Rc<Self>, anchor: &gtk::Button) {
        let terms = self.terms();
        if terms.is_empty() {
            self.count.set_text("Nothing to save yet: add a condition");
            return;
        }
        let entry = gtk::Entry::builder().placeholder_text("Name").activates_default(true).build();
        let dialog = adw::MessageDialog::builder()
            .heading("Save this search")
            .body("Saved searches are kept in your namespace me, so all your devices have them.")
            .extra_child(&entry)
            .default_response("save")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[("cancel", "Cancel"), ("save", "Save")]);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        if let Some(w) = anchor.root().and_downcast::<gtk::Window>() {
            dialog.set_transient_for(Some(&w));
        }
        let f = self.clone();
        dialog.connect_response(None, move |_, r| {
            let name = entry.text().trim().to_string();
            if r != "save" || name.is_empty() {
                return;
            }
            let (scope, terms) = (f.scope(), terms.clone());
            let f2 = f.clone();
            bg(&f.daemon, move |d| d.save(&name, &scope, &terms), move |r| match r {
                Ok(()) => {
                    if let Some(cb) = f2.on_saved.borrow().as_ref() {
                        cb();
                    }
                }
                Err(e) => f2.count.set_text(&format!("Not saved: {e}")),
            });
        });
        dialog.present();
    }
}

/// `album^=Pr` → ("album", "starts with", "Pr").
fn split_term(t: &str) -> (String, &'static str, String) {
    if let Some(k) = t.strip_prefix("has:") {
        return (k.to_string(), "is set", String::new());
    }
    for (name, op) in [("starts with", "^="), ("at least", ">="), ("at most", "<="), ("is", "="), ("more than", ">"), ("less than", "<")] {
        if let Some((k, v)) = t.split_once(op) {
            return (k.to_string(), name, v.to_string());
        }
    }
    (t.to_string(), "is set", String::new())
}

#[cfg(test)]
mod tests {
    #[test]
    fn terms_come_back_as_conditions() {
        assert_eq!(super::split_term("album=Prangli 2026"), ("album".into(), "is", "Prangli 2026".into()));
        assert_eq!(super::split_term("path^=/Pictures/"), ("path".into(), "starts with", "/Pictures/".into()));
        assert_eq!(super::split_term("size>=1000"), ("size".into(), "at least", "1000".into()));
        assert_eq!(super::split_term("has:geo"), ("geo".into(), "is set", "".into()));
    }
}
