//! A library's page and what can be done with one, as in the Haiku utility
//! (the shared design: docs/plan.md): who it is shared with (members by
//! name, their roles, invitations, removing), whether it's public, kept on
//! this device (and how far that has come), a favourite. The same actions
//! from a right-click in the sidebar. And Announces: the public libraries
//! heard on the network, each with Join.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use retifs_desktop_core::NsView;
use serde_json::Value;

use crate::{bg, open_ns, reload, App};

/// The window's actions on a library (each takes its name): the sidebar's
/// right-click menu and the page's buttons use them.
pub fn add_actions(me: &Rc<App>, window: &adw::ApplicationWindow) {
    let act = |name: &str, f: Box<dyn Fn(&Rc<App>, String)>| {
        let a = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
        let me = me.clone();
        a.connect_activate(move |_, p| {
            if let Some(ns) = p.and_then(|p| p.get::<String>()) {
                f(&me, ns);
            }
        });
        window.add_action(&a);
    };
    act("favorite", Box::new(|me, ns| {
        let on = !flag(me, &ns, "favorite");
        bg(&me.daemon, move |d| d.favorite(&ns, on), done(me));
    }));
    act("keep", Box::new(|me, ns| {
        let all = !kept_all(me, &ns);
        bg(&me.daemon, move |d| d.keep(&ns, all), done(me));
    }));
    act("copy-link", Box::new(|me, ns| {
        let me2 = me.clone();
        bg(&me.daemon, move |d| d.link(&ns), move |r| match r {
            Ok(link) => me2.window().clipboard().set_text(&link),
            Err(e) => error(&me2, "Couldn't make its link", &e),
        });
    }));
    act("announce", Box::new(|me, ns| {
        let on = !flag(me, &ns, "announced");
        if !on {
            bg(&me.daemon, move |d| d.announce(&ns, false), done(me));
            return;
        }
        let me2 = me.clone();
        confirm(
            me,
            &format!("Announce {ns} on the network?"),
            "Anyone who hears it can then join it and read everything in it. Only you (and those you invite) can write.",
            "Announce",
            move || {
                let ns = ns.clone();
                bg(&me2.daemon, move |d| d.announce(&ns, true), done(&me2))
            },
        );
    }));
    act("leave", Box::new(|me, ns| {
        let me2 = me.clone();
        confirm(
            me,
            &format!("Leave {ns}?"),
            "Its records go from this device. Others who have it keep theirs; you can join it again.",
            "Leave",
            move || {
                let ns = ns.clone();
                bg(&me2.daemon, move |d| d.leave(&ns), done(&me2))
            },
        );
    }));
    act("members", Box::new(|me, ns| open_ns(me, &ns)));
    act("invite", Box::new(|me, ns| invite_dialog(me, &ns)));
}

/// What `Daemon::libraries` says of a library (`access`, `public`, …).
fn info(me: &Rc<App>, ns: &str) -> Option<Value> {
    me.libs.borrow().iter().find(|l| l["name"] == ns).cloned()
}

fn flag(me: &Rc<App>, ns: &str, key: &str) -> bool {
    info(me, ns).is_some_and(|l| l[key].as_bool().unwrap_or(false))
}

fn kept_all(me: &Rc<App>, ns: &str) -> bool {
    info(me, ns).is_some_and(|l| !l["lazy"].as_bool().unwrap_or(false) && l["keep"] == "all")
}

/// Whether this user may invite others to it (its creator, or granted so).
pub fn may_invite(me: &Rc<App>, ns: &str) -> bool {
    info(me, ns).is_some_and(|l| l["access"].as_str().unwrap_or("").contains("invite"))
}

fn done(me: &Rc<App>) -> impl FnOnce(Result<(), String>) + 'static {
    let me = me.clone();
    move |r| match r {
        Ok(()) => reload(&me),
        Err(e) => error(&me, "That didn't work", &e),
    }
}

pub fn error(me: &Rc<App>, heading: &str, body: &str) {
    let d = adw::MessageDialog::builder().heading(heading).body(body).transient_for(&me.window()).build();
    d.add_response("close", "Close");
    d.present();
}

