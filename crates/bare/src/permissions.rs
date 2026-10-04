//! A slim bar under the URL bar asking whether a page may use your location. Everything else a page
//! can ask for (notifications, camera, microphone, clipboard…) is refused without asking: Bare has
//! no notification system or media stack to hand them to. Allowing is for this visit only.

use gtk4::{self as gtk, prelude::*};

pub struct PermissionBar {
    pub widget: gtk::Revealer,
    label: gtk::Label,
    pub allow: gtk::Button,
    pub block: gtk::Button,
}

impl PermissionBar {
    pub fn new() -> Self {
        let label = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let allow = gtk::Button::builder()
            .label("Allow this time")
            .css_classes(["bare-permission-button"])
            .build();
        let block = gtk::Button::builder()
            .label("Block")
            .css_classes(["bare-permission-button"])
            .build();
        let row = gtk::Box::builder()
            .spacing(8)
            .css_classes(["bare-permission"])
            .build();
        row.append(&label);
        row.append(&allow);
        row.append(&block);
        let widget = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(100)
            .child(&row)
            .build();
        Self {
            widget,
            label,
            allow,
            block,
        }
    }

    /// Show the question and put focus on Block, so an accidental Enter refuses rather than allows.
    pub fn ask(&self, text: &str) {
        self.label.set_text(text);
        self.widget.set_reveal_child(true);
        self.block.grab_focus();
    }

    pub fn hide(&self) {
        self.widget.set_reveal_child(false);
    }
}
