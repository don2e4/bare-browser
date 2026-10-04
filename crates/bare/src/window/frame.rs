//! The frameless window itself: resize edges, drag-to-move, keyboard shortcuts, and the
//! window-level event wiring.

use super::*;
use crate::wm::surface_point;

const CSS: &str = include_str!("../../../../resources/style.css");

/// Thickness of the invisible resize strips along the window edges, and of the corner squares.
const EDGE: i32 = 4;
const CORNER: i32 = 12;
impl Inner {
    pub(super) fn wire_window(
        self: &Rc<Self>,
        handles: &Rc<Vec<gtk::Widget>>,
        page: &gtk::Overlay,
    ) {
        let auto_hide = self.config.chrome == Chrome::Hidden;
        self.window.connect_fullscreened_notify({
            let revealer = self.revealer.clone();
            let handles = handles.clone();
            move |w| {
                revealer.set_reveal_child(!w.is_fullscreen() && !auto_hide);
                set_handles_visible(&handles, w);
            }
        });
        self.window.connect_maximized_notify({
            let handles = handles.clone();
            move |w| set_handles_visible(&handles, w)
        });
        let state_file = self.paths.window_file();
        self.window.connect_close_request(move |w| {
            save_state(w, &state_file);
            glib::Propagation::Proceed
        });

        // Mouse back/forward buttons.
        let nav = gtk::GestureClick::builder()
            .button(0)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        let weak = Rc::downgrade(self);
        nav.connect_pressed(move |gesture, _, _, _| {
            let Some(view) = weak.upgrade().and_then(|s| s.active_view()) else {
                return;
            };
            match gesture.current_button() {
                8 => view.go_back(),
                9 => view.go_forward(),
                _ => return,
            }
            // Don't also hand the press to the page (it would see a stray "button 4/5" mouse event).
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        page.add_controller(nav);
    }

    pub(super) fn wire_shortcuts(self: &Rc<Self>) {
        let c = gtk::ShortcutController::new();
        c.set_propagation_phase(gtk::PropagationPhase::Capture);
        let on = |accels: &str, f: fn(&Rc<Inner>)| key(&c, self, accels, f);

        on("<Control>l|<Control>k|F6", |s| s.focus_bar());
        on("<Control>t", |s| {
            s.open_tab(None, s.active_id(), true);
            if s.config.chrome == Chrome::Bar {
                s.focus_bar();
            }
        });
        on("<Control>w", |s| {
            if let Some(id) = s.active_id() {
                s.close_tab(id);
            }
        });
        on("<Control><Shift>t", |s| s.reopen_closed());
        on("<Control>q", |s| s.window.close());
        on("<Control>d", |s| s.toggle_bookmark());
        on("<Control>f", |s| s.open_find());
        on("F3|<Control>g", |s| s.find_step(true));
        on("<Shift>F3|<Control><Shift>g", |s| s.find_step(false));

        on("<Control>Tab|<Control>Page_Down", |s| s.step_tab(1));
        on(
            "<Control><Shift>Tab|<Control><Shift>ISO_Left_Tab|<Control>ISO_Left_Tab|<Control>Page_Up",
            |s| s.step_tab(-1),
        );
        on("<Alt>1", |s| s.jump_to(0));
        on("<Alt>2", |s| s.jump_to(1));
        on("<Alt>3", |s| s.jump_to(2));
        on("<Alt>4", |s| s.jump_to(3));
        on("<Alt>5", |s| s.jump_to(4));
        on("<Alt>6", |s| s.jump_to(5));
        on("<Alt>7", |s| s.jump_to(6));
        on("<Alt>8", |s| s.jump_to(7));
        on("<Alt>9", |s| {
            let last = s.order.borrow().last();
            if let Some(id) = last {
                s.switch_to(id);
            }
        });

        on("<Alt>Left", |s| {
            if let Some(v) = s.active_view() {
                v.go_back()
            }
        });
        on("<Alt>Right", |s| {
            if let Some(v) = s.active_view() {
                v.go_forward()
            }
        });
        on("<Control>r|F5", |s| {
            if let Some(v) = s.active_view() {
                v.reload()
            }
        });
        on("<Control><Shift>r|<Shift>F5", |s| {
            if let Some(v) = s.active_view() {
                v.reload_bypass_cache()
            }
        });
        on("<Control>plus|<Control>equal|<Control>KP_Add", |s| {
            if let Some(v) = s.active_view() {
                zoom(&v, 0.1)
            }
        });
        on("<Control>minus|<Control>KP_Subtract", |s| {
            if let Some(v) = s.active_view() {
                zoom(&v, -0.1)
            }
        });
        on("<Control>0|<Control>KP_0", |s| {
            if let Some(v) = s.active_view() {
                v.set_zoom_level(1.0)
            }
        });
        on("F11", |s| {
            s.window.set_fullscreened(!s.window.is_fullscreen())
        });
        self.window.add_controller(c);
    }

    pub(super) fn step_tab(self: &Rc<Self>, delta: i32) {
        let Some(active) = self.active_id() else {
            return;
        };
        let target = {
            let order = self.order.borrow();
            if delta > 0 {
                order.next(active)
            } else {
                order.prev(active)
            }
        };
        if let Some(id) = target {
            self.switch_to(id);
        }
    }

    pub(super) fn jump_to(self: &Rc<Self>, n: usize) {
        let id = self.order.borrow().nth(n);
        if let Some(id) = id {
            self.switch_to(id);
        }
    }
}

/// Bind accelerators (`a|b` alternatives) to an action on the browser.
pub(super) fn key(c: &gtk::ShortcutController, inner: &Rc<Inner>, accels: &str, f: fn(&Rc<Inner>)) {
    let weak = Rc::downgrade(inner);
    add_shortcut(c, accels, move || {
        if let Some(s) = weak.upgrade() {
            f(&s);
        }
    });
}

pub(super) fn zoom(web: &WebView, delta: f64) {
    web.set_zoom_level((web.zoom_level() + delta).clamp(0.3, 5.0));
}

pub(super) fn save_state(window: &gtk::ApplicationWindow, file: &Path) {
    // GTK4 keeps default-size in sync with the user's resizing, and leaves it alone while maximized.
    let (width, height) = window.default_size();
    WindowState {
        width,
        height,
        maximized: window.is_maximized(),
    }
    .save(file);
}

pub(super) fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

pub(super) fn add_shortcut(
    controller: &gtk::ShortcutController,
    accels: &str,
    action: impl Fn() + 'static,
) {
    let trigger = gtk::ShortcutTrigger::parse_string(accels).expect("valid accelerator string");
    let action = gtk::CallbackAction::new(move |_, _| {
        action();
        glib::Propagation::Stop
    });
    controller.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
}

// ---- frameless-window plumbing ---------------------------------------------------------------

/// Invisible strips along the edges and corners of the window that start an interactive resize.
pub(super) fn add_resize_handles(root: &gtk::Overlay) -> Rc<Vec<gtk::Widget>> {
    use gdk::SurfaceEdge as E;
    use gtk::Align::{End, Fill, Start};
    // (halign, valign, width, height, edge, cursor). Edges first so corners sit on top of them.
    let specs = [
        (Fill, Start, -1, EDGE, E::North, "n-resize"),
        (Fill, End, -1, EDGE, E::South, "s-resize"),
        (Start, Fill, EDGE, -1, E::West, "w-resize"),
        (End, Fill, EDGE, -1, E::East, "e-resize"),
        (Start, Start, CORNER, CORNER, E::NorthWest, "nw-resize"),
        (End, Start, CORNER, CORNER, E::NorthEast, "ne-resize"),
        (Start, End, CORNER, CORNER, E::SouthWest, "sw-resize"),
        (End, End, CORNER, CORNER, E::SouthEast, "se-resize"),
    ];
    let mut handles = Vec::new();
    for (halign, valign, width, height, edge, cursor) in specs {
        let strip = gtk::Box::builder()
            .halign(halign)
            .valign(valign)
            .width_request(width)
            .height_request(height)
            .build();
        strip.set_cursor_from_name(Some(cursor));

        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        click.connect_pressed(glib::clone!(
            #[weak]
            strip,
            move |gesture, _, x, y| {
                let Some((toplevel, sx, sy)) = surface_point(strip.upcast_ref(), x, y) else {
                    return;
                };
                toplevel.begin_resize(
                    edge,
                    gesture.current_event_device().as_ref(),
                    gdk::BUTTON_PRIMARY as i32,
                    sx,
                    sy,
                    gesture.current_event_time(),
                );
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        ));
        strip.add_controller(click);

        root.add_overlay(&strip);
        root.set_measure_overlay(&strip, false);
        handles.push(strip.upcast::<gtk::Widget>());
    }
    Rc::new(handles)
}

pub(super) fn set_handles_visible(handles: &[gtk::Widget], window: &gtk::ApplicationWindow) {
    let visible = !(window.is_maximized() || window.is_fullscreen());
    for h in handles {
        h.set_visible(visible);
    }
}

/// Alt + left-drag anywhere moves the window, whatever the window manager does (Openbox has this
/// built in; GNOME wants Super; some tiling WMs nothing).
pub(super) fn add_alt_drag_move(root: &gtk::Overlay) {
    let click = gtk::GestureClick::builder()
        .button(gdk::BUTTON_PRIMARY)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    click.connect_pressed(glib::clone!(
        #[weak]
        root,
        move |gesture, _, x, y| {
            if !gesture
                .current_event_state()
                .contains(gdk::ModifierType::ALT_MASK)
            {
                return;
            }
            let Some(device) = gesture.current_event_device() else {
                return;
            };
            let Some((toplevel, sx, sy)) = surface_point(root.upcast_ref(), x, y) else {
                return;
            };
            toplevel.begin_move(
                &device,
                gdk::BUTTON_PRIMARY as i32,
                sx,
                sy,
                gesture.current_event_time(),
            );
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
    ));
    root.add_controller(click);
}
