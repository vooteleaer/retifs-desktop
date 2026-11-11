//! The recovery kit (retifs ADR-0033): making one, and recovering from one
//! on a new device.

use std::sync::Arc;

use adw::prelude::*;
use retifs_desktop_core::{check_recovery_words, Daemon, RecoveryKit, RECOVERY_ABOUT};

use crate::bg;

/// The device page's group: make a kit, or recover from one.
pub fn group(daemon: Arc<Daemon>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Recovery kit")
        .description("If every device of yours is lost, 24 words bring your libraries back on a new one.")
        .build();
    let make = gtk::Button::builder().label("Make…").valign(gtk::Align::Center).build();
    let make_row = adw::ActionRow::builder().title("Make a recovery kit").subtitle("24 words to write down on paper, once").build();
    make_row.add_suffix(&make);
    group.add(&make_row);
    let recover = gtk::Button::builder().label("Recover…").valign(gtk::Align::Center).build();
    let recover_row = adw::ActionRow::builder().title("Recover from a kit").subtitle("On a new device: type your 24 words").build();
    recover_row.add_suffix(&recover);
    group.add(&recover_row);
    let d = daemon.clone();
    make.connect_clicked(move |b| make_kit(b, d.clone()));
    recover.connect_clicked(move |b| recover_dialog(b, daemon.clone()));
    group
}

fn window_of(w: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    w.root().and_downcast::<gtk::Window>()
}

fn message(window: Option<&gtk::Window>, heading: &str, body: &str) {
    let d = adw::MessageDialog::builder().heading(heading).body(body).build();
    d.set_transient_for(window);
    d.add_response("close", "Close");
    d.present();
}

fn make_kit(anchor: &gtk::Button, daemon: Arc<Daemon>) {
    let window = window_of(anchor);
    let ask = adw::MessageDialog::builder().heading("Make a recovery kit?").body(RECOVERY_ABOUT).build();
    ask.set_transient_for(window.as_ref());
    ask.add_responses(&[("cancel", "Cancel"), ("make", "Make kit")]);
    ask.set_response_appearance("make", adw::ResponseAppearance::Suggested);
    ask.connect_response(None, move |_, r| {
        if r != "make" {
            return;
        }
        let window = window.clone();
        bg(&daemon, |d| d.recovery_create(), move |res| match res {
            Ok(kit) => show_words(window.as_ref(), &kit),
            Err(e) => message(window.as_ref(), "No kit made", &e),
        });
    });
    ask.present();
}

/// The words, numbered in columns as they're written down, until the user
/// says they have them: they aren't shown again.
fn show_words(window: Option<&gtk::Window>, kit: &RecoveryKit) {
    let grid = gtk::Grid::builder().row_spacing(6).column_spacing(24).halign(gtk::Align::Center).build();
    for (i, w) in kit.words.iter().enumerate() {
        let label = gtk::Label::builder().label(format!("{:>2}. {w}", i + 1)).xalign(0.0).selectable(false).build();
        label.add_css_class("monospace");
        grid.attach(&label, (i / 6) as i32, (i % 6) as i32, 1, 1);
    }
    let covers = if kit.not_covered.is_empty() {
        format!("It covers all {} of your libraries, and those you make or join later.", kit.covered.len())
    } else {
        format!(
            "It covers {}. Not covered (no device of yours may invite there): {}.",
            kit.covered.join(", "),
            kit.not_covered.join(", ")
        )
    };
    let covers = gtk::Label::builder().label(covers).wrap(true).xalign(0.0).build();
    covers.add_css_class("dim-label");
    let written = gtk::CheckButton::with_label("I have written the 24 words down, in order");
    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).build();
    content.append(&grid);
    content.append(&covers);
    content.append(&written);
    let d = adw::MessageDialog::builder()
        .heading("Your recovery kit")
        .body("Write these words down on paper, in order, and keep the paper somewhere safe. They won't be shown again.")
        .extra_child(&content)
        .close_response("done")
        .build();
    d.set_transient_for(window);
    d.add_response("done", "Done");
    d.set_response_enabled("done", false);
    d.set_response_appearance("done", adw::ResponseAppearance::Suggested);
    let dd = d.clone();
    written.connect_toggled(move |c| dd.set_response_enabled("done", c.is_active()));
    d.present();
}

fn recover_dialog(anchor: &gtk::Button, daemon: Arc<Daemon>) {
    let window = window_of(anchor);
    let text = gtk::TextView::builder().wrap_mode(gtk::WrapMode::Word).monospace(true).height_request(120).build();
    let scroll = gtk::ScrolledWindow::builder().child(&text).min_content_width(420).build();
    let say = gtk::Label::builder().label(format!("0 of {} words", retifs_desktop_core::RECOVERY_WORDS)).xalign(0.0).build();
    say.add_css_class("dim-label");
    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
    content.append(&scroll);
    content.append(&say);
    let d = adw::MessageDialog::builder()
        .heading("Recover from a kit")
        .body("Type your 24 words, in order. A device of yours that keeps your keyring must be running somewhere on the network.")
        .extra_child(&content)
        .build();
    d.set_transient_for(window.as_ref());
    d.add_responses(&[("cancel", "Cancel"), ("recover", "Recover")]);
    d.set_response_enabled("recover", false);
    d.set_response_appearance("recover", adw::ResponseAppearance::Suggested);
    let (dd, say2) = (d.clone(), say.clone());
    text.buffer().connect_changed(move |b| {
        let typed = b.text(&b.start_iter(), &b.end_iter(), false);
        match check_recovery_words(&typed) {
            Ok(_) => {
                say2.set_label("The words check out.");
                dd.set_response_enabled("recover", true);
            }
            Err(e) => {
                say2.set_label(&e);
                dd.set_response_enabled("recover", false);
            }
        }
    });
    d.connect_response(None, move |_, r| {
        if r != "recover" {
            return;
        }
        let b = text.buffer();
        let typed = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
        let Ok(words) = check_recovery_words(&typed) else { return };
        // It takes minutes: say what's going on, so it never looks stuck.
        let spinner = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).build();
        let busy = adw::MessageDialog::builder()
            .heading("Recovering…")
            .body("Looking for a device of yours that keeps your keyring, then bringing each library back. This can take several minutes.")
            .extra_child(&spinner)
            .build();
        busy.set_transient_for(window.as_ref());
        busy.present();
        let window = window.clone();
        bg(&daemon, move |d| d.recovery_restore(&words), move |res| {
            busy.close();
            match res {
                Ok(libs) if libs.is_empty() => message(window.as_ref(), "Nothing to bring back", "The keyring lists no library this device doesn't have."),
                Ok(libs) => message(
                    window.as_ref(),
                    "Your libraries are back",
                    &format!(
                        "Back on this device: {}. Their content arrives in the background. In each library, remove the devices you lost (Members…).",
                        libs.join(", ")
                    ),
                ),
                Err(e) => message(window.as_ref(), "Not recovered", &e),
            }
        });
    });
    d.present();
}
