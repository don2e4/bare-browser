//! Find in page: a small box in the page's top-right corner. Typing searches live; Enter and
//! Shift+Enter step through matches; Esc closes it. The matching itself is WebKit's FindController.

use gtk4::{self as gtk, prelude::*};

pub struct FindBar {
    pub widget: gtk::Box,
    entry: gtk::Entry,
    count: gtk::Label,
    pub next: gtk::Button,
    pub previous: gtk::Button,
}

impl FindBar {
    pub fn new() -> Self {
        let entry = gtk::Entry::builder()
            .placeholder_text("Find in page")
            .width_chars(22)
            .css_classes(["bare-find-entry"])
            .build();
        let count = gtk::Label::builder()
            .width_chars(10)
            .xalign(1.0)
            .css_classes(["bare-find-count"])
            .build();
        let previous = gtk::Button::builder()
            .label("\u{2191}")
            .css_classes(["flat", "bare-find-step"])
            .tooltip_text("Previous match (Shift+Enter)")
            .focusable(false)
            .build();
        let next = gtk::Button::builder()
            .label("\u{2193}")
            .css_classes(["flat", "bare-find-step"])
            .tooltip_text("Next match (Enter)")
            .focusable(false)
            .build();
        let widget = gtk::Box::builder()
            .spacing(4)
            .css_classes(["bare-find"])
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .visible(false)
            .build();
        for w in [
            entry.upcast_ref::<gtk::Widget>(),
            count.upcast_ref(),
            previous.upcast_ref(),
            next.upcast_ref(),
        ] {
            widget.append(w);
        }
        Self {
            widget,
            entry,
            count,
            next,
            previous,
        }
    }

    pub fn entry(&self) -> &gtk::Entry {
        &self.entry
    }

    pub fn text(&self) -> String {
        self.entry.text().to_string()
    }

    pub fn is_open(&self) -> bool {
        self.widget.is_visible()
    }

    pub fn open(&self) {
        self.widget.set_visible(true);
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
    }

    pub fn close(&self) {
        self.widget.set_visible(false);
        self.set_count(None);
    }

    /// `None` clears the count; `Some(0)` says nothing matched.
    pub fn set_count(&self, found: Option<u32>) {
        match found {
            None => {
                self.count.set_text("");
                self.count.remove_css_class("nomatch");
            }
            Some(0) => {
                self.count.set_text("No matches");
                self.count.add_css_class("nomatch");
            }
            Some(1) => {
                self.count.set_text("1 match");
                self.count.remove_css_class("nomatch");
            }
            Some(n) => {
                self.count.set_text(&format!("{n} matches"));
                self.count.remove_css_class("nomatch");
            }
        }
    }
}
