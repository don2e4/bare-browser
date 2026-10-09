//! The tab tree on the left: every tab, one per line, drawn like the `tree` command, with a tab
//! opened from a link in another tab under it. Like the URL bar's dropdown it never takes keyboard
//! focus, so rows are plain boxes. The empty space under them moves the window, as a title bar would.
//!
//! The connectors are `tree`'s (`├──`, `└──`, `│`), but drawn as lines rather than set as text: box-
//! drawing characters only join up when rows are exactly one text line tall, and four monospace
//! characters per level are too wide for a sidebar.

use bare_core::tabs::{Row, TabId};
use gtk4::{self as gtk, gdk, glib, pango, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Switch,
    Close,
    /// Fold or unfold the tabs under this one.
    Fold,
}

type Handler = Rc<dyn Fn(TabId, Action)>;

/// What a row shows for a tab.
pub struct Label {
    pub title: String,
    /// Shown on hover.
    pub url: String,
    /// Unloaded to save memory; it comes back when you switch to it.
    pub discarded: bool,
}

/// Width of one level of the tree, in pixels.
const INDENT: i32 = 16;

pub struct Sidebar {
    pub widget: gtk::Box,
    list: gtk::Box,
    /// Scrolls the list when there are more tabs than fit.
    viewport: gtk::Viewport,
    /// Each tab's row and title label, for updates that don't change the tree.
    rows: RefCell<HashMap<TabId, (gtk::Box, gtk::Label)>>,
    active: Rc<Cell<Option<TabId>>>,
    on_action: Rc<RefCell<Option<Handler>>>,
}

impl Sidebar {
    pub fn new() -> Self {
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["bare-tabs"])
            .build();
        let viewport = gtk::Viewport::builder()
            .child(&list)
            .scroll_to_focus(false)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&viewport)
            .build();
        // Below the last tab: a title bar's worth of nothing (drag to move, double-click to maximize).
        let handle = gtk::WindowHandle::builder().vexpand(true).build();
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["bare-sidebar"])
            .width_request(120)
            .focusable(false)
            .build();
        widget.append(&scroller);
        widget.append(&handle);
        Self {
            widget,
            list,
            viewport,
            rows: RefCell::default(),
            active: Rc::default(),
            on_action: Rc::default(),
        }
    }

    /// Called when a row is clicked: switch on a click, close on a middle-click, fold on a double-click.
    pub fn connect_action(&self, f: impl Fn(TabId, Action) + 'static) {
        *self.on_action.borrow_mut() = Some(Rc::new(f));
    }

    /// Redraw the whole tree; `label` says what each tab's row shows.
    pub fn set(&self, rows: &[Row], label: impl Fn(TabId) -> Label, active: Option<TabId>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut built = HashMap::with_capacity(rows.len());
        for r in rows {
            let (row, title_label) = self.build_row(r);
            fill(&row, &title_label, &label(r.id));
            if Some(r.id) == active {
                row.add_css_class("active");
            }
            self.list.append(&row);
            built.insert(r.id, (row, title_label));
        }
        *self.rows.borrow_mut() = built;
        self.active.set(active);
        if let Some(id) = active {
            self.reveal(id);
        }
    }

    /// A tab's title, address or loaded state changed (the tree didn't).
    pub fn update(&self, id: TabId, label: &Label) {
        if let Some((row, title)) = self.rows.borrow().get(&id) {
            fill(row, title, label);
        }
    }

    pub fn set_active(&self, id: TabId) {
        let rows = self.rows.borrow();
        if let Some((row, _)) = self.active.get().and_then(|old| rows.get(&old)) {
            row.remove_css_class("active");
        }
        if let Some((row, _)) = rows.get(&id) {
            row.add_css_class("active");
        }
        self.active.set(Some(id));
        drop(rows);
        self.reveal(id);
    }

    /// Scroll the list so that the active tab's row is in view (after F1 brings the sidebar back).
    pub fn reveal_active(&self) {
        if let Some(id) = self.active.get() {
            self.reveal(id);
        }
    }

    /// Scroll the list so that tab `id`'s row is in view.
    fn reveal(&self, id: TabId) {
        let Some(row) = self.rows.borrow().get(&id).map(|(row, _)| row.clone()) else {
            return;
        };
        if !self.widget.is_visible() {
            return;
        }
        if row.height() > 0 {
            self.viewport.scroll_to(&row, None);
            return;
        }
        // A row built just now has no place yet, and GTK skips scrolling to it: wait for the frame
        // that lays it out. By then another tab may be the active one; then it's that one's turn.
        let (viewport, active) = (self.viewport.clone(), self.active.clone());
        row.add_tick_callback(move |row, _| {
            if row.height() == 0 && active.get() == Some(id) {
                return glib::ControlFlow::Continue;
            }
            if active.get() == Some(id) {
                viewport.scroll_to(row, None);
            }
            glib::ControlFlow::Break
        });
    }

    fn build_row(&self, r: &Row) -> (gtk::Box, gtk::Label) {
        let row = gtk::Box::builder().css_classes(["bare-tab"]).build();
        if !r.prefix.is_empty() {
            row.append(&connectors(&r.prefix));
        }
        let label = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(pango::EllipsizeMode::End)
            .single_line_mode(true)
            .build();
        row.append(&label);
        if r.hidden > 0 {
            let folded = gtk::Label::builder()
                .label(format!("+{}", r.hidden))
                .tooltip_text("Show the tabs under this one")
                .css_classes(["bare-folded"])
                .build();
            // Unfolds with a single click, and the row underneath doesn't also see it.
            let click = gtk::GestureClick::builder()
                .button(gdk::BUTTON_PRIMARY)
                .build();
            let unfold = self.dispatch(r.id, |_, _| Some(Action::Fold));
            click.connect_pressed(move |g, presses, x, y| {
                g.set_state(gtk::EventSequenceState::Claimed);
                unfold(g, presses, x, y);
            });
            folded.add_controller(click);
            row.append(&folded);
        }
        let has_children = r.has_children;
        let click = gtk::GestureClick::builder().button(0).build();
        click.connect_pressed(self.dispatch(r.id, move |button, presses| {
            match (button, presses) {
                (gdk::BUTTON_PRIMARY, 2) if has_children => Some(Action::Fold),
                (gdk::BUTTON_PRIMARY, 1) => Some(Action::Switch),
                (gdk::BUTTON_MIDDLE, 1) => Some(Action::Close),
                _ => None,
            }
        }));
        row.add_controller(click);
        (row, label)
    }

    /// A press handler that turns `(button, presses)` into an action on tab `id`. The action runs
    /// once the press has been handled: it usually rebuilds the list, row included.
    fn dispatch(
        &self,
        id: TabId,
        decide: impl Fn(u32, i32) -> Option<Action> + 'static,
    ) -> impl Fn(&gtk::GestureClick, i32, f64, f64) + 'static {
        let on_action = self.on_action.clone();
        move |g, presses, _, _| {
            let Some(action) = decide(g.current_button(), presses) else {
                return;
            };
            let on_action = on_action.clone();
            glib::idle_add_local_once(move || {
                if let Some(f) = on_action.borrow().clone() {
                    f(id, action);
                }
            });
        }
    }
}

