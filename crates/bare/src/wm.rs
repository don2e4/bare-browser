//! Asking the window manager to move or resize the frameless window. There is no title bar to grab,
//! so the pieces of Bare that act as one (the URL bar, the empty new-tab page, the window edges) hand
//! a drag over to the window manager with these.

use gtk4::{self as gtk, gdk, graphene, prelude::*};

/// Translate widget-relative coordinates to the toplevel surface, as `begin_move`/`begin_resize` want.
pub(crate) fn surface_point(
    widget: &gtk::Widget,
    x: f64,
    y: f64,
) -> Option<(gdk::Toplevel, f64, f64)> {
    let native = widget.native()?;
    let toplevel = native.surface()?.downcast::<gdk::Toplevel>().ok()?;
    let p = widget.compute_point(
        native.upcast_ref::<gtk::Widget>(),
        &graphene::Point::new(x as f32, y as f32),
    )?;
    let (tx, ty) = native.surface_transform();
    Some((toplevel, f64::from(p.x()) + tx, f64::from(p.y()) + ty))
}

/// Start moving the window, as if its title bar had been grabbed at `(x, y)` (widget coordinates).
/// Returns whether the window manager was asked.
pub(crate) fn begin_move(
    widget: &gtk::Widget,
    gesture: &impl IsA<gtk::EventController>,
    x: f64,
    y: f64,
) -> bool {
    let (Some(device), Some((toplevel, sx, sy))) =
        (gesture.current_event_device(), surface_point(widget, x, y))
    else {
        return false;
    };
    toplevel.begin_move(
        &device,
        gdk::BUTTON_PRIMARY as i32,
        sx,
        sy,
        gesture.current_event_time(),
    );
    true
}