fn confirm(me: &Rc<App>, heading: &str, body: &str, ok: &str, then: impl Fn() + 'static) {
    let d = adw::MessageDialog::builder().heading(heading).body(body).transient_for(&me.window()).default_response("cancel").close_response("cancel").build();
    d.add_responses(&[("cancel", "Cancel"), ("ok", ok)]);
    d.set_response_appearance("ok", adw::ResponseAppearance::Destructive);
    d.connect_response(None, move |_, r| {
        if r == "ok" {
            then();
        }
    });
    d.present();
}

/// The right-click menu of a library in the sidebar.
pub fn menu(me: &Rc<App>, ns: &str) -> gio::Menu {
    let item = |label: &str, action: &str| {
        let i = gio::MenuItem::new(Some(label), None);
        i.set_action_and_target_value(Some(&format!("win.{action}")), Some(&ns.to_variant()));
        i
    };
    let me_ns = ns == "me";
    let own = flag(me, ns, "own");
    let menu = gio::Menu::new();
    let top = gio::Menu::new();
    if !me_ns {
        top.append_item(&item(if flag(me, ns, "favorite") { "Remove from favorites" } else { "Add to favorites" }, "favorite"));
    }
    top.append_item(&item(if kept_all(me, ns) { "Fetch as needed only" } else { "Keep on this device" }, "keep"));
    menu.append_section(None, &top);
    if !me_ns {
        let share = gio::Menu::new();
        share.append_item(&item("Members…", "members"));
        if may_invite(me, ns) {
            share.append_item(&item("Invite…", "invite"));
        }
        share.append_item(&item(if flag(me, ns, "public") { "Copy link" } else { "Copy link (whoever has it can read)" }, "copy-link"));
        if own {
            share.append_item(&item(if flag(me, ns, "announced") { "Stop announcing" } else { "Announce on the network…" }, "announce"));
        }
        menu.append_section(None, &share);
        if !own {
            let leave = gio::Menu::new();
            leave.append_item(&item("Leave library…", "leave"));
            menu.append_section(None, &leave);
        }
    }
    menu
}