fn fill(row: &gtk::Box, title: &gtk::Label, label: &Label) {
    title.set_text(&label.title);
    row.set_tooltip_text(
        Some(&label.url)
            .filter(|u| !u.is_empty())
            .map(String::as_str),
    );
    if label.discarded {
        row.add_css_class("discarded");
    } else {
        row.remove_css_class("discarded");
    }
}

/// The lines in front of a tab's title, from its `tree` prefix: one [`INDENT`]-wide column per level,
/// each `│` (an ancestor with more siblings to come), blank, `├──` or `└──`.
fn connectors(prefix: &str) -> gtk::DrawingArea {
    let columns: Vec<char> = prefix.chars().step_by(4).collect();
    let area = gtk::DrawingArea::builder()
        .content_width(INDENT * columns.len() as i32)
        .css_classes(["bare-tree"])
        .build();
    area.set_draw_func(move |area, cr, _, height| {
        let c = area.color();
        cr.set_source_rgba(
            c.red().into(),
            c.green().into(),
            c.blue().into(),
            f64::from(c.alpha()) * 0.5,
        );
        cr.set_line_width(1.0);
        // Half-pixel offsets keep one-pixel lines sharp.
        let (bottom, middle) = (f64::from(height), f64::from(height / 2) + 0.5);
        // As in `tree`, a tab's line drops from under the first letter of its parent's title.
        for (i, &column) in columns.iter().enumerate() {
            let x = f64::from(i as i32 * INDENT + 4) + 0.5;
            let arm = f64::from((i as i32 + 1) * INDENT - 3);
            match column {
                '│' => {
                    cr.move_to(x, 0.0);
                    cr.line_to(x, bottom);
                }
                '├' => {
                    cr.move_to(x, 0.0);
                    cr.line_to(x, bottom);
                    cr.move_to(x, middle);
                    cr.line_to(arm, middle);
                }
                '└' => {
                    cr.move_to(x, 0.0);
                    cr.line_to(x, middle);
                    cr.line_to(arm, middle);
                }
                _ => {}
            }
        }
        let _ = cr.stroke();
    });
    area
}
