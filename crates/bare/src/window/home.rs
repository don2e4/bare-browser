//! The new-tab page, drawn natively.

use super::*;

/// The new-tab page: a wordmark, a tagline, and (optionally) one quiet line of key hints.
pub(super) fn build_home(config: &Config) -> gtk::WindowHandle {
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    column.append(
        &gtk::Label::builder()
            .label("bare")
            .css_classes(["bare-home-title"])
            .build(),
    );
    column.append(
        &gtk::Label::builder()
            .label("just the page.")
            .css_classes(["bare-home-tag"])
            .build(),
    );
    if config.home_hint {
        column.append(
            &gtk::Label::builder()
                .label("Ctrl+L search or address  \u{b7}  Ctrl+T new tab  \u{b7}  Ctrl+W close  \u{b7}  F11 fullscreen")
                .css_classes(["bare-home-hint"])
                .build(),
        );
    }
    let page = gtk::Box::builder()
        .css_classes(["bare-home-page"])
        .hexpand(true)
        .vexpand(true)
        .build();
    page.append(&column);
    page.set_halign(gtk::Align::Fill);
    column.set_hexpand(true);
    // Nothing on this page needs the mouse, so all of it moves the window (a title bar's job).
    gtk::WindowHandle::builder()
        .child(&page)
        .hexpand(true)
        .vexpand(true)
        .build()
}