/// A library's page: sharing, members, on this device, state.
pub fn page(me: &Rc<App>, ns: &NsView) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let name = ns.name.clone();
    let public = flag(me, &name, "public");
    let own = flag(me, &name, "own");

    let sharing = adw::PreferencesGroup::builder()
        .title("Sharing")
        .description(if public {
            "Public: anyone may join it and read it."
        } else {
            "Private: those it's shared with are its members below. Its link lets anyone who has it read it; an invitation makes a member you can see and remove."
        })
        .build();
    if may_invite(me, &name) {
        let invite = gtk::Button::builder().label("Invite…").valign(gtk::Align::Center).build();
        invite.add_css_class("suggested-action");
        invite.set_action_name(Some("win.invite"));
        invite.set_action_target_value(Some(&name.to_variant()));
        sharing.set_header_suffix(Some(&invite));
    }
    let link = adw::ActionRow::builder().title("Link").subtitle(if public { "Anyone may read it anyway" } else { "Whoever has it can read everything in it" }).build();
    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.set_valign(gtk::Align::Center);
    copy.add_css_class("flat");
    copy.set_tooltip_text(Some("Copy its link"));
    copy.set_action_name(Some("win.copy-link"));
    copy.set_action_target_value(Some(&name.to_variant()));
    link.add_suffix(&copy);
    sharing.add(&link);
    if own && name != "me" {
        let announced = adw::SwitchRow::builder().title("Announce on the network").subtitle("Others hear it and may join it").active(flag(me, &name, "announced")).build();
        let me2 = me.clone();
        let n = name.clone();
        announced.connect_active_notify(move |s| {
            if s.is_active() != flag(&me2, &n, "announced") {
                WidgetExt::activate_action(&me2.window(), "win.announce", Some(&n.to_variant())).ok();
            }
        });
        sharing.add(&announced);
    }
    page.add(&sharing);

    // Members and invitations come from the daemon: filled in when here.
    let members = adw::PreferencesGroup::builder().title("Members").build();
    let asking = adw::ActionRow::builder().title("Asking…").build();
    members.add(&asking);
    page.add(&members);
    let invites = adw::PreferencesGroup::builder().title("Invitations not used yet").visible(false).build();
    page.add(&invites);
    let (me2, n, members2, invites2) = (me.clone(), name.clone(), members.clone(), invites.clone());
    bg(&me.daemon, {
        let n = n.clone();
        move |d| (d.members(&n), d.invites(&n))
    }, move |(m, i)| {
        members2.remove(&asking);
        fill_members(&me2, &n, &members2, &invites2, m, i);
    });

    let device = adw::PreferencesGroup::builder().title("On this device").build();
    let keep = adw::SwitchRow::builder()
        .title("Keep on this device")
        .subtitle("Everything in it here, readable offline; off: fetched as it's looked at")
        .active(!ns.lazy && ns.keep == "all")
        .build();
    {
        let (me2, n, was) = (me.clone(), name.clone(), !ns.lazy && ns.keep == "all");
        keep.connect_active_notify(move |s| {
            if s.is_active() != was {
                WidgetExt::activate_action(&me2.window(), "win.keep", Some(&n.to_variant())).ok();
            }
        });
    }
    device.add(&keep);
    if ns.coming() {
        let here = ns.kept_here.unwrap_or(0);
        let row = adw::ActionRow::builder()
            .title(if ns.syncing && ns.kept == 0 { "Fetching its records…".to_string() } else { format!("Fetching {} of {} files…", here, ns.kept) })
            .build();
        let bar = gtk::ProgressBar::builder().valign(gtk::Align::Center).width_request(200).build();
        if ns.kept > 0 {
            bar.set_fraction(here as f64 / ns.kept as f64);
        } else {
            bar.pulse();
        }
        row.add_suffix(&bar);
        device.add(&row);
    }
    if name != "me" {
        let fav = adw::SwitchRow::builder().title("Favorite").subtitle("Listed first, in every app and on every device of yours").active(flag(me, &name, "favorite")).build();
        let (me2, n) = (me.clone(), name.clone());
        fav.connect_active_notify(move |s| {
            if s.is_active() != flag(&me2, &n, "favorite") {
                WidgetExt::activate_action(&me2.window(), "win.favorite", Some(&n.to_variant())).ok();
            }
        });
        device.add(&fav);
    }
    device.add(&adw::ActionRow::builder().title("Peers").subtitle(format!("{} node(s) this one syncs with", ns.peers)).build());
    page.add(&device);

    let state = adw::PreferencesGroup::builder().title("State").build();
    state.add(&adw::ActionRow::builder().title("Records").subtitle(ns.records.to_string()).build());
    state.add(&adw::ActionRow::builder().title("Device name").subtitle(&ns.device).build());
    page.add(&state);
    if !own && name != "me" {
        let leave = gtk::Button::builder().label("Leave library…").halign(gtk::Align::Start).build();
        leave.add_css_class("destructive-action");
        leave.set_action_name(Some("win.leave"));
        leave.set_action_target_value(Some(&name.to_variant()));
        let g = adw::PreferencesGroup::new();
        g.add(&leave);
        page.add(&g);
    }
    page
}

