//! The slim URL bar: one entry between two drag gutters. The whole bar moves the window when dragged. Unfocused, it shows the current URL with
//! the host bright and the rest dimmed; focused, it is plain editable text, all selected.

use bare_core::display;
use gtk4::{self as gtk, gdk, glib, pango, prelude::*};
use std::{cell::Cell, cell::RefCell, rc::Rc};

/// How far (in pixels) a press on the bar must travel before it counts as dragging the window rather
/// than clicking the address.
const DRAG_THRESHOLD: f64 = 5.0;

/// Width of each side gutter. Dragging a gutter moves the window (there is no title bar to grab).
const GUTTER: i32 = 24;

#[derive(Clone)]
pub struct UrlBar {
    pub widget: gtk::Box,
    entry: gtk::Entry,
    badge: gtk::Label,
    current: Rc<RefCell<String>>,
    focused: Rc<Cell<bool>>,
    /// The user has changed the text since the bar gained focus. Until then there is nothing to
    /// protect, so dragging the bar moves the window; after, dragging selects text.
    edited: Rc<Cell<bool>>,
}

impl UrlBar {
    pub fn new() -> Self {
        let entry = gtk::Entry::builder()
            .hexpand(true)
            .has_frame(false)
            .placeholder_text("Search or enter address")
            .input_purpose(gtk::InputPurpose::Url)
            .enable_emoji_completion(false)
            .css_classes(["bare-url"])
            .build();

        // Shows how many tabs are open (nothing while there is only one). There is no tab strip.
        let badge = gtk::Label::builder()
            .css_classes(["bare-badge"])
            .width_chars(3)
            .tooltip_text("Open tabs (click to switch)")
            .visible(false)
            .build();

        let widget = gtk::Box::builder().css_classes(["bare-bar"]).build();
        widget.append(&gutter());
        widget.append(&entry);
        widget.append(&badge);
        widget.append(&gutter());

        let bar = Self {
            widget,
            entry,
            badge,
            current: Rc::default(),
            focused: Rc::default(),
            edited: Rc::default(),
        };
        bar.wire();
        bar
    }

    pub fn entry(&self) -> &gtk::Entry {
        &self.entry
    }

    pub fn badge(&self) -> &gtk::Label {
        &self.badge
    }

    pub fn focused(&self) -> bool {
        self.focused.get()
    }

    pub fn set_tab_count(&self, n: usize) {
        self.badge.set_visible(n > 1);
        self.badge.set_text(&n.to_string());
    }

    /// The page's URL changed. Doesn't clobber what the user is typing.
    pub fn set_url(&self, url: &str) {
        *self.current.borrow_mut() = url.to_string();
        if !self.focused.get() {
            self.show_current();
        }
    }

    /// Focus the bar with everything selected, ready to type over.
    pub fn focus(&self) {
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
    }

    fn show_current(&self) {
        show(&self.entry, &self.current.borrow());
    }

    fn wire(&self) {
        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter(glib::clone!(
            #[weak(rename_to = entry)]
            self.entry,
            #[strong(rename_to = focused)]
            self.focused,
            #[strong(rename_to = edited)]
            self.edited,
            move |_| {
                focused.set(true);
                edited.set(false);
                entry.set_attributes(&pango::AttrList::new());
            }
        ));
        // Typing, pasting or deleting makes the bar "being edited" (see `edited`).
        self.entry.connect_changed(glib::clone!(
            #[strong(rename_to = focused)]
            self.focused,
            #[strong(rename_to = edited)]
            self.edited,
            move |_| {
                if focused.get() {
                    edited.set(true);
                }
            }
        ));
        focus.connect_leave(glib::clone!(
            #[weak(rename_to = entry)]
            self.entry,
            #[strong(rename_to = focused)]
            self.focused,
            #[strong(rename_to = current)]
            self.current,
            #[strong(rename_to = edited)]
            self.edited,
            move |_| {
                focused.set(false);
                edited.set(false);
                // Abandon unsubmitted edits.
                show(&entry, &current.borrow());
            }
        ));
        self.entry.add_controller(focus);

        // Until you have edited the text, the whole bar is the window's title bar: drag it to move the
        // window (a freshly opened Bare has the bar focused, so "focused" must not mean "editing"). A
        // plain click still selects the address, like every browser. Once you have edited, dragging
        // selects text as usual.
        let drag = gtk::GestureDrag::builder()
            .button(gdk::BUTTON_PRIMARY)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        let armed = Rc::new(Cell::new(false)); // this press began while the text was unedited
        let moved = Rc::new(Cell::new(false)); // ...and has become a window move
        drag.connect_drag_begin(glib::clone!(
            #[strong(rename_to = focused)]
            self.focused,
            #[strong(rename_to = edited)]
            self.edited,
            #[strong]
            armed,
            #[strong]
            moved,
            move |gesture, _, _| {
                moved.set(false);
                // Drags move the window unless the user is in the middle of editing the text.
                armed.set(!focused.get() || !edited.get());
                if armed.get() {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                }
            }
        ));
        drag.connect_drag_update(glib::clone!(
            #[weak(rename_to = entry)]
            self.entry,
            #[strong]
            armed,
            #[strong]
            moved,
            move |gesture, dx, dy| {
                if !armed.get() || moved.get() || dx.hypot(dy) < DRAG_THRESHOLD {
                    return;
                }
                // Hand over from where the press began, so the window stays under the pointer even though
                // the pointer has already travelled a few pixels.
                if let Some((x, y)) = gesture.start_point() {
                    moved.set(crate::wm::begin_move(entry.upcast_ref(), gesture, x, y));
                }
            }
        ));
        drag.connect_drag_end(glib::clone!(
            #[weak(rename_to = entry)]
            self.entry,
            move |_, _, _| {
                if armed.replace(false) && !moved.get() {
                    entry.grab_focus();
                    entry.select_region(0, -1);
                }
            }
        ));
        self.entry.add_controller(drag);
    }
}

fn gutter() -> gtk::WindowHandle {
    gtk::WindowHandle::builder().width_request(GUTTER).build()
}

/// Put `url` in the entry: host emphasised, the rest dimmed (or the query, for a search page).
fn show(entry: &gtk::Entry, url: &str) {
    let view = display::bar_view(url);
    entry.set_text(&view.text);

    let attrs = pango::AttrList::new();
    if let Some(host) = view.host {
        let mut dim = pango::AttrInt::new_foreground_alpha(0x7000);
        dim.set_start_index(0);
        dim.set_end_index(view.text.len() as u32);
        attrs.insert(dim);
        let mut bright = pango::AttrInt::new_foreground_alpha(0xFFFF);
        bright.set_start_index(host.start as u32);
        bright.set_end_index(host.end as u32);
        attrs.insert(bright);
    }
    entry.set_attributes(&attrs);
}
