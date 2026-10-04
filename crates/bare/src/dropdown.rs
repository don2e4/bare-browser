//! The suggestion list under the URL bar. It never takes keyboard focus: the entry keeps it, and the
//! browser feeds Up/Down/Enter in. Rows are plain boxes, not a ListBox, for exactly that reason.

use bare_core::suggest::{Kind, Suggestion};
use gtk4::{self as gtk, gdk, glib, pango, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

type Activate = Rc<dyn Fn(Suggestion, bool)>;

pub struct Dropdown {
    pub widget: gtk::Box,
    rows: RefCell<Vec<(gtk::Box, Suggestion)>>,
    /// The highlighted row. With nothing typed there is no typed row, so nothing is highlighted until
    /// the user presses Down.
    selected: Cell<Option<usize>>,
    /// Has the user moved the selection (arrow keys)? If not, Enter means "what I typed".
    navigated: Cell<bool>,
    on_activate: Rc<RefCell<Option<Activate>>>,
}

impl Dropdown {
    pub fn new() -> Self {
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["bare-dropdown"])
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .width_request(720)
            .focusable(false)
            .visible(false)
            .build();
        Self {
            widget,
            rows: RefCell::default(),
            selected: Cell::new(None),
            navigated: Cell::new(false),
            on_activate: Rc::default(),
        }
    }

    /// Called when a row is clicked: `(suggestion, open in a new tab)`.
    pub fn connect_activate(&self, f: impl Fn(Suggestion, bool) + 'static) {
        *self.on_activate.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set(&self, items: Vec<Suggestion>) {
        while let Some(child) = self.widget.first_child() {
            self.widget.remove(&child);
        }
        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let row = build_row(&item);
            let click = gtk::GestureClick::builder()
                .button(gdk::BUTTON_PRIMARY)
                .build();
            click.connect_pressed(glib::clone!(
                #[strong(rename_to = cb)]
                self.on_activate,
                #[strong]
                item,
                move |g, _, _, _| {
                    let alt = g
                        .current_event_state()
                        .contains(gdk::ModifierType::ALT_MASK);
                    if let Some(f) = cb.borrow().clone() {
                        f(item.clone(), alt);
                    }
                }
            ));
            row.add_controller(click);
            self.widget.append(&row);
            rows.push((row, item));
        }
        let typed_row = matches!(
            rows.first().map(|(_, s): &(gtk::Box, Suggestion)| &s.kind),
            Some(Kind::Go | Kind::Search)
        );
        self.selected.set(typed_row.then_some(0));
        self.navigated.set(false);
        let any = !rows.is_empty();
        *self.rows.borrow_mut() = rows;
        self.paint();
        self.widget.set_visible(any);
        // Rows were swapped while the card may have been hidden; make sure it is measured afresh,
        // or it keeps the height of whatever it last showed.
        self.widget.queue_resize();
    }

    pub fn hide(&self) {
        self.widget.set_visible(false);
    }

    pub fn is_open(&self) -> bool {
        self.widget.is_visible()
    }

    pub fn move_selection(&self, delta: i32) {
        let n = self.rows.borrow().len() as i32;
        if n == 0 || !self.is_open() {
            return;
        }
        self.navigated.set(true);
        let next = match (self.selected.get(), delta > 0) {
            (None, true) => 0,
            (None, false) => (n - 1) as usize,
            (Some(i), _) => (i as i32 + delta).rem_euclid(n) as usize,
        };
        self.selected.set(Some(next));
        self.paint();
    }

    /// The highlighted suggestion, but only if the user moved to it.
    pub fn chosen(&self) -> Option<Suggestion> {
        if !self.navigated.get() || !self.is_open() {
            return None;
        }
        let i = self.selected.get()?;
        self.rows.borrow().get(i).map(|(_, s)| s.clone())
    }

    fn paint(&self) {
        for (i, (row, _)) in self.rows.borrow().iter().enumerate() {
            if Some(i) == self.selected.get() {
                row.add_css_class("selected");
            } else {
                row.remove_css_class("selected");
            }
        }
    }
}

fn build_row(s: &Suggestion) -> gtk::Box {
    let (label, show_url) = match s.kind {
        Kind::Go => ("go", false),
        Kind::Search => ("search", false),
        Kind::Tab(_) => ("tab", true),
        Kind::Bookmark => ("\u{2605}", true),
        Kind::History => ("history", true),
    };
    let row = gtk::Box::builder()
        .spacing(12)
        .css_classes(["bare-row"])
        .build();
    row.append(
        &gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .width_request(52)
            .css_classes(["bare-kind"])
            .build(),
    );
    let title = if s.title.is_empty() {
        pretty_url(&s.url)
    } else {
        s.title.clone()
    };
    row.append(
        &gtk::Label::builder()
            .label(&title)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(pango::EllipsizeMode::End)
            .build(),
    );
    // A history entry for a search has the query as its title and nothing useful as an address.
    if show_url && !s.url.starts_with("bare://") && title != pretty_url(&s.url) {
        row.append(
            &gtk::Label::builder()
                .label(pretty_url(&s.url))
                .xalign(1.0)
                .max_width_chars(38)
                .ellipsize(pango::EllipsizeMode::Middle)
                .css_classes(["bare-dim"])
                .build(),
        );
    }
    row
}

fn pretty_url(url: &str) -> String {
    let u = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    u.trim_end_matches('/').to_string()
}