fn fill_members(me: &Rc<App>, ns: &str, members: &adw::PreferencesGroup, invites: &adw::PreferencesGroup, m: Result<Value, String>, i: Result<Vec<retifs_desktop_core::InviteInfo>, String>) {
    let manage = may_invite(me, ns);
    let list = match m {
        Ok(Value::Array(list)) => list,
        Ok(_) => Vec::new(),
        Err(e) => {
            members.add(&adw::ActionRow::builder().title("Couldn't read them").subtitle(e).build());
            return;
        }
    };
    for m in list {
        let words = m["words"].as_str().unwrap_or("").to_string();
        let name = m["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&words).to_string();
        let mut detail = m["role"].as_str().unwrap_or("").to_string();
        if m["creator"].as_bool().unwrap_or(false) {
            detail.push_str(" · made it");
        }
        for l in m["limits"].as_array().into_iter().flatten() {
            detail.push_str(&format!(" · {}", l.as_str().unwrap_or("")));
        }
        if name != words {
            detail.push_str(&format!(" · {words}"));
        }
        let row = adw::ActionRow::builder().title(&name).subtitle(&detail).build();
        row.set_tooltip_text(m["writer"].as_str());
        if manage && !m["you"].as_bool().unwrap_or(false) && !m["creator"].as_bool().unwrap_or(false) {
            let remove = gtk::Button::builder().label("Remove").valign(gtk::Align::Center).build();
            remove.add_css_class("flat");
            let (me2, ns2, writer, who) = (me.clone(), ns.to_string(), m["writer"].as_str().unwrap_or("").to_string(), name.clone());
            remove.connect_clicked(move |_| {
                let (me3, ns3, writer) = (me2.clone(), ns2.clone(), writer.clone());
                confirm(
                    &me2,
                    &format!("Remove {who} from {ns2}?"),
                    "What they write stops counting, and the library's key changes: they keep what they've read, but can't read what's written from now on.",
                    "Remove",
                    move || {
                        let (ns4, w) = (ns3.clone(), writer.clone());
                        bg(&me3.daemon, move |d| d.remove_member(&ns4, &w), done(&me3));
                    },
                );
            });
            row.add_suffix(&remove);
        }
        members.add(&row);
    }
    let pending: Vec<_> = i.unwrap_or_default().into_iter().filter(|i| !i.spent).collect();
    invites.set_visible(!pending.is_empty());
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    for inv in pending {
        let role = if !inv.write { "read" } else if inv.grant { "read, write and invite" } else { "read and write" };
        let left = if inv.expires < now { "expired".to_string() } else { format!("{} h left", (inv.expires - now) / 3_600_000) };
        let row = adw::ActionRow::builder().title(format!("Invitation {}", &inv.id[..8.min(inv.id.len())])).subtitle(format!("{role} · {left}")).build();
        if manage {
            let withdraw = gtk::Button::builder().label("Withdraw").valign(gtk::Align::Center).build();
            withdraw.add_css_class("flat");
            let (me2, ns2, id) = (me.clone(), ns.to_string(), inv.id.clone());
            withdraw.connect_clicked(move |_| {
                let (ns3, id2) = (ns2.clone(), id.clone());
                bg(&me2.daemon, move |d| d.cancel_invite(&ns3, &id2), done(&me2));
            });
            row.add_suffix(&withdraw);
        }
        invites.add(&row);
    }
}

/// An invitation: who it makes the person, for how long; then its link.
pub fn invite_dialog(me: &Rc<App>, ns: &str) {
    let read = gtk::CheckButton::with_label("Read");
    let write = gtk::CheckButton::with_label("Read and write");
    let grant = gtk::CheckButton::with_label("Read, write and invite others");
    write.set_group(Some(&read));
    grant.set_group(Some(&read));
    read.set_active(true);
    let days = gtk::DropDown::from_strings(&["Usable for 1 day", "Usable for 1 week", "Usable for 30 days"]);
    days.set_selected(1);
    let fields = gtk::Box::new(gtk::Orientation::Vertical, 6);
    for w in [read.upcast_ref::<gtk::Widget>(), write.upcast_ref(), grant.upcast_ref(), days.upcast_ref()] {
        fields.append(w);
    }
    let d = adw::MessageDialog::builder()
        .heading(format!("Invite someone to {ns}"))
        .body("They may:")
        .extra_child(&fields)
        .transient_for(&me.window())
        .default_response("make")
        .close_response("cancel")
        .build();
    d.add_responses(&[("cancel", "Cancel"), ("make", "Make invitation")]);
    d.set_response_appearance("make", adw::ResponseAppearance::Suggested);
    let (me2, ns) = (me.clone(), ns.to_string());
    d.connect_response(None, move |_, r| {
        if r != "make" {
            return;
        }
        let role = if grant.is_active() { "invite" } else if write.is_active() { "write" } else { "read" };
        let ms = [1u64, 7, 30][days.selected().min(2) as usize] * 86_400_000;
        let (me3, ns2) = (me2.clone(), ns.clone());
        bg(&me2.daemon, move |d| d.invite(&ns2, role, ms), move |r| match r {
            Ok(link) => show_link(&me3, &link),
            Err(e) => error(&me3, "Couldn't make the invitation", &e),
        });
    });
    d.present();
}

fn show_link(me: &Rc<App>, link: &str) {
    let entry = gtk::Entry::builder().text(link).editable(false).width_chars(48).build();
    let d = adw::MessageDialog::builder()
        .heading("The invitation")
        .body("Pass it in person or end to end: whoever sees it can read; the first to open it becomes the member.")
        .extra_child(&entry)
        .transient_for(&me.window())
        .build();
    d.add_responses(&[("close", "Close"), ("copy", "Copy")]);
    d.set_response_appearance("copy", adw::ResponseAppearance::Suggested);
    let (me2, link) = (me.clone(), link.to_string());
    d.connect_response(None, move |_, r| {
        if r == "copy" {
            me2.window().clipboard().set_text(&link);
        }
        reload(&me2);
    });
    d.present();
}

/// Announces: the public libraries heard on the network, each with Join.
pub fn announces_page(me: &Rc<App>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder().title("Heard on the network").description("Public libraries others announce. Joining fetches only what you look at.").build();
    page.add(&group);
    *me.announces.borrow_mut() = Some(group);
    page
}

pub fn fill_announces(me: &Rc<App>) {
    let me2 = me.clone();
    bg(&me.daemon, |d| d.heard(), move |r| {
        let Some(group) = me2.announces.borrow().clone() else { return };
        for row in me2.announce_rows.borrow_mut().drain(..) {
            group.remove(&row);
        }
        let list = match r {
            Ok(l) => l,
            Err(e) => {
                let row = adw::ActionRow::builder().title("Couldn't ask retifsd").subtitle(e).build();
                group.add(&row);
                me2.announce_rows.borrow_mut().push(row);
                return;
            }
        };
        if list.is_empty() {
            let row = adw::ActionRow::builder().title("Nothing heard yet").subtitle("Public libraries show up here as their announces arrive.").build();
            group.add(&row);
            me2.announce_rows.borrow_mut().push(row);
        }
        for l in list {
            let has: Vec<String> = l.has.iter().take(2).map(|(t, n)| format!("{n} {}", t.rsplit(['/', '.']).next().unwrap_or(t))).collect();
            let row = adw::ActionRow::builder()
                .title(&l.name)
                .subtitle(format!("{} · {} · {} hop(s) away", l.about, has.join(", "), l.hops))
                .subtitle_lines(2)
                .build();
            match &l.here {
                Some(here) => {
                    let joined = gtk::Label::new(Some(&format!("Joined as {here}")));
                    joined.add_css_class("dim-label");
                    row.add_suffix(&joined);
                }
                None if l.readable => {
                    let join = gtk::Button::builder().label("Join").valign(gtk::Align::Center).build();
                    let me3 = me2.clone();
                    join.connect_clicked(move |b| {
                        b.set_sensitive(false);
                        b.set_label("Joining…");
                        let (me4, l2) = (me3.clone(), l.clone());
                        bg(&me3.daemon, move |d| d.join_heard(&l2), move |r| match r {
                            Ok(name) => {
                                reload(&me4);
                                fill_announces(&me4);
                                open_ns(&me4, &name);
                            }
                            Err(e) => error(&me4, "Couldn't join it", &e),
                        });
                    });
                    row.add_suffix(&join);
                }
                None => {
                    let newer = gtk::Label::new(Some("Needs a newer retifs"));
                    newer.add_css_class("dim-label");
                    row.add_suffix(&newer);
                }
            }
            group.add(&row);
            me2.announce_rows.borrow_mut().push(row);
        }
    });
}
